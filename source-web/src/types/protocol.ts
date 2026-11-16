/** TypeScript mirrors of source-core protocol types (kept in sync by hand for Phase 1–2). */

export const PROTOCOL_VERSION = "1.0.0";

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
} as const;

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