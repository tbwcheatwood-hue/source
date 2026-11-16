//! LLM provider router — local providers active; cloud dormant until enabled AND network permits.

use anyhow::Result;
use source_core::error::CoreError;
use source_core::protocol::{
    ListModelsResult, ProbeProviderParams, ProviderProbeResult, SelectedProviderResult,
};
use source_core::types::{
    AuditEvent, AuditEventKind, LlmMessage, LlmProviderId, LlmProviderKind, LlmRequest, LlmResponse,
    LlmRole, ProviderConfig, SessionId,
};
use std::time::Instant;
use uuid::Uuid;

use crate::db::Db;
use crate::policy::{PolicyEngine, PolicyHttpClient};

pub const SELECTED_PROVIDER_KV: &str = "selected_provider_id";

pub struct LlmRouter {
    db: Db,
    policy: PolicyEngine,
    http: PolicyHttpClient,
}

impl LlmRouter {
    pub fn new(db: Db, policy: PolicyEngine, http: PolicyHttpClient) -> Self {
        Self { db, policy, http }
    }

    pub async fn list(&self) -> Result<Vec<ProviderConfig>> {
        self.db.list_providers().await
    }

    pub async fn upsert(&self, config: ProviderConfig) -> Result<ProviderConfig> {
        self.db.upsert_provider(&config).await?;
        Ok(config)
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        if enabled {
            if let Some(p) = self.db.get_provider(id).await? {
                if p.kind.requires_cloud_egress() {
                    // Enabling cloud does not bypass network tier — complete() will enforce.
                    tracing::info!(
                        provider = id,
                        tier = %self.policy.tier(),
                        "cloud provider enabled; egress still gated by network policy"
                    );
                }
            }
        }
        self.db.set_provider_enabled(id, enabled).await
    }

    /// Resolve which provider Chat / Complete should use when the request omits an id.
    pub async fn resolve_provider_id(
        &self,
        requested: &str,
        session_id: Option<&SessionId>,
    ) -> Result<String, CoreError> {
        let trimmed = requested.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
        if let Some(sid) = session_id {
            if let Ok(Some(sess)) = self.db.get_session(sid).await {
                if let Some(pid) = sess.provider_id {
                    if !pid.0.is_empty() {
                        return Ok(pid.0);
                    }
                }
            }
        }
        if let Some(selected) = self
            .get_selected_provider()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?
            .provider_id
        {
            return Ok(selected);
        }
        // Prefer first enabled local provider; fall back to seeded ollama id.
        let providers = self
            .list()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        if let Some(p) = providers
            .iter()
            .find(|p| p.enabled && p.kind.is_local())
            .or_else(|| providers.iter().find(|p| p.enabled))
        {
            return Ok(p.id.0.clone());
        }
        Ok("ollama-local".into())
    }

    pub async fn get_selected_provider(&self) -> Result<SelectedProviderResult> {
        let provider_id = match self.db.get_kv(SELECTED_PROVIDER_KV).await? {
            Some(v) => v
                .as_str()
                .map(|s| s.to_string())
                .or_else(|| v.get("provider_id").and_then(|x| x.as_str()).map(|s| s.to_string())),
            None => None,
        };
        let provider = match &provider_id {
            Some(id) => self.db.get_provider(id).await?,
            None => None,
        };
        Ok(SelectedProviderResult {
            provider_id,
            provider,
        })
    }

    pub async fn set_selected_provider(&self, provider_id: &str) -> Result<SelectedProviderResult> {
        let provider = self
            .db
            .get_provider(provider_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("provider `{provider_id}` not found"))?;
        self.db
            .set_kv(SELECTED_PROVIDER_KV, &serde_json::json!(provider_id))
            .await?;
        Ok(SelectedProviderResult {
            provider_id: Some(provider_id.to_string()),
            provider: Some(provider),
        })
    }

