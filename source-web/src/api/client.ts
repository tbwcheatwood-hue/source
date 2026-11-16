import {
  JsonRpcRequest,
  JsonRpcResponse,
  Methods,
  DaemonStatus,
  NetworkTier,
  NetworkTierResult,
  PROTOCOL_VERSION,
} from "../types/protocol";

export type ConnectionState = "idle" | "connecting" | "open" | "closed" | "error";

function defaultHttpBase(): string {
  if (typeof window === "undefined") return "http://127.0.0.1:7420";
  const params = new URLSearchParams(window.location.search);
  return params.get("daemon") ?? `${window.location.protocol}//${window.location.hostname}:7420`;
}

function defaultWsUrl(): string {
  if (typeof window === "undefined") return "ws://127.0.0.1:7420/ws";
  const params = new URLSearchParams(window.location.search);
  const override = params.get("ws");
  if (override) return override;
  const http = defaultHttpBase();
  const u = new URL(http);
  u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
  u.pathname = "/ws";
  u.port = u.port || "7420";
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
  onState?: (state: ConnectionState, detail?: string) => void;
  onNotification?: (method: string, params: unknown) => void;
  onTier?: (tier: NetworkTier) => void;

  constructor(httpBase = defaultHttpBase(), wsUrl = defaultWsUrl()) {
    this.httpBase = httpBase.replace(/\/$/, "");
    this.wsUrl = wsUrl;
  }

  get protocolVersion(): string {
    return PROTOCOL_VERSION;
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

  connectWs(): void {
    if (this.socket && (this.socket.readyState === WebSocket.OPEN || this.socket.readyState === WebSocket.CONNECTING)) {
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

  /** Prefer WS when open; otherwise HTTP. */
  async rpc<T>(method: string, params: unknown = {}): Promise<T> {
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

  disconnect(): void {
    this.socket?.close();
    this.socket = null;
  }
}