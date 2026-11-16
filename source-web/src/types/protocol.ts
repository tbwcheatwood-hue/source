/** TypeScript mirrors of source-core protocol types (kept in sync by hand). Protocol 1.2.0. */

export const PROTOCOL_VERSION = "1.2.0";

export type NetworkTier =
  | "LOCAL_ONLY"
  | "LOCAL_AND_LAN"
  | "UNRESTRICTED_CONFIGURABLE";

export type Mode = "plan" | "act";

export type JsonRpcId = number | string | null;

export interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: JsonRpcId;
  method: string;
  params?: unknown;
}

export interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}

export interface JsonRpcResponse<T = unknown> {
  jsonrpc: "2.0";
  id: JsonRpcId;
  result?: T;
  error?: JsonRpcError;
}

export interface NetworkTierResult {
  tier: NetworkTier;
  allowlist: string[];
  denylist: string[];
  require_approval_for_public: boolean;
}

export interface ProviderStatus {
  id: string;
  kind: string;
  display_name: string;
  enabled: boolean;
  reachable: boolean | null;
  base_url: string;
}

export interface DaemonStatus {
  version: string;
  protocol_version: string;
  mode: Mode;
  network_tier: NetworkTier;
  data_root: string;
  active_session: string | null;
  uptime_secs: number;
  providers: ProviderStatus[];
}

export interface SessionMeta {
  id: string;
  title: string;
  mode: Mode;
  workspace_root: string;
  created_at: string;
  updated_at: string;
  network_tier: NetworkTier;
  provider_id: string | null;
}

export const Methods = {
  Status: "source/status",
  GetNetworkTier: "source/network/getTier",
  SetNetworkTier: "source/network/setTier",
  EvaluateNetwork: "source/network/evaluate",
  CreateSession: "source/session/create",
  ListSessions: "source/session/list",
  SetMode: "source/mode/set",
  ListProviders: "source/llm/listProviders",
  ListTools: "source/tools/list",
  ListAuditEvents: "source/audit/list",
  SearchSymbols: "source/context/searchSymbols",
  PtyOpen: "source/pty/open",
  PtyWrite: "source/pty/write",
  PtyRead: "source/pty/read",
  PtyClose: "source/pty/close",
  PtyList: "source/pty/list",
  RagIndex: "source/rag/index",
  RagQuery: "source/rag/query",
  RagStatus: "source/rag/status",
  TemplateList: "source/templates/list",
  TemplateScaffold: "source/templates/scaffold",
  CacheVendorStatus: "source/cache/vendorStatus",
  McpList: "source/mcp/list",
  McpReload: "source/mcp/reload",
  McpCall: "source/mcp/call",
  BrowserNavigate: "source/browser/navigate",
  BrowserInspect: "source/browser/inspect",
  AppVerify: "source/browser/verifyApp",
  // Phase 4
  MeshStart: "source/mesh/start",
  MeshStop: "source/mesh/stop",
  MeshStatus: "source/mesh/status",
  MeshDiscover: "source/mesh/discover",
  MeshRegisterPeer: "source/mesh/registerPeer",
  MeshUnregisterPeer: "source/mesh/unregisterPeer",
  MeshListPeers: "source/mesh/listPeers",
  MeshBeginPairing: "source/mesh/beginPairing",
  MeshConfirmPairing: "source/mesh/confirmPairing",
  MeshCompletePairing: "source/mesh/completePairing",
  MeshRejectPairing: "source/mesh/rejectPairing",
  MeshTrustPeer: "source/mesh/trustPeer",
  MeshRevokePeer: "source/mesh/revokePeer",
  ClusterResources: "source/cluster/resources",
  ClusterDispatch: "source/cluster/dispatch",
  ClusterTaskStatus: "source/cluster/taskStatus",
  ClusterCancelTask: "source/cluster/cancelTask",
  ClusterListTasks: "source/cluster/listTasks",
  LlmCompleteRemote: "source/llm/completeRemote",
} as const;

export interface ToolDefinition {
  name: string;
  description: string;
  input_schema: unknown;
  categories: string[];
  enabled: boolean;
  offline_capable: boolean;
  requires_wan: boolean;
}

