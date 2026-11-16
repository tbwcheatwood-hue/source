import { useCallback, useEffect, useMemo, useState } from "react";
import { ConnectionState, SourceClient } from "../api/client";
import {
  DaemonStatus,
  Mode,
  NetworkTier,
  NetworkTierResult,
  SessionMeta,
} from "../types/protocol";

const WORKSPACE_KEY = "source.workspaceRoot";
const SESSION_KEY = "source.sessionId";

function defaultWorkspace(): string {
  if (typeof window === "undefined") return ".";
  return localStorage.getItem(WORKSPACE_KEY) ?? ".";
}

export function useDaemon() {
  const client = useMemo(() => new SourceClient(), []);
  const [conn, setConn] = useState<ConnectionState>("idle");
  const [connDetail, setConnDetail] = useState<string | undefined>();
  const [status, setStatus] = useState<DaemonStatus | null>(null);
  const [tierInfo, setTierInfo] = useState<NetworkTierResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [helloTier, setHelloTier] = useState<NetworkTier | null>(null);
  const [session, setSession] = useState<SessionMeta | null>(null);
  const [selectedProviderId, setSelectedProviderId] = useState<string | null>(null);
  const [workspaceRoot, setWorkspaceRootState] = useState(defaultWorkspace);

  const setWorkspaceRoot = useCallback((root: string) => {
    setWorkspaceRootState(root);
    localStorage.setItem(WORKSPACE_KEY, root);
  }, []);

  const ensureSession = useCallback(async (): Promise<SessionMeta> => {
    if (session) return session;
    const saved = localStorage.getItem(SESSION_KEY);
    if (saved) {
      try {
        const listed = await client.listSessions();
        const found = listed.find((s) => s.id === saved);
        if (found) {
          setSession(found);
          setWorkspaceRoot(found.workspace_root);
          return found;
        }
      } catch {
        /* create below */
      }
    }
    const created = await client.createSession("Source IDE", workspaceRoot, "plan");
    localStorage.setItem(SESSION_KEY, created.id);
    setSession(created);
    return created;
  }, [client, session, workspaceRoot, setWorkspaceRoot]);

  const refresh = useCallback(async () => {
    try {
      const [s, t, selected] = await Promise.all([
        client.status(),
        client.getNetworkTier(),
        client.getSelectedProvider().catch(() => null),
      ]);
      setStatus(s);
      setTierInfo(t);
      if (selected) {
        setSelectedProviderId(selected.provider_id ?? null);
      }
      setError(null);
      try {
        await ensureSession();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client, ensureSession]);

  useEffect(() => {
    client.onState = (s, detail) => {
      setConn(s);
      setConnDetail(detail);
    };
    client.onTier = (t) => setHelloTier(t);
    client.connectWs();
    void refresh();
    const id = window.setInterval(() => void refresh(), 8000);
    return () => {
      window.clearInterval(id);
      client.disconnect();
    };
  }, [client, refresh]);

  const setTier = async (tier: NetworkTier) => {
    const t = await client.setNetworkTier(tier);
    setTierInfo(t);
    const s = await client.status();
    setStatus(s);
  };

  const setMode = async (mode: Mode) => {
    const sess = await ensureSession();
    const res = await client.setMode(sess.id, mode);
    setSession({ ...sess, mode: res.mode });
    await refresh();
  };

  return {
    client,
    conn,
    connDetail,
    status,
    tierInfo,
    helloTier,
    error,
    session,
    setSession,
    selectedProviderId,
    setSelectedProviderId,
    workspaceRoot,
    setWorkspaceRoot,
    ensureSession,
    refresh,
    setTier,
    setMode,
  };
}

export type DaemonContext = ReturnType<typeof useDaemon>;
