use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::policy::{ApprovalCategory, ApprovalDecision, NetworkTier};
use crate::types::{
    CheckpointId, CheckpointMeta, DaemonStatus, LlmRequest, LlmResponse, Mode, ProviderConfig,
    SessionId, SessionMeta, ToolCall, ToolDefinition, ToolResult,
};

pub const PROTOCOL_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(i64),
    String(String),
    Null,
}

impl From<i64> for JsonRpcId {
    fn from(v: i64) -> Self {
        Self::Number(v)
    }
}

impl From<String> for JsonRpcId {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl JsonRpcRequest {
    pub fn new(id: impl Into<JsonRpcId>, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: id.into(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("method not found: {method}"),
            data: None,
        }
    }

    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: msg.into(),
            data: None,
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: msg.into(),
            data: None,
        }
    }

    pub fn application(code: i64, msg: impl Into<String>, data: Option<Value>) -> Self {
        Self {
            code,
            message: msg.into(),
            data,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: JsonRpcId, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(id: JsonRpcId, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Notification {
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

/// Canonical JSON-RPC method names for the Source daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Status,
    SetNetworkTier,
    GetNetworkTier,
    CreateSession,
    ListSessions,
    GetSession,
    SetMode,
    ListProviders,
    UpsertProvider,
    EnableProvider,
    DisableProvider,
    Complete,
    BuildRepoMap,
    CompactTerminal,
    CompactConversation,
    CreateCheckpoint,
    ListCheckpoints,
    Rollback,
    RequestApproval,
    ResolveApproval,
    ListPendingApprovals,
    RegisterTool,
    ListTools,
    InvokeTool,
    EvaluateNetwork,
    ListAuditEvents,
}

impl Method {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Status => "source/status",
            Self::SetNetworkTier => "source/network/setTier",
            Self::GetNetworkTier => "source/network/getTier",
            Self::CreateSession => "source/session/create",
            Self::ListSessions => "source/session/list",
            Self::GetSession => "source/session/get",
            Self::SetMode => "source/mode/set",
            Self::ListProviders => "source/llm/listProviders",
            Self::UpsertProvider => "source/llm/upsertProvider",
            Self::EnableProvider => "source/llm/enableProvider",
            Self::DisableProvider => "source/llm/disableProvider",
            Self::Complete => "source/llm/complete",
            Self::BuildRepoMap => "source/context/buildRepoMap",
            Self::CompactTerminal => "source/context/compactTerminal",
            Self::CompactConversation => "source/context/compactConversation",
            Self::CreateCheckpoint => "source/checkpoint/create",
            Self::ListCheckpoints => "source/checkpoint/list",
            Self::Rollback => "source/checkpoint/rollback",
            Self::RequestApproval => "source/approval/request",
            Self::ResolveApproval => "source/approval/resolve",
            Self::ListPendingApprovals => "source/approval/listPending",
            Self::RegisterTool => "source/tools/register",
            Self::ListTools => "source/tools/list",
            Self::InvokeTool => "source/tools/invoke",
            Self::EvaluateNetwork => "source/network/evaluate",
            Self::ListAuditEvents => "source/audit/list",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "source/status" => Self::Status,
            "source/network/setTier" => Self::SetNetworkTier,
            "source/network/getTier" => Self::GetNetworkTier,
            "source/session/create" => Self::CreateSession,
            "source/session/list" => Self::ListSessions,
            "source/session/get" => Self::GetSession,
            "source/mode/set" => Self::SetMode,
            "source/llm/listProviders" => Self::ListProviders,
            "source/llm/upsertProvider" => Self::UpsertProvider,
            "source/llm/enableProvider" => Self::EnableProvider,
            "source/llm/disableProvider" => Self::DisableProvider,
            "source/llm/complete" => Self::Complete,
            "source/context/buildRepoMap" => Self::BuildRepoMap,
            "source/context/compactTerminal" => Self::CompactTerminal,
            "source/context/compactConversation" => Self::CompactConversation,
            "source/checkpoint/create" => Self::CreateCheckpoint,
            "source/checkpoint/list" => Self::ListCheckpoints,
            "source/checkpoint/rollback" => Self::Rollback,
            "source/approval/request" => Self::RequestApproval,
            "source/approval/resolve" => Self::ResolveApproval,
            "source/approval/listPending" => Self::ListPendingApprovals,
            "source/tools/register" => Self::RegisterTool,
            "source/tools/list" => Self::ListTools,
            "source/tools/invoke" => Self::InvokeTool,
            "source/network/evaluate" => Self::EvaluateNetwork,
            "source/audit/list" => Self::ListAuditEvents,
            _ => return None,
        })
    }
}

// --- Typed params / results used by the daemon and TypeScript client ---

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetNetworkTierParams {
    pub tier: NetworkTier,
    #[serde(default)]
    pub allowlist: Option<Vec<String>>,
    #[serde(default)]
    pub denylist: Option<Vec<String>>,
    #[serde(default)]
    pub require_approval_for_public: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NetworkTierResult {
    pub tier: NetworkTier,
    pub allowlist: Vec<String>,
    pub denylist: Vec<String>,
    pub require_approval_for_public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateSessionParams {
    pub title: String,
    pub workspace_root: String,
    #[serde(default)]
    pub mode: Option<Mode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionIdParams {
    pub session_id: SessionId,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetModeParams {
    pub session_id: SessionId,
    pub mode: Mode,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProviderIdParams {
    pub provider_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BuildRepoMapParams {
    pub root: String,
    #[serde(default)]
    pub max_symbols: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompactTerminalParams {
    pub log: String,
    #[serde(default)]
    pub max_chars: Option<usize>,
    #[serde(default)]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompactConversationParams {
    pub turns: Vec<crate::context::ConversationTurn>,
    pub used_tokens: u32,
    #[serde(default)]
    pub budget: Option<crate::context::ContextBudget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateCheckpointParams {
    pub session_id: SessionId,
    pub workspace_root: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RollbackParams {
    pub session_id: SessionId,
    pub checkpoint_id: CheckpointId,
    pub workspace_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResolveApprovalParams {
    pub approval_id: Uuid,
    pub decision: ApprovalDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RequestApprovalParams {
    pub session_id: Option<SessionId>,
    pub category: ApprovalCategory,
    pub summary: String,
    #[serde(default)]
    pub detail: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvaluateNetworkParams {
    pub host: String,
    #[serde(default)]
    pub ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct InvokeToolParams {
    pub session_id: SessionId,
    pub call: ToolCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListAuditParams {
    #[serde(default)]
    pub session_id: Option<SessionId>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Helper aggregate documenting request/response shapes for codegen/docs.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProtocolCatalog {
    pub version: String,
    pub status: DaemonStatus,
    pub session: SessionMeta,
    pub checkpoint: CheckpointMeta,
    pub provider: ProviderConfig,
    pub llm_request: LlmRequest,
    pub llm_response: LlmResponse,
    pub tool: ToolDefinition,
    pub tool_result: ToolResult,
}