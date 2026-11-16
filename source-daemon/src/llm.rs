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
        let url = openai_url(base_url, "models");
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
        let url = openai_url(&provider.base_url, "chat/completions");
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
        let raw = resp
            .text()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let value = parse_json_body(&raw).map_err(|e| {
            CoreError::Other(format!(
                "provider {} returned unparseable JSON: {e}; body={}",
                provider.id,
                truncate_chars(&raw, 400)
            ))
        })?;
        let parsed = parse_openai_chat_response(&value).ok_or_else(|| {
            CoreError::Other(format!(
                "provider {} returned no assistant content (expected choices[0].message.content); body={}",
                provider.id,
                truncate_chars(&raw, 400)
            ))
        })?;
        Ok(LlmResponse {
            provider_id: provider.id.clone(),
            model: value
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or(provider.model.as_str())
                .to_string(),
            content: parsed.content,
            prompt_tokens: value
                .pointer("/usage/prompt_tokens")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            completion_tokens: value
                .pointer("/usage/completion_tokens")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            finish_reason: parsed.finish_reason,
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
        let raw = resp
            .text()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let value = parse_json_body(&raw).map_err(|e| {
            CoreError::Other(format!(
                "ollama returned unparseable JSON: {e}; body={}",
                truncate_chars(&raw, 400)
            ))
        })?;
        // Prefer native Ollama chat/generate shapes; fall back to OpenAI-compat
        // in case the base URL was pointed at /v1 by mistake.
        let parsed = parse_ollama_chat_response(&value)
            .or_else(|| parse_openai_chat_response(&value))
            .ok_or_else(|| {
                CoreError::Other(format!(
                    "ollama returned no assistant content (expected message.content or response); body={}",
                    truncate_chars(&raw, 400)
                ))
            })?;
        Ok(LlmResponse {
            provider_id: provider.id.clone(),
            model: value
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or(provider.model.as_str())
                .to_string(),
            content: parsed.content,
            prompt_tokens: value
                .get("prompt_eval_count")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            completion_tokens: value
                .get("eval_count")
                .and_then(|v| v.as_u64())
                .map(|n| n as u32),
            finish_reason: parsed.finish_reason.or_else(|| {
                value
                    .get("done_reason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            }),
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
            .get("content")
            .and_then(json_to_text)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                CoreError::Other(format!(
                    "anthropic returned no assistant text; body={}",
                    truncate_chars(&value.to_string(), 400)
                ))
            })?;
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
            .pointer("/candidates/0/content/parts")
            .and_then(json_to_text)
            .or_else(|| {
                value
                    .pointer("/candidates/0/content/parts/0/text")
                    .and_then(json_to_text)
            })
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                CoreError::Other(format!(
                    "gemini returned no assistant text; body={}",
                    truncate_chars(&value.to_string(), 400)
                ))
            })?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedAssistant {
    content: String,
    finish_reason: Option<String>,
}

/// Coerce OpenAI/Ollama content fields that may be a string, null, or multipart array.
fn json_to_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Array(items) => {
            let mut out = String::new();
            for item in items {
                if let Some(piece) = json_to_text(item) {
                    out.push_str(&piece);
                } else if let Some(text) = item.get("text").and_then(json_to_text) {
                    out.push_str(&text);
                } else if let Some(text) = item.get("content").and_then(json_to_text) {
                    out.push_str(&text);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out)
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(json_to_text) {
                return Some(text);
            }
            if let Some(text) = map.get("content").and_then(json_to_text) {
                return Some(text);
            }
            None
        }
    }
}

fn parse_openai_chat_response(value: &serde_json::Value) -> Option<ParsedAssistant> {
    // Non-stream chat.completions
    let message = value.pointer("/choices/0/message");
    let mut content = message
        .and_then(|m| m.get("content"))
        .and_then(json_to_text)
        .unwrap_or_default();
    // Some local servers put the final answer only in reasoning/refusal fields.
    if content.trim().is_empty() {
        if let Some(m) = message {
            for key in ["reasoning_content", "reasoning", "refusal"] {
                if let Some(extra) = m.get(key).and_then(json_to_text) {
                    if !extra.trim().is_empty() {
                        content = extra;
                        break;
                    }
                }
            }
        }
    }
    // Accidental stream-shaped single object: choices[0].delta.content
    if content.trim().is_empty() {
        content = value
            .pointer("/choices/0/delta/content")
            .and_then(json_to_text)
            .unwrap_or_default();
    }
    // Legacy text completions
    if content.trim().is_empty() {
        content = value
            .pointer("/choices/0/text")
            .and_then(json_to_text)
            .unwrap_or_default();
    }
    // Aggregated NDJSON stream payload synthesized by parse_json_body
    if content.trim().is_empty() {
        if let Some(chunks) = value.get("source_stream_chunks").and_then(|v| v.as_array()) {
            let mut joined = String::new();
            for chunk in chunks {
                if let Some(piece) = parse_openai_chat_response(chunk).map(|p| p.content) {
                    joined.push_str(&piece);
                }
            }
            content = joined;
        }
    }
    if content.trim().is_empty() {
        return None;
    }
    let finish_reason = value
        .pointer("/choices/0/finish_reason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(ParsedAssistant {
        content,
        finish_reason,
    })
}

fn parse_ollama_chat_response(value: &serde_json::Value) -> Option<ParsedAssistant> {
    let mut content = value
        .pointer("/message/content")
        .and_then(json_to_text)
        .unwrap_or_default();
    // Thinking models may leave content blank while filling message.thinking.
    if content.trim().is_empty() {
        content = value
            .pointer("/message/thinking")
            .and_then(json_to_text)
            .unwrap_or_default();
    }
    // /api/generate uses `response`
    if content.trim().is_empty() {
        content = value
            .get("response")
            .and_then(json_to_text)
            .unwrap_or_default();
    }
    // Aggregated NDJSON stream from Ollama when stream=false was ignored
    if content.trim().is_empty() {
        if let Some(chunks) = value.get("source_stream_chunks").and_then(|v| v.as_array()) {
            let mut joined = String::new();
            for chunk in chunks {
                if let Some(piece) = chunk
                    .pointer("/message/content")
                    .and_then(json_to_text)
                    .or_else(|| chunk.get("response").and_then(json_to_text))
                {
                    joined.push_str(&piece);
                }
            }
            content = joined;
        }
    }
    if content.trim().is_empty() {
        return None;
    }
    let finish_reason = value
        .get("done_reason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(ParsedAssistant {
        content,
        finish_reason,
    })
}

