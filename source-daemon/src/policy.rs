//! Network policy enforcement for HTTP/WS clients and the tool-runner gate.

use anyhow::Result;
use parking_lot::RwLock;
use source_core::error::CoreError;
use source_core::policy::{NetworkDecision, NetworkPolicy, NetworkTier};
use source_core::types::{AuditEvent, AuditEventKind, SessionId};
use std::net::IpAddr;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::Db;

#[derive(Clone)]
pub struct PolicyEngine {
    db: Db,
    policy: Arc<RwLock<NetworkPolicy>>,
}

impl PolicyEngine {
    pub async fn load(db: Db) -> Result<Self> {
        let policy = db.load_network_policy().await?;
        Ok(Self {
            db,
            policy: Arc::new(RwLock::new(policy)),
        })
    }

    pub fn current(&self) -> NetworkPolicy {
        self.policy.read().clone()
    }

    pub fn tier(&self) -> NetworkTier {
        self.policy.read().tier
    }

    pub async fn set_policy(&self, policy: NetworkPolicy) -> Result<NetworkPolicy> {
        self.db.save_network_policy(&policy).await?;
        *self.policy.write() = policy.clone();
        let _ = self
            .db
            .append_audit(AuditEvent {
                id: Uuid::new_v4(),
                session_id: None,
                kind: AuditEventKind::NetworkAttempt,
                summary: format!("network tier set to {}", policy.tier),
                detail: serde_json::json!({
                    "tier": policy.tier.as_str(),
                    "allowlist": policy.allowlist,
                    "denylist": policy.denylist,
                }),
                content_hash: String::new(),
                prev_hash: None,
                created_at: chrono::Utc::now(),
            })
            .await?;
        Ok(policy)
    }

    pub fn evaluate(&self, host: &str, ip: Option<IpAddr>) -> NetworkDecision {
        self.policy.read().evaluate(host, ip)
    }

    pub fn evaluate_url(&self, url: &str) -> Result<NetworkDecision, CoreError> {
        self.policy.read().evaluate_url(url)
    }

    /// Interceptor used by HTTP/WS client + tool-runner. Audits every attempt.
    pub async fn intercept(
        &self,
        host: &str,
        ip: Option<IpAddr>,
        session_id: Option<SessionId>,
        purpose: &str,
    ) -> Result<NetworkDecision, CoreError> {
        let decision = self.evaluate(host, ip);
        let allowed = matches!(decision, NetworkDecision::Allow { .. });
        let _ = self
            .db
            .append_audit(AuditEvent {
                id: Uuid::new_v4(),
                session_id,
                kind: AuditEventKind::NetworkAttempt,
                summary: format!(
                    "{} {} -> {}",
                    purpose,
                    host,
                    if allowed { "allow" } else { "block/approve" }
                ),
                detail: serde_json::json!({
                    "host": host,
                    "ip": ip.map(|i| i.to_string()),
                    "purpose": purpose,
                    "decision": decision,
                    "tier": self.tier().as_str(),
                }),
                content_hash: String::new(),
                prev_hash: None,
                created_at: chrono::Utc::now(),
            })
            .await;

        match &decision {
            NetworkDecision::Allow { .. } => Ok(decision),
            NetworkDecision::Deny { reason, .. } => Err(CoreError::NetworkBlocked(reason.clone())),
            NetworkDecision::NeedsApproval { host, .. } => {
                Err(CoreError::ApprovalRequired(format!("network:{host}")))
            }
        }
    }

    pub async fn assert_url_allowed(
        &self,
        url: &str,
        session_id: Option<SessionId>,
        purpose: &str,
    ) -> Result<(), CoreError> {
        let parsed = url::Url::parse(url)
            .map_err(|e| CoreError::InvalidArgument(format!("invalid url: {e}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| CoreError::InvalidArgument("url missing host".into()))?;
        let ip = host.parse::<IpAddr>().ok();
        self.intercept(host, ip, session_id, purpose).await?;
        Ok(())
    }
}

/// Policy-aware reqwest wrapper that refuses blocked destinations before connect.
#[derive(Clone)]
pub struct PolicyHttpClient {
    inner: reqwest::Client,
    pub policy: PolicyEngine,
}

impl PolicyHttpClient {
    pub fn new(policy: PolicyEngine) -> Result<Self> {
        let inner = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(Self { inner, policy })
    }

    pub async fn request_json(
        &self,
        method: reqwest::Method,
        url: &str,
        headers: &[(String, String)],
        body: Option<&serde_json::Value>,
        session_id: Option<SessionId>,
        purpose: &str,
    ) -> Result<reqwest::Response, CoreError> {
        self.policy
            .assert_url_allowed(url, session_id, purpose)
            .await?;
        let mut builder = self.inner.request(method, url);
        for (k, v) in headers {
            builder = builder.header(k, v);
        }
        if let Some(b) = body {
            builder = builder.json(b);
        }
        builder
            .send()
            .await
            .map_err(|e| CoreError::Other(format!("http error: {e}")))
    }

    pub fn raw(&self) -> &reqwest::Client {
        &self.inner
    }
}