    pub async fn list_models(&self, provider_id: &str) -> Result<ListModelsResult, CoreError> {
        let provider = self
            .db
            .get_provider(provider_id)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?
            .ok_or_else(|| CoreError::ProviderUnavailable(provider_id.to_string()))?;
        let models = self
            .fetch_models(&provider.kind, &provider.base_url, provider.api_key_env.as_deref(), None)
            .await?;
        Ok(ListModelsResult {
            provider_id: provider.id.0.clone(),
            base_url: provider.base_url,
            kind: provider.kind,
            models,
        })
    }

    pub async fn probe(
        &self,
        params: ProbeProviderParams,
        session_id: Option<SessionId>,
    ) -> Result<ProviderProbeResult, CoreError> {
        let (provider_id, kind, base_url, api_key_env) =
            self.resolve_probe_target(&params).await?;
        let tier = self.policy.tier();
        let started = Instant::now();

        // Pre-check policy so the UI can show LOCAL_ONLY impact even when the probe fails.
        let network_allowed = match self
            .http
            .policy
            .assert_url_allowed(&base_url, session_id.clone(), "llm-probe")
            .await
        {
            Ok(()) => true,
            Err(CoreError::NetworkBlocked(_)) | Err(CoreError::ApprovalRequired(_)) => false,
            Err(e) => {
                return Ok(ProviderProbeResult {
                    ok: false,
                    latency_ms: started.elapsed().as_millis() as u64,
                    provider_id,
                    base_url,
                    kind,
                    models: vec![],
                    error: Some(e.to_string()),
                    tier,
                    network_allowed: false,
                });
            }
        };

        if !network_allowed {
            return Ok(ProviderProbeResult {
                ok: false,
                latency_ms: started.elapsed().as_millis() as u64,
                provider_id,
                base_url: base_url.clone(),
                kind,
                models: vec![],
                error: Some(format!(
                    "network tier {tier} blocks `{base_url}` (LOCAL_ONLY allows localhost only)"
                )),
                tier,
                network_allowed: false,
            });
        }

        match self
            .fetch_models(&kind, &base_url, api_key_env.as_deref(), session_id)
            .await
        {
            Ok(models) => Ok(ProviderProbeResult {
                ok: true,
                latency_ms: started.elapsed().as_millis() as u64,
                provider_id,
                base_url,
                kind,
                models,
                error: None,
                tier,
                network_allowed: true,
            }),
            Err(e) => Ok(ProviderProbeResult {
                ok: false,
                latency_ms: started.elapsed().as_millis() as u64,
                provider_id,
                base_url,
                kind,
                models: vec![],
                error: Some(e.to_string()),
                tier,
                network_allowed: true,
            }),
        }
    }

    async fn resolve_probe_target(
        &self,
        params: &ProbeProviderParams,
    ) -> Result<(Option<String>, LlmProviderKind, String, Option<String>), CoreError> {
        if let Some(id) = params.provider_id.as_deref().filter(|s| !s.is_empty()) {
            let provider = self
                .db
                .get_provider(id)
                .await
                .map_err(|e| CoreError::Other(e.to_string()))?
                .ok_or_else(|| CoreError::ProviderUnavailable(id.to_string()))?;
            let kind = params.kind.unwrap_or(provider.kind);
            let base_url = params
                .base_url
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(provider.base_url);
            let api_key_env = params
                .api_key_env
                .clone()
                .or(provider.api_key_env);
            return Ok((Some(provider.id.0), kind, base_url, api_key_env));
        }
        let kind = params.kind.unwrap_or(LlmProviderKind::Ollama);
        let base_url = params.base_url.clone().unwrap_or_else(|| match kind {
            LlmProviderKind::Ollama => "http://127.0.0.1:11434".into(),
            LlmProviderKind::LmStudio => "http://127.0.0.1:1234/v1".into(),
            _ => "http://127.0.0.1:11434".into(),
        });
        Ok((None, kind, base_url, params.api_key_env.clone()))
    }

