import { useEffect, useMemo, useState } from "react";
import { ConnectionState, SourceClient } from "../api/client";
import { DaemonStatus, NetworkTier, NetworkTierResult } from "../types/protocol";

export function useDaemon() {
  const client = useMemo(() => new SourceClient(), []);
  const [conn, setConn] = useState<ConnectionState>("idle");
  const [connDetail, setConnDetail] = useState<string | undefined>();
  const [status, setStatus] = useState<DaemonStatus | null>(null);
  const [tierInfo, setTierInfo] = useState<NetworkTierResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [helloTier, setHelloTier] = useState<NetworkTier | null>(null);

  useEffect(() => {
    client.onState = (s, detail) => {
      setConn(s);
      setConnDetail(detail);
    };
    client.onTier = (t) => setHelloTier(t);
    client.connectWs();

    const refresh = async () => {
      try {
        const [s, t] = await Promise.all([client.status(), client.getNetworkTier()]);
        setStatus(s);
        setTierInfo(t);
        setError(null);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    };

    void refresh();
    const id = window.setInterval(() => void refresh(), 5000);
    return () => {
      window.clearInterval(id);
      client.disconnect();
    };
  }, [client]);

  const setTier = async (tier: NetworkTier) => {
    const t = await client.setNetworkTier(tier);
    setTierInfo(t);
    const s = await client.status();
    setStatus(s);
  };

  return {
    client,
    conn,
    connDetail,
    status,
    tierInfo,
    helloTier,
    error,
    setTier,
  };
}