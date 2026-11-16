use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::mesh::{
    ClusterSnapshot, DiscoveredPeer, DispatchResult, MeshPeerInfo, MeshStatus, PairingSession,
    PeerRole, PeerSource, RemoteInferRequest, RemoteInferResponse, RemoteTaskInfo, RemoteTaskSpec,
};
use crate::policy::{ApprovalCategory, ApprovalDecision, NetworkTier};
use crate::types::{
    CheckpointId, CheckpointMeta, DaemonStatus, LlmRequest, LlmResponse, Mode, ProviderConfig,
    SessionId, SessionMeta, ToolCall, ToolDefinition, ToolResult,
};

pub const PROTOCOL_VERSION: &str = "1.2.0";

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
    // Phase 3
    SearchSymbols,
    PtyOpen,
    PtyWrite,
    PtyRead,
    PtyResize,
    PtyClose,
    PtyList,
    RagIndex,
    RagQuery,
    RagStatus,
    TemplateList,
    TemplateScaffold,
    CacheVendorStatus,
    McpList,
    McpReload,
    McpCall,
    BrowserNavigate,
    BrowserInspect,
    AppVerify,
    // Phase 4 — mesh / cluster
    MeshStart,
    MeshStop,
    MeshStatus,
    MeshDiscover,
    MeshRegisterPeer,
    MeshUnregisterPeer,
    MeshListPeers,
    MeshBeginPairing,
    MeshConfirmPairing,
    MeshCompletePairing,
    MeshRejectPairing,
    MeshTrustPeer,
    MeshRevokePeer,
    ClusterResources,
    ClusterDispatch,
    ClusterTaskStatus,
    ClusterCancelTask,
    ClusterListTasks,
    LlmCompleteRemote,
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
            Self::SearchSymbols => "source/context/searchSymbols",
            Self::PtyOpen => "source/pty/open",
            Self::PtyWrite => "source/pty/write",
            Self::PtyRead => "source/pty/read",
            Self::PtyResize => "source/pty/resize",
            Self::PtyClose => "source/pty/close",
            Self::PtyList => "source/pty/list",
            Self::RagIndex => "source/rag/index",
            Self::RagQuery => "source/rag/query",
            Self::RagStatus => "source/rag/status",
            Self::TemplateList => "source/templates/list",
            Self::TemplateScaffold => "source/templates/scaffold",
            Self::CacheVendorStatus => "source/cache/vendorStatus",
            Self::McpList => "source/mcp/list",
            Self::McpReload => "source/mcp/reload",
            Self::McpCall => "source/mcp/call",
            Self::BrowserNavigate => "source/browser/navigate",
            Self::BrowserInspect => "source/browser/inspect",
            Self::AppVerify => "source/browser/verifyApp",
            Self::MeshStart => "source/mesh/start",
            Self::MeshStop => "source/mesh/stop",
            Self::MeshStatus => "source/mesh/status",
            Self::MeshDiscover => "source/mesh/discover",
            Self::MeshRegisterPeer => "source/mesh/registerPeer",
            Self::MeshUnregisterPeer => "source/mesh/unregisterPeer",
            Self::MeshListPeers => "source/mesh/listPeers",
            Self::MeshBeginPairing => "source/mesh/beginPairing",
            Self::MeshConfirmPairing => "source/mesh/confirmPairing",
            Self::MeshCompletePairing => "source/mesh/completePairing",
            Self::MeshRejectPairing => "source/mesh/rejectPairing",
            Self::MeshTrustPeer => "source/mesh/trustPeer",
            Self::MeshRevokePeer => "source/mesh/revokePeer",
            Self::ClusterResources => "source/cluster/resources",
            Self::ClusterDispatch => "source/cluster/dispatch",
            Self::ClusterTaskStatus => "source/cluster/taskStatus",
            Self::ClusterCancelTask => "source/cluster/cancelTask",
            Self::ClusterListTasks => "source/cluster/listTasks",
            Self::LlmCompleteRemote => "source/llm/completeRemote",
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
            "source/context/searchSymbols" => Self::SearchSymbols,
            "source/pty/open" => Self::PtyOpen,
            "source/pty/write" => Self::PtyWrite,
            "source/pty/read" => Self::PtyRead,
            "source/pty/resize" => Self::PtyResize,
            "source/pty/close" => Self::PtyClose,
            "source/pty/list" => Self::PtyList,
            "source/rag/index" => Self::RagIndex,
            "source/rag/query" => Self::RagQuery,
            "source/rag/status" => Self::RagStatus,
            "source/templates/list" => Self::TemplateList,
            "source/templates/scaffold" => Self::TemplateScaffold,
            "source/cache/vendorStatus" => Self::CacheVendorStatus,
            "source/mcp/list" => Self::McpList,
            "source/mcp/reload" => Self::McpReload,
            "source/mcp/call" => Self::McpCall,
            "source/browser/navigate" => Self::BrowserNavigate,
            "source/browser/inspect" => Self::BrowserInspect,
            "source/browser/verifyApp" => Self::AppVerify,
            "source/mesh/start" => Self::MeshStart,
            "source/mesh/stop" => Self::MeshStop,
            "source/mesh/status" => Self::MeshStatus,
            "source/mesh/discover" => Self::MeshDiscover,
            "source/mesh/registerPeer" => Self::MeshRegisterPeer,
            "source/mesh/unregisterPeer" => Self::MeshUnregisterPeer,
            "source/mesh/listPeers" => Self::MeshListPeers,
            "source/mesh/beginPairing" => Self::MeshBeginPairing,
            "source/mesh/confirmPairing" => Self::MeshConfirmPairing,
            "source/mesh/completePairing" => Self::MeshCompletePairing,
            "source/mesh/rejectPairing" => Self::MeshRejectPairing,
            "source/mesh/trustPeer" => Self::MeshTrustPeer,
            "source/mesh/revokePeer" => Self::MeshRevokePeer,
            "source/cluster/resources" => Self::ClusterResources,
            "source/cluster/dispatch" => Self::ClusterDispatch,
            "source/cluster/taskStatus" => Self::ClusterTaskStatus,
            "source/cluster/cancelTask" => Self::ClusterCancelTask,
            "source/cluster/listTasks" => Self::ClusterListTasks,
            "source/llm/completeRemote" => Self::LlmCompleteRemote,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SearchSymbolsParams {
    pub root: String,
    pub query: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub path_glob: Option<String>,
    #[serde(default)]
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PtyOpenParams {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
    #[serde(default)]
    pub env: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PtyIdParams {
    pub pty_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PtyWriteParams {
    pub pty_id: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PtyReadParams {
    pub pty_id: String,
    #[serde(default)]
    pub max_bytes: Option<usize>,
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PtyResizeParams {
    pub pty_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RagIndexParams {
    #[serde(default)]
    pub docs_root: Option<String>,
    #[serde(default)]
    pub force: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RagQueryParams {
    pub query: String,
    #[serde(default)]
    pub top_k: Option<usize>,
    #[serde(default)]
    pub bm25_weight: Option<f32>,
    #[serde(default)]
    pub vector_weight: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TemplateScaffoldParams {
    pub template_id: String,
    pub dest: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpCallParams {
    pub server_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BrowserNavigateParams {
    pub url: String,
    #[serde(default)]
    pub wait_ms: Option<u64>,
    #[serde(default)]
    pub session_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BrowserInspectParams {
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub script: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AppVerifyParams {
    pub launch_command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    pub url: String,
    #[serde(default)]
    pub ready_selector: Option<String>,
    #[serde(default)]
    pub assert_text: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

// --- Phase 4 mesh / cluster params ---

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MeshStartParams {
    #[serde(default)]
    pub advertise: Option<bool>,
    #[serde(default)]
    pub browse: Option<bool>,
    #[serde(default)]
    pub role: Option<PeerRole>,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RegisterPeerParams {
    /// Hostname, IPv4/IPv6, or Tailscale MagicDNS name.
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub source: Option<PeerSource>,
    #[serde(default)]
    pub peer_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PeerIdParams {
    pub peer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BeginPairingParams {
    #[serde(default)]
    pub label: Option<String>,
    /// PIN lifetime in seconds (default 300).
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfirmPairingParams {
    pub pairing_id: Uuid,
    pub peer_id: String,
    pub peer_public_key: String,
    pub pin: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompletePairingParams {
    /// Remote peer host (IP / MagicDNS).
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub pin: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RejectPairingParams {
    pub pairing_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TrustPeerParams {
    pub peer_id: String,
    pub trusted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClusterDispatchParams {
    pub tasks: Vec<RemoteTaskSpec>,
    #[serde(default)]
    pub parallel: Option<bool>,
    #[serde(default)]
    pub session_id: Option<SessionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskIdParams {
    pub task_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListTasksParams {
    #[serde(default)]
    pub include_completed: Option<bool>,
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
    pub mesh_status: MeshStatus,
    pub mesh_peer: MeshPeerInfo,
    pub discovered_peer: DiscoveredPeer,
    pub pairing: PairingSession,
    pub cluster: ClusterSnapshot,
    pub dispatch: DispatchResult,
    pub remote_task: RemoteTaskInfo,
    pub remote_infer_request: RemoteInferRequest,
    pub remote_infer_response: RemoteInferResponse,
}