/// Parse a single JSON object, or aggregate NDJSON / SSE `data:` lines into one object.
fn parse_json_body(raw: &str) -> Result<serde_json::Value, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty body".into());
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Ok(value);
    }

    let mut chunks: Vec<serde_json::Value> = Vec::new();
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() || line == "[DONE]" || line == "data: [DONE]" {
            continue;
        }
        let payload = line.strip_prefix("data:").map(str::trim).unwrap_or(line);
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(payload) {
            Ok(v) => chunks.push(v),
            Err(_) => continue,
        }
    }
    if chunks.is_empty() {
        return Err("no JSON object found".into());
    }
    if chunks.len() == 1 {
        return Ok(chunks.remove(0));
    }
    // Prefer the last object that already has full content; otherwise expose chunks.
    if let Some(last_full) = chunks.iter().rev().find(|v| {
        parse_ollama_chat_response(v).is_some() || parse_openai_chat_response(v).is_some()
    }) {
        // If any chunk is a stream delta, concatenate via source_stream_chunks.
        let looks_streamy = chunks.iter().any(|v| {
            v.pointer("/choices/0/delta").is_some()
                || (v.get("done") == Some(&serde_json::Value::Bool(false)))
        });
        if looks_streamy {
            return Ok(serde_json::json!({ "source_stream_chunks": chunks }));
        }
        return Ok(last_full.clone());
    }
    Ok(serde_json::json!({ "source_stream_chunks": chunks }))
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
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
    let path = path.trim_start_matches('/');
    format!("{base}/{path}")
}