    async fn fetch_models(
        &self,
        kind: &LlmProviderKind,
        base_url: &str,
        api_key_env: Option<&str>,
        session_id: Option<SessionId>,
    ) -> Result<Vec<String>, CoreError> {
        match kind {
            LlmProviderKind::Ollama => {
                self.fetch_ollama_tags(base_url, session_id).await
            }
            LlmProviderKind::LmStudio
            | LlmProviderKind::LlamaCpp
            | LlmProviderKind::OpenaiCompatible
            | LlmProviderKind::Openai
            | LlmProviderKind::OpenRouter
            | LlmProviderKind::Custom => {
                self.fetch_openai_models(base_url, api_key_env, session_id)
                    .await
            }
            other => Err(CoreError::Other(format!(
                "model listing is not supported for provider kind {other:?}"
            ))),
        }
    }

    async fn fetch_ollama_tags(
        &self,
        base_url: &str,
        session_id: Option<SessionId>,
    ) -> Result<Vec<String>, CoreError> {
        let url = join_url(base_url, "api/tags");
        let resp = self
            .http
            .request_json(
                reqwest::Method::GET,
                &url,
                &[],
                None,
                session_id,
                "llm-ollama-tags",
            )
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Other(format!(
                "ollama /api/tags returned {status}: {text}"
            )));
        }
        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        Ok(parse_ollama_tags(&value))
    }

    async fn fetch_openai_models(
        &self,
        base_url: &str,
        api_key_env: Option<&str>,
        session_id: Option<SessionId>,
    ) -> Result<Vec<String>, CoreError> {
        let url = join_url(base_url, "models");
        let mut headers: Vec<(String, String)> = Vec::new();
        if let Some(env_name) = api_key_env {
            if let Ok(key) = std::env::var(env_name) {
                headers.push(("Authorization".into(), format!("Bearer {key}")));
            }
        }
        let resp = self
            .http
            .request_json(
                reqwest::Method::GET,
                &url,
                &headers,
                None,
                session_id,
                "llm-openai-models",
            )
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Other(format!(
                "models endpoint returned {status}: {text}"
            )));
        }
        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        Ok(parse_openai_models(&value))
    }

    pub async fn complete(
        &self,
        request: LlmRequest,
        session_id: Option<SessionId>,
    ) -> Result<LlmResponse, CoreError> {
        let provider = self
            .db
            .get_provider(&request.provider_id.0)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?
            .ok_or_else(|| CoreError::ProviderUnavailable(request.provider_id.0.clone()))?;

        if !provider.enabled {
            return Err(CoreError::ProviderUnavailable(format!(
                "{} is disabled",
                provider.id
            )));
        }

        // Cloud connectors remain dormant until enabled AND network tier permits.
        if provider.kind.requires_cloud_egress() {
            self.http
                .policy
                .assert_url_allowed(&provider.base_url, session_id, "llm-cloud")
                .await?;
        } else {
            // Local providers still go through interceptor (loopback should pass LOCAL_ONLY).
            self.http
                .policy
                .assert_url_allowed(&provider.base_url, session_id, "llm-local")
                .await?;
        }

        let response = match provider.kind {
            LlmProviderKind::Ollama => self.complete_ollama(&provider, &request, session_id).await?,
            LlmProviderKind::LlamaCpp
            | LlmProviderKind::LmStudio
            | LlmProviderKind::OpenaiCompatible
            | LlmProviderKind::Openai
            | LlmProviderKind::OpenRouter
            | LlmProviderKind::Custom => {
                self.complete_openai_compatible(&provider, &request, session_id)
                    .await?
            }
            LlmProviderKind::Anthropic => {
                self.complete_anthropic(&provider, &request, session_id).await?
            }
            LlmProviderKind::Gemini => self.complete_gemini(&provider, &request, session_id).await?,
        };

        let _ = self
            .db
            .append_audit(AuditEvent {
                id: Uuid::new_v4(),
                session_id,
                kind: AuditEventKind::ProviderCall,
                summary: format!("llm complete via {}", provider.id),
                detail: serde_json::json!({
                    "provider": provider.id.0,
                    "model": response.model,
                    "prompt_tokens": response.prompt_tokens,
                    "completion_tokens": response.completion_tokens,
                }),
                content_hash: String::new(),
                prev_hash: None,
                created_at: chrono::Utc::now(),
            })
            .await;

        Ok(response)
    }

    async fn complete_openai_compatible(
        &self,
        provider: &ProviderConfig,
        request: &LlmRequest,
        session_id: Option<SessionId>,
    ) -> Result<LlmResponse, CoreError> {
        let url = join_url(&provider.base_url, "chat/completions");
        let mut headers = provider.extra_headers.clone();
        if let Some(env_name) = &provider.api_key_env {
            if let Ok(key) = std::env::var(env_name) {
                headers.insert("Authorization".into(), format!("Bearer {key}"));
            }
        }
        let body = serde_json::json!({
            "model": provider.model,
            "messages": messages_openai(&request.messages),
            "temperature": request.temperature.unwrap_or(0.2),
            "max_tokens": request.max_tokens,
            "stream": false,
        });
        let header_pairs: Vec<(String, String)> = headers.into_iter().collect();
        let resp = self
            .http
            .request_json(
                reqwest::Method::POST,
                &url,
                &header_pairs,
                Some(&body),
                session_id,
                "llm-openai-compatible",
            )
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Other(format!(
                "provider {} returned {status}: {text}",
                provider.id
            )));
        }
        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let content = value
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(LlmResponse {
            provider_id: provider.id.clone(),
            model: provider.model.clone(),
            content,
            prompt_tokens: value
                .pointer("/usage/prompt_tokens")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            completion_tokens: value
                .pointer("/usage/completion_tokens")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            finish_reason: value
                .pointer("/choices/0/finish_reason")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    }

    async fn complete_ollama(
        &self,
        provider: &ProviderConfig,
        request: &LlmRequest,
        session_id: Option<SessionId>,
    ) -> Result<LlmResponse, CoreError> {
        let url = join_url(&provider.base_url, "api/chat");
        let body = serde_json::json!({
            "model": provider.model,
            "messages": messages_openai(&request.messages),
            "stream": false,
            "options": {
                "temperature": request.temperature.unwrap_or(0.2),
                "num_predict": request.max_tokens,
            }
        });
        let resp = self
            .http
            .request_json(
                reqwest::Method::POST,
                &url,
                &[],
                Some(&body),
                session_id,
                "llm-ollama",
            )
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Other(format!(
                "ollama returned {status}: {text}"
            )));
        }
        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let content = value
            .pointer("/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(LlmResponse {
            provider_id: provider.id.clone(),
            model: provider.model.clone(),
            content,
            prompt_tokens: value
                .get("prompt_eval_count")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            completion_tokens: value
                .get("eval_count")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            finish_reason: value
                .get("done_reason")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    }

    async fn complete_anthropic(
        &self,
        provider: &ProviderConfig,
        request: &LlmRequest,
        session_id: Option<SessionId>,
    ) -> Result<LlmResponse, CoreError> {
        let url = join_url(&provider.base_url, "v1/messages");
        let mut headers = vec![
            ("anthropic-version".into(), "2023-06-01".into()),
            ("content-type".into(), "application/json".into()),
        ];
        if let Some(env_name) = &provider.api_key_env {
            if let Ok(key) = std::env::var(env_name) {
                headers.push(("x-api-key".into(), key));
            }
        }
        let system = request
            .messages
            .iter()
            .find(|m| matches!(m.role, LlmRole::System))
            .map(|m| m.content.clone());
        let messages: Vec<serde_json::Value> = request
            .messages
            .iter()
            .filter(|m| !matches!(m.role, LlmRole::System))
            .map(|m| {
                serde_json::json!({
                    "role": match m.role {
                        LlmRole::Assistant => "assistant",
                        _ => "user",
                    },
                    "content": m.content,
                })
            })
            .collect();
        let mut body = serde_json::json!({
            "model": provider.model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(1024),
        });
        if let Some(sys) = system {
            body["system"] = serde_json::Value::String(sys);
        }
        let resp = self
            .http
            .request_json(
                reqwest::Method::POST,
                &url,
                &headers,
                Some(&body),
                session_id,
                "llm-anthropic",
            )
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Other(format!(
                "anthropic returned {status}: {text}"
            )));
        }
        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let content = value
            .pointer("/content/0/text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(LlmResponse {
            provider_id: provider.id.clone(),
            model: provider.model.clone(),
            content,
            prompt_tokens: value
                .pointer("/usage/input_tokens")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            completion_tokens: value
                .pointer("/usage/output_tokens")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            finish_reason: value
                .get("stop_reason")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    }

    async fn complete_gemini(
        &self,
        provider: &ProviderConfig,
        request: &LlmRequest,
        session_id: Option<SessionId>,
    ) -> Result<LlmResponse, CoreError> {
        let key = provider
            .api_key_env
            .as_ref()
            .and_then(|e| std::env::var(e).ok())
            .unwrap_or_default();
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            provider.base_url.trim_end_matches('/'),
            provider.model,
            key
        );
        let contents: Vec<serde_json::Value> = request
            .messages
            .iter()
            .filter(|m| !matches!(m.role, LlmRole::System))
            .map(|m| {
                serde_json::json!({
                    "role": match m.role {
                        LlmRole::Assistant => "model",
                        _ => "user",
                    },
                    "parts": [{ "text": m.content }],
                })
            })
            .collect();
        let body = serde_json::json!({ "contents": contents });
        let resp = self
            .http
            .request_json(
                reqwest::Method::POST,
                &url,
                &[],
                Some(&body),
                session_id,
                "llm-gemini",
            )
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Other(format!(
                "gemini returned {status}: {text}"
            )));
        }
        let value: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let content = value
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(LlmResponse {
            provider_id: LlmProviderId::new(provider.id.0.clone()),
            model: provider.model.clone(),
            content,
            prompt_tokens: None,
            completion_tokens: None,
            finish_reason: None,
        })
    }
}

