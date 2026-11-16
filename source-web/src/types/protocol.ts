/** TypeScript mirrors of source-core protocol types (kept in sync by hand). Protocol 1.4.0. */

export const PROTOCOL_VERSION = "1.4.0";

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

export interface CheckpointMeta {
  id: string;
  session_id: string;
  step_index: number;
  git_commit: string;
  label: string;
  created_at: string;
  audit_event_id: string | null;
}

export type AuditEventKind =
  | "shell"
  | "file_diff"
  | "network_attempt"
  | "delegation"
  | "mode_change"
  | "checkpoint"
  | "rollback"
  | "approval"
  | "provider_call"
  | "tool";

export interface AuditEvent {
  id: string;
  session_id: string | null;
  kind: AuditEventKind;
  summary: string;
  detail: unknown;
  content_hash: string;
  prev_hash: string | null;
  created_at: string;
}

export type LlmProviderKind =
  | "openai_compatible"
  | "ollama"
  | "llama_cpp"
  | "lm_studio"
  | "anthropic"
  | "openai"
  | "gemini"
  | "open_router"
  | "custom";

export interface ProviderConfig {
  id: string;
  kind: LlmProviderKind | string;
  display_name: string;
  base_url: string;
  model: string;
  api_key_env: string | null;
  enabled: boolean;
  extra_headers: Record<string, string>;
}

export interface LlmMessage {
  role: "system" | "user" | "assistant" | "tool";
  content: string;
  tool_call_id?: string;
  name?: string;
}

export interface LlmRequest {
  provider_id: string;
  messages: LlmMessage[];
  temperature?: number;
  max_tokens?: number;
  stream?: boolean;
}

export interface LlmResponse {
  provider_id: string;
  model: string;
  content: string;
  prompt_tokens: number | null;
  completion_tokens: number | null;
  finish_reason: string | null;
}

export interface ProbeProviderParams {
  provider_id?: string | null;
  kind?: LlmProviderKind | string | null;
  base_url?: string | null;
  api_key_env?: string | null;
}

export interface ProviderProbeResult {
  ok: boolean;
  latency_ms: number;
  provider_id?: string | null;
  base_url: string;
  kind: LlmProviderKind | string;
  models: string[];
  error?: string | null;
  tier: NetworkTier;
  network_allowed: boolean;
}

export interface ListModelsResult {
  provider_id: string;
  base_url: string;
  kind: LlmProviderKind | string;
  models: string[];
}

export interface SelectedProviderResult {
  provider_id?: string | null;
  provider?: ProviderConfig | null;
}

export interface ToolCall {
  id: string;
  name: string;
  arguments: unknown;
}

export interface ToolResult {
  call_id: string;
  name: string;
  ok: boolean;
  output: string;
  error?: string | null;
}

export interface ApprovalRecord {
  id: string;
  session_id: string | null;
  category: string;
  summary: string;
  detail: unknown;
  status: string;
  created_at?: string;
}

export const Methods = {
  Status: "source/status",
  GetNetworkTier: "source/network/getTier",
  SetNetworkTier: "source/network/setTier",
  EvaluateNetwork: "source/network/evaluate",
  CreateSession: "source/session/create",
  ListSessions: "source/session/list",
  GetSession: "source/session/get",
  SetMode: "source/mode/set",
  ListProviders: "source/llm/listProviders",
  UpsertProvider: "source/llm/upsertProvider",
  EnableProvider: "source/llm/enableProvider",
  DisableProvider: "source/llm/disableProvider",
  ProbeProvider: "source/llm/probeProvider",
  ListModels: "source/llm/listModels",
  GetSelectedProvider: "source/llm/getSelectedProvider",
  SetSelectedProvider: "source/llm/setSelectedProvider",
  SetSessionProvider: "source/session/setProvider",
  Complete: "source/llm/complete",
  CreateCheckpoint: "source/checkpoint/create",
  ListCheckpoints: "source/checkpoint/list",
  Rollback: "source/checkpoint/rollback",
  ResolveApproval: "source/approval/resolve",
  ListPendingApprovals: "source/approval/listPending",
  ListTools: "source/tools/list",
  InvokeTool: "source/tools/invoke",
  ListAuditEvents: "source/audit/list",
  SearchSymbols: "source/context/searchSymbols",
  BuildRepoMap: "source/context/buildRepoMap",
  PtyOpen: "source/pty/open",
  PtyWrite: "source/pty/write",
  PtyRead: "source/pty/read",
  PtyResize: "source/pty/resize",
  PtyClose: "source/pty/close",
  PtyList: "source/pty/list",
  RagIndex: "source/rag/index",
  RagQuery: "source/rag/query",
  RagStatus: "source/rag/status",
  TemplateList: "source/templates/list",
  TemplateScaffold: "source/templates/scaffold",
  CacheVendorStatus: "source/cache/vendorStatus",
  McpList: "source/mcp/list",
  McpListAll: "source/mcp/listAll",
  McpReload: "source/mcp/reload",
  McpCall: "source/mcp/call",
  McpUpsert: "source/mcp/upsert",
  McpDelete: "source/mcp/delete",
  McpSetEnabled: "source/mcp/setEnabled",
  StateGet: "source/state/get",
  StateSet: "source/state/set",
  BrowserNavigate: "source/browser/navigate",
  BrowserInspect: "source/browser/inspect",
  AppVerify: "source/browser/verifyApp",
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

export interface TemplateInfo {
  id: string;
  title: string;
  description: string;
  path: string;
  builtin: boolean;
}

export interface VendorCacheStatus {
  root: string;
  npm_dir: string;
  pnpm_dir: string;
  cargo_dir: string;
  wheels_dir: string;
  npm_bytes: number;
  pnpm_bytes: number;
  cargo_bytes: number;
  wheels_bytes: number;
  helpers: {
    npm_offline_install: string;
    pnpm_offline_install: string;
    cargo_vendor: string;
    pip_wheelhouse: string;
    create_desktop_app: string;
    build_desktop: string;
    linux_tauri_deps: string;
  };
}

export interface RagStatus {
  docs_root?: string;
  indexed_docs?: number;
  indexed_chunks?: number;
  status?: string;
  [key: string]: unknown;
}

export interface RagHit {
  path?: string;
  title?: string;
  snippet?: string;
  score?: number;
  [key: string]: unknown;
}

export interface McpToolInfo {
  name: string;
  description: string;
  input_schema: unknown;
  offline_capable: boolean;
  requires_wan: boolean;
}

export interface McpServerInfo {
  id: string;
  name: string;
  transport: string;
  offline_capable: boolean;
  requires_wan: boolean;
  enabled: boolean;
  running: boolean;
  tools: McpToolInfo[];
  path: string;
  command?: string[] | null;
  url?: string | null;
  env?: Record<string, string>;
}

export type McpTransport = "stdio" | "sse" | "streamable_http";

export interface McpUpsertParams {
  id: string;
  name: string;
  transport: McpTransport;
  command?: string[] | null;
  url?: string | null;
  offline_capable?: boolean;
  requires_wan?: boolean;
  enabled?: boolean;
  env?: Record<string, string>;
}

export interface PtySessionInfo {
  pty_id: string;
  cwd?: string;
  shell?: string;
  cols?: number;
  rows?: number;
  [key: string]: unknown;
}