export type DestinationClass =
  | "loopback"
  | "private_lan"
  | "cgnat"
  | "public"
  | "unknown";

export type NetworkDecision =
  | { allow: { class: DestinationClass } }
  | { deny: { reason: string; class: DestinationClass } }
  | { needs_approval: { class: DestinationClass; host: string } };

/** Phase 4 mesh / cluster types */

export type PeerRole = "orchestrator" | "worker" | "hybrid";
export type PeerSource = "mdns" | "manual" | "tailscale";
export type PeerConnectionState =
  | "discovered"
  | "pairing"
  | "paired"
  | "connected"
  | "offline"
  | "untrusted";

export interface GpuDevice {
  index: number;
  name: string;
  vram_total_mb: number;
  vram_free_mb: number;
  utilization_pct: number | null;
}

export interface LoadedModel {
  name: string;
  provider_id: string | null;
  vram_mb: number | null;
  context_length: number | null;
}

export interface PeerCapabilities {
  can_infer: boolean;
  can_accept_tasks: boolean;
  models: string[];
  vram_total_mb: number;
  vram_free_mb: number;
}

export interface MeshPeerInfo {
  peer_id: string;
  public_key: string;
  label: string | null;
  trusted: boolean;
  address: string | null;
  port: number | null;
  source: PeerSource;
  role: PeerRole;
  state: PeerConnectionState;
  fingerprint: string | null;
  capabilities: PeerCapabilities;
  last_seen_at: string | null;
  paired_at: string | null;
  created_at: string;
}

export interface DiscoveredPeer {
  instance_name: string;
  peer_id: string | null;
  host: string;
  port: number;
  addresses: string[];
  txt: Record<string, string>;
  discovered_at: string;
}

export interface MeshStatus {
  enabled: boolean;
  advertising: boolean;
  browsing: boolean;
  node_id: string;
  public_key_hex: string;
  label: string;
  hostname: string;
  role: PeerRole;
  bind_port: number;
  service_type: string;
  network_tier: NetworkTier;
  mesh_allowed: boolean;
  paired_peers: number;
  connected_peers: number;
  discovered_peers: number;
  pending_pairings: number;
  active_remote_tasks: number;
}

export interface PairingSession {
  pairing_id: string;
  pin: string;
  node_id: string;
  public_key_hex: string;
  label: string;
  expires_at: string;
  role: string;
}

export interface NodeResources {
  node_id: string;
  label: string;
  hostname: string;
  role: PeerRole;
  address: string;
  port: number;
  cpu_cores: number;
  cpu_usage_pct: number;
  ram_total_mb: number;
  ram_available_mb: number;
  gpus: GpuDevice[];
  loaded_models: LoadedModel[];
  active_remote_tasks: number;
  mesh_enabled: boolean;
  network_tier: NetworkTier;
  public_key_hex: string;
  updated_at: string;
}

export interface ClusterPeerSnapshot {
  peer: MeshPeerInfo;
  resources: NodeResources | null;
  reachable: boolean;
  last_error: string | null;
}

export type RemoteTaskState =
  | "pending"
  | "assigned"
  | "running"
  | "completed"
  | "failed"
  | "cancelled";

export type RemoteTaskKind = "infer" | "tool" | "plan_subtask" | "custom";

export interface RemoteTaskInfo {
  task_id: string;
  parent_id: string | null;
  kind: RemoteTaskKind;
  label: string;
  state: RemoteTaskState;
  assigned_peer_id: string | null;
  required_model: string | null;
  min_vram_mb: number | null;
  result: unknown | null;
  error: string | null;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
}

export interface ClusterSnapshot {
  local: NodeResources;
  peers: ClusterPeerSnapshot[];
  active_tasks: RemoteTaskInfo[];
  mesh_status: MeshStatus;
  collected_at: string;
}

export interface DispatchResult {
  batch_id: string;
  tasks: RemoteTaskInfo[];
}

export interface RemoteInferResponse {
  response: {
    provider_id: string;
    model: string;
    content: string;
    prompt_tokens: number | null;
    completion_tokens: number | null;
    finish_reason: string | null;
  };
  routed_peer_id: string;
  routed_address: string;
  local: boolean;
}
