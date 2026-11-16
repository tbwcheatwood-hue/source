import { isTauri, tauriInvoke } from "../lib/tauri";
import {
  JsonRpcRequest,
  JsonRpcResponse,
  Methods,
  DaemonStatus,
  NetworkTier,
  NetworkTierResult,
  PROTOCOL_VERSION,
  SessionMeta,
  CheckpointMeta,
  AuditEvent,
  LlmRequest,
  LlmResponse,
  ProviderConfig,
  ProbeProviderParams,
  ProviderProbeResult,
  ListModelsResult,
  SelectedProviderResult,
  ToolDefinition,
  ToolCall,
  ToolResult,
  ApprovalRecord,
  ClusterSnapshot,
  MeshStatus,
  MeshPeerInfo,
  DiscoveredPeer,
  PairingSession,
  TemplateInfo,
  VendorCacheStatus,
  RagStatus,
  RagHit,
  McpServerInfo,
  Mode,
} from "../types/protocol";

export type ConnectionState = "idle" | "connecting" | "open" | "closed" | "error";

function defaultHttpBase(): string {
  if (typeof window === "undefined") return "http://127.0.0.1:7420";
  const params = new URLSearchParams(window.location.search);
  const override = params.get("daemon");
  if (override) return override.replace(/\/$/, "");
  // Vite dev: relative URLs hit the proxy. Production static: same origin as daemon.
  if (import.meta.env.DEV) return "";
  return window.location.origin;
}

function defaultWsUrl(httpBase: string): string {
  if (typeof window === "undefined") return "ws://127.0.0.1:7420/ws";
  const params = new URLSearchParams(window.location.search);
  const override = params.get("ws");
  if (override) return override;
  if (!httpBase) {
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    return `${proto}//${window.location.host}/ws`;
  }
  const u = new URL(httpBase);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  u.pathname = "/ws";
  return u.toString();
}

export class SourceClient {
  httpBase: string;
  wsUrl: string;
  private nextId = 1;
  private socket: WebSocket | null = null;
  private pending = new Map<
    number | string,
    { resolve: (v: unknown) => void; reject: (e: Error) => void }
  >();
  private useIpc: boolean;
  onState?: (state: ConnectionState, detail?: string) => void;
  onNotification?: (method: string, params: unknown) => void;
  onTier?: (tier: NetworkTier) => void;

  constructor(httpBase = defaultHttpBase(), wsUrl?: string) {
    this.httpBase = httpBase.replace(/\/$/, "");
    this.wsUrl = wsUrl ?? defaultWsUrl(this.httpBase);
    this.useIpc = isTauri();
  }

  get protocolVersion(): string {
    return PROTOCOL_VERSION;
  }

  get transport(): "tauri-ipc" | "websocket" | "http" {
    if (this.useIpc) return "tauri-ipc";
    if (this.socket?.readyState === WebSocket.OPEN) return "websocket";
    return "http";
  }

  private setState(state: ConnectionState, detail?: string) {
    this.onState?.(state, detail);
  }

  async rpcHttp<T>(method: string, params: unknown = {}): Promise<T> {
    const id = this.nextId++;
    const body: JsonRpcRequest = {
      jsonrpc: "2.0",
      id,
      method,
      params,
    };
    const res = await fetch(`${this.httpBase}/rpc`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!res.ok) {
      throw new Error(`HTTP ${res.status}`);
    }
    const data = (await res.json()) as JsonRpcResponse<T>;
    if (data.error) {
      throw new Error(`${data.error.code}: ${data.error.message}`);
    }
    return data.result as T;
  }

  async rpcIpc<T>(method: string, params: unknown = {}): Promise<T> {
    return tauriInvoke<T>("daemon_rpc", {
      method,
      params: params ?? {},
    });
  }

