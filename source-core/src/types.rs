use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

use crate::policy::{ApprovalCategory, NetworkTier};

/// Canonical data directory: `~/.source/`
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspacePaths {
    pub root: PathBuf,
    pub db_path: PathBuf,
    pub checkpoints_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub offline_docs_dir: PathBuf,
    pub templates_dir: PathBuf,
    pub extensions_dir: PathBuf,
    pub rag_dir: PathBuf,
    pub vendor_dir: PathBuf,
    pub static_dir: Option<PathBuf>,
}

impl WorkspacePaths {
    pub fn from_home(home: impl Into<PathBuf>) -> Self {
        let root = home.into().join(".source");
        Self {
            db_path: root.join("source.db"),
            checkpoints_dir: root.join("checkpoints"),
            logs_dir: root.join("logs"),
            cache_dir: root.join("cache"),
            offline_docs_dir: root.join("offline-docs"),
            templates_dir: root.join("templates"),
            extensions_dir: root.join("extensions"),
            rag_dir: root.join("cache").join("rag"),
            vendor_dir: root.join("cache").join("vendor"),
            static_dir: None,
            root,
        }
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::create_dir_all(&self.checkpoints_dir)?;
        std::fs::create_dir_all(&self.logs_dir)?;
        std::fs::create_dir_all(&self.cache_dir)?;
        std::fs::create_dir_all(&self.offline_docs_dir)?;
        std::fs::create_dir_all(&self.templates_dir)?;
        std::fs::create_dir_all(&self.extensions_dir)?;
        std::fs::create_dir_all(&self.rag_dir)?;
        std::fs::create_dir_all(&self.vendor_dir)?;
        std::fs::create_dir_all(self.vendor_dir.join("npm"))?;
        std::fs::create_dir_all(self.vendor_dir.join("pnpm"))?;
        std::fs::create_dir_all(self.vendor_dir.join("cargo"))?;
        std::fs::create_dir_all(self.vendor_dir.join("wheels"))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct CheckpointId(pub Uuid);

impl CheckpointId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CheckpointId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for CheckpointId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Plan = read-only; Act = execution with per-category approval gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Plan,
    Act,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Plan => "plan",
            Mode::Act => "act",
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionMeta {
    pub id: SessionId,
    pub title: String,
    pub mode: Mode,
    pub workspace_root: PathBuf,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub network_tier: NetworkTier,
    pub provider_id: Option<LlmProviderId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct LlmProviderId(pub String);

impl LlmProviderId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for LlmProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderKind {
    OpenaiCompatible,
    Ollama,
    LlamaCpp,
    LmStudio,
    Anthropic,
    Openai,
    Gemini,
    OpenRouter,
    Custom,
}

impl LlmProviderKind {
    pub fn is_local(&self) -> bool {
        matches!(
            self,
            LlmProviderKind::OpenaiCompatible
                | LlmProviderKind::Ollama
                | LlmProviderKind::LlamaCpp
                | LlmProviderKind::LmStudio
        )
    }

    pub fn is_cloud(&self) -> bool {
        !self.is_local() || matches!(self, LlmProviderKind::Custom)
    }

    pub fn requires_cloud_egress(&self) -> bool {
        matches!(
            self,
            LlmProviderKind::Anthropic
                | LlmProviderKind::Openai
                | LlmProviderKind::Gemini
                | LlmProviderKind::OpenRouter
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProviderConfig {
    pub id: LlmProviderId,
    pub kind: LlmProviderKind,
    pub display_name: String,
    pub base_url: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub enabled: bool,
    pub extra_headers: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LlmRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LlmRequest {
    pub provider_id: LlmProviderId,
    pub messages: Vec<LlmMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LlmResponse {
    pub provider_id: LlmProviderId,
    pub model: String,
    pub content: String,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub categories: Vec<ApprovalCategory>,
    pub enabled: bool,
    /// Tool can run with no WAN (LOCAL_ONLY / LOCAL_AND_LAN).
    #[serde(default = "default_true")]
    pub offline_capable: bool,
    /// Tool needs public Internet; filtered out unless tier is UNRESTRICTED_CONFIGURABLE.
    #[serde(default)]
    pub requires_wan: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub ok: bool,
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventKind {
    Shell,
    FileDiff,
    NetworkAttempt,
    Delegation,
    ModeChange,
    Checkpoint,
    Rollback,
    Approval,
    ProviderCall,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuditEvent {
    pub id: Uuid,
    pub session_id: Option<SessionId>,
    pub kind: AuditEventKind,
    pub summary: String,
    pub detail: serde_json::Value,
    pub content_hash: String,
    pub prev_hash: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl AuditEvent {
    pub fn hash_payload(kind: AuditEventKind, summary: &str, detail: &serde_json::Value, prev: Option<&str>) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(format!("{:?}", kind).as_bytes());
        hasher.update(summary.as_bytes());
        hasher.update(detail.to_string().as_bytes());
        if let Some(p) = prev {
            hasher.update(p.as_bytes());
        }
        hex::encode(hasher.finalize())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckpointMeta {
    pub id: CheckpointId,
    pub session_id: SessionId,
    pub step_index: u64,
    pub git_commit: String,
    pub label: String,
    pub created_at: DateTime<Utc>,
    pub audit_event_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStatus {
    pub version: String,
    pub protocol_version: String,
    pub mode: Mode,
    pub network_tier: NetworkTier,
    pub data_root: PathBuf,
    pub active_session: Option<SessionId>,
    pub uptime_secs: u64,
    pub providers: Vec<ProviderStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProviderStatus {
    pub id: LlmProviderId,
    pub kind: LlmProviderKind,
    pub display_name: String,
    pub enabled: bool,
    pub reachable: Option<bool>,
    pub base_url: String,
}