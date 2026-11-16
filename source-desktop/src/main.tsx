import { StrictMode, useCallback, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";

interface BridgeInfo {
  protocol_version: string;
  daemon_base_url: string;
  desktop_version: string;
}

function DesktopApp() {
  const [info, setInfo] = useState<BridgeInfo | null>(null);
  const [status, setStatus] = useState<unknown>(null);
  const [tier, setTier] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const bi = await invoke<BridgeInfo>("bridge_info");
      setInfo(bi);
      const st = await invoke("daemon_status");
      setStatus(st);
      const ti = await invoke("daemon_network_tier");
      setTier(ti);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <main>
      <h1>Source</h1>
      <p>Desktop shell with Tauri IPC → source-daemon JSON-RPC (Phase 1–2).</p>

      <div className="card">
        <div className="row">
          <span>Bridge protocol</span>
          <code>{info?.protocol_version ?? "—"}</code>
        </div>
        <div className="row">
          <span>Desktop</span>
          <code>{info?.desktop_version ?? "—"}</code>
        </div>
        <div className="row">
          <span>Daemon URL</span>
          <code>{info?.daemon_base_url ?? "—"}</code>
        </div>
        <button type="button" onClick={() => void refresh()}>
          Refresh via IPC
        </button>
        {error ? <p className="err">{error}</p> : null}
      </div>

      <div className="card">
        <strong>Network tier (via IPC)</strong>
        <pre>{JSON.stringify(tier, null, 2)}</pre>
      </div>

      <div className="card">
        <strong>Daemon status (via IPC)</strong>
        <pre>{JSON.stringify(status, null, 2)}</pre>
      </div>
    </main>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

createRoot(root).render(
  <StrictMode>
    <DesktopApp />
  </StrictMode>,
);