  connectWs(): void {
    if (this.useIpc) {
      this.setState("open", "tauri-ipc");
      return;
    }
    if (
      this.socket &&
      (this.socket.readyState === WebSocket.OPEN || this.socket.readyState === WebSocket.CONNECTING)
    ) {
      return;
    }
    this.setState("connecting");
    const ws = new WebSocket(this.wsUrl);
    this.socket = ws;
    ws.onopen = () => this.setState("open");
    ws.onerror = () => this.setState("error", "WebSocket error");
    ws.onclose = () => {
      this.setState("closed");
      for (const [, p] of this.pending) {
        p.reject(new Error("WebSocket closed"));
      }
      this.pending.clear();
    };
    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(String(ev.data)) as JsonRpcResponse & {
          method?: string;
          params?: unknown;
        };
        if (msg.method && msg.id === undefined) {
          this.onNotification?.(msg.method, msg.params);
          if (msg.method === "source/hello") {
            const tier = (msg.params as { tier?: NetworkTier } | undefined)?.tier;
            if (tier) this.onTier?.(tier);
          }
          return;
        }
        if (msg.id === undefined || msg.id === null) return;
        const pending = this.pending.get(msg.id);
        if (!pending) return;
        this.pending.delete(msg.id);
        if (msg.error) {
          pending.reject(new Error(`${msg.error.code}: ${msg.error.message}`));
        } else {
          pending.resolve(msg.result);
        }
      } catch (err) {
        this.setState("error", String(err));
      }
    };
  }

  rpcWs<T>(method: string, params: unknown = {}): Promise<T> {
    this.connectWs();
    const ws = this.socket;
    if (!ws) return Promise.reject(new Error("no socket"));
    const id = this.nextId++;
    const payload: JsonRpcRequest = { jsonrpc: "2.0", id, method, params };
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, {
        resolve: (v) => resolve(v as T),
        reject,
      });
      const send = () => ws.send(JSON.stringify(payload));
      if (ws.readyState === WebSocket.OPEN) send();
      else {
        ws.addEventListener("open", send, { once: true });
      }
    });
  }

  /** Prefer Tauri IPC, then WS when open; otherwise HTTP. */
  async rpc<T>(method: string, params: unknown = {}): Promise<T> {
    if (this.useIpc) {
      return this.rpcIpc<T>(method, params);
    }
    if (this.socket?.readyState === WebSocket.OPEN) {
      return this.rpcWs<T>(method, params);
    }
    return this.rpcHttp<T>(method, params);
  }

  status(): Promise<DaemonStatus> {
    return this.rpc(Methods.Status);
  }

  getNetworkTier(): Promise<NetworkTierResult> {
    return this.rpc(Methods.GetNetworkTier);
  }

  setNetworkTier(tier: NetworkTier): Promise<NetworkTierResult> {
    return this.rpc(Methods.SetNetworkTier, { tier });
  }

  createSession(title: string, workspaceRoot: string, mode?: Mode): Promise<SessionMeta> {
    return this.rpc(Methods.CreateSession, {
      title,
      workspace_root: workspaceRoot,
      mode: mode ?? "plan",
    });
  }

  listSessions(): Promise<SessionMeta[]> {
    return this.rpc(Methods.ListSessions);
  }

  setMode(sessionId: string, mode: Mode): Promise<{ session_id: string; mode: Mode }> {
    return this.rpc(Methods.SetMode, { session_id: sessionId, mode });
  }

  complete(req: LlmRequest): Promise<LlmResponse> {
    return this.rpc(Methods.Complete, req);
  }

  listProviders(): Promise<ProviderConfig[]> {
    return this.rpc(Methods.ListProviders);
  }

  upsertProvider(config: ProviderConfig): Promise<ProviderConfig> {
    return this.rpc(Methods.UpsertProvider, config);
  }

  enableProvider(providerId: string): Promise<{ provider_id: string; enabled: boolean }> {
    return this.rpc(Methods.EnableProvider, { provider_id: providerId });
  }

  disableProvider(providerId: string): Promise<{ provider_id: string; enabled: boolean }> {
    return this.rpc(Methods.DisableProvider, { provider_id: providerId });
  }

  probeProvider(params: ProbeProviderParams): Promise<ProviderProbeResult> {
    return this.rpc(Methods.ProbeProvider, params);
  }

  listModels(providerId: string): Promise<ListModelsResult> {
    return this.rpc(Methods.ListModels, { provider_id: providerId });
  }

  getSelectedProvider(): Promise<SelectedProviderResult> {
    return this.rpc(Methods.GetSelectedProvider);
  }

  setSelectedProvider(providerId: string): Promise<SelectedProviderResult> {
    return this.rpc(Methods.SetSelectedProvider, { provider_id: providerId });
  }

  setSessionProvider(sessionId: string, providerId: string): Promise<SessionMeta> {
    return this.rpc(Methods.SetSessionProvider, {
      session_id: sessionId,
      provider_id: providerId,
    });
  }

  listTools(): Promise<ToolDefinition[]> {
    return this.rpc(Methods.ListTools);
  }

  invokeTool(sessionId: string, call: ToolCall): Promise<ToolResult> {
    return this.rpc(Methods.InvokeTool, { session_id: sessionId, call });
  }

  listAudit(sessionId?: string | null, limit = 100): Promise<AuditEvent[]> {
    return this.rpc(Methods.ListAuditEvents, {
      session_id: sessionId ?? null,
      limit,
    });
  }

  createCheckpoint(
    sessionId: string,
    workspaceRoot: string,
    label: string,
  ): Promise<CheckpointMeta> {
    return this.rpc(Methods.CreateCheckpoint, {
      session_id: sessionId,
      workspace_root: workspaceRoot,
      label,
    });
  }

  listCheckpoints(sessionId: string): Promise<CheckpointMeta[]> {
    return this.rpc(Methods.ListCheckpoints, { session_id: sessionId });
  }

  rollback(
    sessionId: string,
    checkpointId: string,
    workspaceRoot: string,
  ): Promise<CheckpointMeta> {
    return this.rpc(Methods.Rollback, {
      session_id: sessionId,
      checkpoint_id: checkpointId,
      workspace_root: workspaceRoot,
    });
  }

  listPendingApprovals(): Promise<ApprovalRecord[]> {
    return this.rpc(Methods.ListPendingApprovals);
  }

  resolveApproval(approvalId: string, decision: "approved" | "denied"): Promise<unknown> {
    return this.rpc(Methods.ResolveApproval, {
      approval_id: approvalId,
      decision,
    });
  }

  ptyOpen(opts: {
    cwd?: string;
    shell?: string;
    cols?: number;
    rows?: number;
  } = {}): Promise<{ pty_id: string }> {
    return this.rpc(Methods.PtyOpen, opts);
  }

  ptyWrite(ptyId: string, data: string): Promise<unknown> {
    return this.rpc(Methods.PtyWrite, { pty_id: ptyId, data });
  }

  ptyRead(
    ptyId: string,
    opts: { max_bytes?: number; wait_ms?: number } = {},
  ): Promise<{ data?: string; alive?: boolean; exit_code?: number | null; [key: string]: unknown }> {
    return this.rpc(Methods.PtyRead, { pty_id: ptyId, ...opts });
  }

  ptyResize(ptyId: string, cols: number, rows: number): Promise<unknown> {
    return this.rpc(Methods.PtyResize, { pty_id: ptyId, cols, rows });
  }

  ptyClose(ptyId: string): Promise<unknown> {
    return this.rpc(Methods.PtyClose, { pty_id: ptyId });
  }

  meshStatus(): Promise<MeshStatus> {
    return this.rpc(Methods.MeshStatus);
  }

  meshStart(params: Record<string, unknown> = {}): Promise<MeshStatus> {
    return this.rpc(Methods.MeshStart, params);
  }

  meshStop(): Promise<unknown> {
    return this.rpc(Methods.MeshStop);
  }

  meshDiscover(): Promise<DiscoveredPeer[]> {
    return this.rpc(Methods.MeshDiscover);
  }

  meshListPeers(): Promise<MeshPeerInfo[]> {
    return this.rpc(Methods.MeshListPeers);
  }

  meshRegisterPeer(host: string, port?: number, label?: string): Promise<MeshPeerInfo> {
    return this.rpc(Methods.MeshRegisterPeer, {
      host,
      port: port ?? null,
      label: label ?? null,
      source: "manual",
    });
  }

  meshBeginPairing(label?: string): Promise<PairingSession> {
    return this.rpc(Methods.MeshBeginPairing, { label: label ?? null });
  }

  meshCompletePairing(host: string, pin: string, port?: number): Promise<unknown> {
    return this.rpc(Methods.MeshCompletePairing, {
      host,
      pin,
      port: port ?? null,
    });
  }

  meshTrustPeer(peerId: string, trusted: boolean): Promise<unknown> {
    return this.rpc(Methods.MeshTrustPeer, { peer_id: peerId, trusted });
  }

  clusterResources(): Promise<ClusterSnapshot> {
    return this.rpc(Methods.ClusterResources);
  }

  templateList(): Promise<TemplateInfo[]> {
    return this.rpc(Methods.TemplateList);
  }

  templateScaffold(templateId: string, dest: string, name?: string): Promise<unknown> {
    return this.rpc(Methods.TemplateScaffold, {
      template_id: templateId,
      dest,
      name: name ?? null,
    });
  }

  cacheVendorStatus(): Promise<VendorCacheStatus> {
    return this.rpc(Methods.CacheVendorStatus);
  }

  ragStatus(): Promise<RagStatus> {
    return this.rpc(Methods.RagStatus);
  }

  ragIndex(force = false): Promise<RagStatus> {
    return this.rpc(Methods.RagIndex, { force });
  }

  ragQuery(query: string, topK = 8): Promise<{ hits?: RagHit[]; [key: string]: unknown }> {
    return this.rpc(Methods.RagQuery, { query, top_k: topK });
  }

  mcpList(): Promise<McpServerInfo[]> {
    return this.rpc(Methods.McpList);
  }

  mcpReload(): Promise<McpServerInfo[]> {
    return this.rpc(Methods.McpReload);
  }

  mcpCall(serverId: string, toolName: string, args: unknown = {}): Promise<unknown> {
    return this.rpc(Methods.McpCall, {
      server_id: serverId,
      tool_name: toolName,
      arguments: args,
    });
  }

  async fsOp(
    sessionId: string,
    op: string,
    path: string,
    extra: Record<string, unknown> = {},
  ): Promise<ToolResult> {
    return this.invokeTool(sessionId, {
      id: `fs-${Date.now()}`,
      name: "fs_ops",
      arguments: { op, path, ...extra },
    });
  }

  disconnect(): void {
    this.socket?.close();
    this.socket = null;
  }
}
