//! Source shared library: protocol types, network policy, context models, and errors.

pub mod context;
pub mod error;
pub mod mesh;
pub mod policy;
pub mod protocol;
pub mod types;

pub use context::{
    CompactionResult, ContextBudget, ConversationTurn, RepoMap, RepoSymbol, TerminalLogSlice,
};
pub use error::{CoreError, CoreResult};
pub use mesh::{
    mesh_tier_allowed, ClusterPeerSnapshot, ClusterSnapshot, DiscoveredPeer, DispatchResult,
    GpuDevice, LoadedModel, MeshPeerInfo, MeshStatus, NodeResources, PairingSession,
    PeerCapabilities, PeerConnectionState, PeerRole, PeerSource, RemoteInferRequest,
    RemoteInferResponse, RemoteTaskInfo, RemoteTaskKind, RemoteTaskSpec, RemoteTaskState,
    MESH_SERVICE_TYPE, MESH_SERVICE_TYPE_BARE,
};
pub use policy::{
    ApprovalCategory, ApprovalDecision, ApprovalRequest, DestinationClass, NetworkDecision,
    NetworkPolicy, NetworkTier, PeerKeyRecord, ToolPermission,
};
pub use protocol::{
    JsonRpcError, JsonRpcId, JsonRpcRequest, JsonRpcResponse, Method, Notification, PROTOCOL_VERSION,
};
pub use types::{
    AuditEvent, AuditEventKind, CheckpointId, CheckpointMeta, DaemonStatus, LlmMessage,
    LlmProviderId, LlmProviderKind, LlmRequest, LlmResponse, Mode, ProviderConfig, SessionId,
    SessionMeta, ToolCall, ToolDefinition, ToolResult, WorkspacePaths,
};