/// OpenAI-compatible URL helper. Ensures `/v1/{path}` even when the stored
/// base_url is bare (`http://127.0.0.1:1234`) instead of `.../v1`.
fn openai_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    if base.ends_with("/v1") || base.contains("/v1/") {
        format!("{base}/{path}")
    } else {
        format!("{base}/v1/{path}")
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
            join_url("http://127.0.0.1:11434/", "api/chat"),
            "http://127.0.0.1:11434/api/chat"
        );
        assert_eq!(
            openai_url("http://127.0.0.1:1234/v1", "models"),
            "http://127.0.0.1:1234/v1/models"
        );
        assert_eq!(
            openai_url("http://127.0.0.1:1234/v1", "chat/completions"),
            "http://127.0.0.1:1234/v1/chat/completions"
        );
        // Bare LM Studio / llama.cpp base must still hit /v1/...
        assert_eq!(
            openai_url("http://127.0.0.1:1234", "chat/completions"),
            "http://127.0.0.1:1234/v1/chat/completions"
        );
        assert_eq!(
            openai_url("http://127.0.0.1:1234/", "models"),
            "http://127.0.0.1:1234/v1/models"
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

    #[test]
    fn parse_ollama_chat_fixture_string_content() {
        let fixture = json!({
            "model": "llama3.2",
            "created_at": "2024-01-01T00:00:00Z",
            "message": {
                "role": "assistant",
                "content": "Hello from Ollama"
            },
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 12,
            "eval_count": 4
        });
        let parsed = parse_ollama_chat_response(&fixture).expect("ollama content");
        assert_eq!(parsed.content, "Hello from Ollama");
        assert_eq!(parsed.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn parse_ollama_generate_fixture_response_field() {
        let fixture = json!({
            "model": "llama3.2",
            "response": "generate path answer",
            "done": true
        });
        let parsed = parse_ollama_chat_response(&fixture).expect("generate content");
        assert_eq!(parsed.content, "generate path answer");
    }

    #[test]
    fn parse_ollama_thinking_model_falls_back_to_thinking() {
        let fixture = json!({
            "message": {
                "role": "assistant",
                "content": "",
                "thinking": "I should greet the user."
            },
            "done": true
        });
        let parsed = parse_ollama_chat_response(&fixture).expect("thinking fallback");
        assert_eq!(parsed.content, "I should greet the user.");
    }

    #[test]
    fn parse_ollama_ndjson_stream_chunks() {
        let raw = r#"
{"message":{"role":"assistant","content":"Hel"},"done":false}
{"message":{"role":"assistant","content":"lo"},"done":false}
{"message":{"role":"assistant","content":""},"done":true,"done_reason":"stop"}
"#;
        let value = parse_json_body(raw).expect("ndjson");
        let parsed = parse_ollama_chat_response(&value).expect("aggregated");
        assert_eq!(parsed.content, "Hello");
    }

    #[test]
    fn parse_openai_chat_fixture_string_content() {
        let fixture = json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "model": "local-model",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello from LM Studio"
                },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 8, "completion_tokens": 5 }
        });
        let parsed = parse_openai_chat_response(&fixture).expect("openai content");
        assert_eq!(parsed.content, "Hello from LM Studio");
        assert_eq!(parsed.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn parse_openai_chat_fixture_multipart_content_array() {
        // Root cause of many "(empty)" replies: content is an array of parts,
        // and the old parser only accepted JSON strings via as_str().
        let fixture = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": [
                        { "type": "text", "text": "Part A. " },
                        { "type": "text", "text": "Part B." }
                    ]
                },
                "finish_reason": "stop"
            }]
        });
        let parsed = parse_openai_chat_response(&fixture).expect("array content");
        assert_eq!(parsed.content, "Part A. Part B.");
    }

    #[test]
    fn parse_openai_stream_delta_fixture() {
        let fixture = json!({
            "choices": [{
                "delta": { "content": "stream delta text" },
                "finish_reason": null
            }]
        });
        let parsed = parse_openai_chat_response(&fixture).expect("delta");
        assert_eq!(parsed.content, "stream delta text");
    }

    #[test]
    fn parse_openai_sse_ndjson_aggregates_deltas() {
        let raw = "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"!\"}}]}\n\ndata: [DONE]\n";
        let value = parse_json_body(raw).expect("sse");
        let parsed = parse_openai_chat_response(&value).expect("aggregated deltas");
        assert_eq!(parsed.content, "Hi!");
    }

    #[test]
    fn parse_openai_null_content_is_empty() {
        let fixture = json!({
            "choices": [{
                "message": { "role": "assistant", "content": null },
                "finish_reason": "stop"
            }]
        });
        assert!(parse_openai_chat_response(&fixture).is_none());
    }

    #[test]
    fn json_to_text_handles_nested_shapes() {
        assert_eq!(json_to_text(&json!("plain")).as_deref(), Some("plain"));
        assert_eq!(
            json_to_text(&json!([{ "text": "a" }, { "text": "b" }])).as_deref(),
            Some("ab")
        );
        assert_eq!(json_to_text(&json!({ "text": "obj" })).as_deref(), Some("obj"));
        assert_eq!(json_to_text(&json!(null)), None);
    }
}