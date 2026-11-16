/**
 * Local Vite entry for source-desktop package tooling only.
 * Tauri loads the unified Source IDE from `source-web` (see tauri.conf.json).
 */
import { StrictMode, useCallback, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";

interface BridgeInfo {
  protocol_version: string;
  daemon_base_url: string;
  desktop_version: string;
}

function DesktopBridgeSmoke() {
  const [info, setInfo] = useState<BridgeInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setInfo(await invoke<BridgeInfo>("bridge_info"));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <main style={{ fontFamily: "IBM Plex Sans, sans-serif", padding: 24, color: "#e3ebe6" }}>
      <h1 style={{ fontFamily: "Fraunces, Georgia, serif" }}>Source</h1>
      <p>
        Tauri production UI is <code>source-web</code>. This page is only the package-local Vite
        entry for IPC smoke.
      </p>
      <pre>{JSON.stringify(info, null, 2)}</pre>
      {error ? <p style={{ color: "#f5a8a8" }}>{error}</p> : null}
      <button type="button" onClick={() => void refresh()}>
        Refresh bridge_info
      </button>
    </main>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

createRoot(root).render(
  <StrictMode>
    <DesktopBridgeSmoke />
  </StrictMode>,
);