fn messages_openai(messages: &[LlmMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": match m.role {
                    LlmRole::System => "system",
                    LlmRole::User => "user",
                    LlmRole::Assistant => "assistant",
                    LlmRole::Tool => "tool",
                },
                "content": m.content,
            })
        })
        .collect()
}

fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1") && (path.starts_with("chat/") || path == "models") {
        format!("{base}/{path}")
    } else if path.starts_with("v1/") || path.starts_with("api/") {
        format!("{base}/{path}")
    } else if base.contains("/v1") {
        format!("{base}/{path}")
    } else {
        format!("{base}/{path}")
    }
}

fn parse_ollama_tags(value: &serde_json::Value) -> Vec<String> {
    value
        .get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    m.get("name")
                        .or_else(|| m.get("model"))
                        .and_then(|n| n.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_openai_models(value: &serde_json::Value) -> Vec<String> {
    value
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|n| n.as_str()).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn join_url_ollama_and_lmstudio() {
        assert_eq!(
            join_url("http://127.0.0.1:11434", "api/tags"),
            "http://127.0.0.1:11434/api/tags"
        );
        assert_eq!(
            join_url("http://127.0.0.1:1234/v1", "models"),
            "http://127.0.0.1:1234/v1/models"
        );
        assert_eq!(
            join_url("http://127.0.0.1:1234/v1", "chat/completions"),
            "http://127.0.0.1:1234/v1/chat/completions"
        );
    }

    #[test]
    fn parse_model_catalogs() {
        let ollama = json!({
            "models": [
                { "name": "llama3.2:latest" },
                { "name": "codellama:7b" }
            ]
        });
        assert_eq!(
            parse_ollama_tags(&ollama),
            vec!["llama3.2:latest", "codellama:7b"]
        );
        let openai = json!({
            "data": [
                { "id": "local-model" },
                { "id": "qwen2.5-coder" }
            ]
        });
        assert_eq!(
            parse_openai_models(&openai),
            vec!["local-model", "qwen2.5-coder"]
        );
    }
}