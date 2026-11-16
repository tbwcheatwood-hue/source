//! Mesh / cluster types for LAN + Tailscale peer discovery, pairing, and distributed compute.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::policy::NetworkTier;
use crate::types::{LlmRequest, LlmResponse};

pub const MESH_SERVICE_TYPE: &str = "_source-mesh._tcp.local.";
pub const MESH_SERVICE_TYPE_BARE: &str = "_source-mesh._tcp.local";
pub const DEFAULT_MESH_PORT: u16 = 7420;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PeerRole {
    Orchestrator,
    Worker,
    Hybrid,
}

impl PeerRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Orchestrator => "orchestrator",
            Self::Worker => "worker",
            Self::Hybrid => "hybrid",
        }
    }
}

impl std::str::FromStr for PeerRole {
    type Err = crate::error::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "orchestrator" | "planner" => Ok(Self::Orchestrator),
            "worker" => Ok(Self::Worker),
            "hybrid" | "" => Ok(Self::Hybrid),
            other => Err(crate::error::CoreError::InvalidArgument(format!(
                "unknown peer role: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PeerConnectionState {
    Discovered,
    Pairing,
    Paired,
    Connected,
    Offline,
    Untrusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PeerSource {
    Mdns,
    Manual,
    Tailscale,
}

impl PeerSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mdns => "mdns",
            Self::Manual => "manual",
            Self::Tailscale => "tailscale",
        }
    }
}

impl std::str::FromStr for PeerSource {
    type Err = crate::error::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mdns" | "zeroconf" => Ok(Self::Mdns),
            "manual" | "ip" => Ok(Self::Manual),
            "tailscale" | "magicdns" => Ok(Self::Tailscale),
            other => Err(crate::error::CoreError::InvalidArgument(format!(
                "unknown peer source: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GpuDevice {
    pub index: u32,
    pub name: String,
    pub vram_total_mb: u64,
    pub vram_free_mb: u64,
    pub utilization_pct: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LoadedModel {
    pub name: String,
    pub provider_id: Option<String>,
    pub vram_mb: Option<u64>,
    pub context_length: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeResources {
    pub node_id: String,
    pub label: String,
    pub hostname: String,
    pub role: PeerRole,
    pub address: String,
    pub port: u16,
    pub cpu_cores: u32,
    pub cpu_usage_pct: f32,
    pub ram_total_mb: u64,
    pub ram_available_mb: u64,
    pub gpus: Vec<GpuDevice>,
    pub loaded_models: Vec<LoadedModel>,
    pub active_remote_tasks: u32,
    pub mesh_enabled: bool,
    pub network_tier: NetworkTier,
    pub public_key_hex: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MeshPeerInfo {
    pub peer_id: String,
    pub public_key: String,
    pub label: Option<String>,
    pub trusted: bool,
    pub address: Option<String>,
    pub port: Option<u16>,
    pub source: PeerSource,
    pub role: PeerRole,
    pub state: PeerConnectionState,
    pub fingerprint: Option<String>,
    pub capabilities: PeerCapabilities,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub paired_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct PeerCapabilities {
    #[serde(default)]
    pub can_infer: bool,
    #[serde(default)]
    pub can_accept_tasks: bool,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub vram_total_mb: u64,
    #[serde(default)]
    pub vram_free_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscoveredPeer {
    pub instance_name: String,
    pub peer_id: Option<String>,
    pub host: String,
    pub port: u16,
    pub addresses: Vec<String>,
    pub txt: std::collections::HashMap<String, String>,
    pub discovered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MeshStatus {
    pub enabled: bool,
    pub advertising: bool,
    pub browsing: bool,
    pub node_id: String,
    pub public_key_hex: String,
    pub label: String,
    pub hostname: String,
    pub role: PeerRole,
    pub bind_port: u16,
    pub service_type: String,
    pub network_tier: NetworkTier,
    pub mesh_allowed: bool,
    pub paired_peers: usize,
    pub connected_peers: usize,
    pub discovered_peers: usize,
    pub pending_pairings: usize,
    pub active_remote_tasks: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PairingSession {
    pub pairing_id: Uuid,
    pub pin: String,
    pub node_id: String,
    pub public_key_hex: String,
    pub label: String,
    pub expires_at: DateTime<Utc>,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClusterSnapshot {
    pub local: NodeResources,
    pub peers: Vec<ClusterPeerSnapshot>,
    pub active_tasks: Vec<RemoteTaskInfo>,
    pub mesh_status: MeshStatus,
    pub collected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClusterPeerSnapshot {
    pub peer: MeshPeerInfo,
    pub resources: Option<NodeResources>,
    pub reachable: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RemoteTaskState {
    Pending,
    Assigned,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl RemoteTaskState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Assigned => "assigned",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::str::FromStr for RemoteTaskState {
    type Err = crate::error::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "assigned" => Ok(Self::Assigned),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" | "canceled" => Ok(Self::Cancelled),
            other => Err(crate::error::CoreError::InvalidArgument(format!(
                "unknown task state: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RemoteTaskKind {
    Infer,
    Tool,
    PlanSubtask,
    Custom,
}

impl RemoteTaskKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Infer => "infer",
            Self::Tool => "tool",
            Self::PlanSubtask => "plan_subtask",
            Self::Custom => "custom",
        }
    }
}

impl std::str::FromStr for RemoteTaskKind {
    type Err = crate::error::CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "infer" | "inference" | "llm" => Ok(Self::Infer),
            "tool" => Ok(Self::Tool),
            "plan_subtask" | "subtask" | "plan" => Ok(Self::PlanSubtask),
            "custom" => Ok(Self::Custom),
            other => Err(crate::error::CoreError::InvalidArgument(format!(
                "unknown task kind: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoteTaskSpec {
    pub kind: RemoteTaskKind,
    pub label: String,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub required_model: Option<String>,
    #[serde(default)]
    pub min_vram_mb: Option<u64>,
    #[serde(default)]
    pub preferred_peer_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoteTaskInfo {
    pub task_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub kind: RemoteTaskKind,
    pub label: String,
    pub state: RemoteTaskState,
    pub assigned_peer_id: Option<String>,
    pub required_model: Option<String>,
    pub min_vram_mb: Option<u64>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DispatchResult {
    pub batch_id: Uuid,
    pub tasks: Vec<RemoteTaskInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoteInferRequest {
    pub request: LlmRequest,
    #[serde(default)]
    pub required_model: Option<String>,
    #[serde(default)]
    pub min_vram_mb: Option<u64>,
    #[serde(default)]
    pub preferred_peer_id: Option<String>,
    #[serde(default)]
    pub allow_local_fallback: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoteInferResponse {
    pub response: LlmResponse,
    pub routed_peer_id: String,
    pub routed_address: String,
    pub local: bool,
}

/// Whether the network tier permits mesh / LAN peer traffic.
pub fn mesh_tier_allowed(tier: NetworkTier) -> bool {
    matches!(
        tier,
        NetworkTier::LocalAndLan | NetworkTier::UnrestrictedConfigurable
    )
}
