//! LLM provider router — local providers active; cloud dormant until enabled AND network permits.

use anyhow::Result;
use source_core::error::CoreError;
use source_core::types::{
    AuditEvent, AuditEventKind, LlmMessage, LlmProviderId, LlmProviderKind, LlmRequest, LlmResponse,
    LlmRole, ProviderConfig, SessionId,
};
use uuid::Uuid;

use crate::db::Db;
use crate::policy::{PolicyEngine, PolicyHttpClient};

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
    if base.ends_with("/v1") && path.starts_with("chat/") {
        format!("{base}/{path}")
    } else if path.starts_with("v1/") || path.starts_with("api/") {
        format!("{base}/{path}")
    } else if base.contains("/v1") {
        format!("{base}/{path}")
    } else {
        format!("{base}/{path}")
    }
}