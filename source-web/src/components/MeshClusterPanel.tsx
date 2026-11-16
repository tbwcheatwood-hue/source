import { FormEvent, useCallback, useEffect, useState } from "react";
import type { SourceClient } from "../api/client";
import type {
  ClusterSnapshot,
  DiscoveredPeer,
  MeshPeerInfo,
  MeshStatus,
  PairingSession,
} from "../types/protocol";

export function MeshClusterPanel({ client }: { client: SourceClient }) {
  const [status, setStatus] = useState<MeshStatus | null>(null);
  const [cluster, setCluster] = useState<ClusterSnapshot | null>(null);
  const [peers, setPeers] = useState<MeshPeerInfo[]>([]);
  const [discovered, setDiscovered] = useState<DiscoveredPeer[]>([]);
  const [pairing, setPairing] = useState<PairingSession | null>(null);
  const [host, setHost] = useState("");
  const [pin, setPin] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [st, peerList, disc] = await Promise.all([
        client.meshStatus(),
        client.meshListPeers(),
        client.meshDiscover(),
      ]);
      setStatus(st);
      setPeers(peerList);
      setDiscovered(disc);
      if (st.mesh_allowed) {
        try {
          setCluster(await client.clusterResources());
        } catch {
          setCluster(null);
        }
      } else {
        setCluster(null);
      }
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client]);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => void refresh(), 5000);
    return () => window.clearInterval(id);
  }, [refresh]);

  const startMesh = async () => {
    setBusy(true);
    try {
      await client.meshStart({ advertise: true, browse: true });
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const stopMesh = async () => {
    setBusy(true);
    try {
      await client.meshStop();
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const beginPair = async () => {
    setBusy(true);
    try {
      const offer = await client.meshBeginPairing();
      setPairing(offer);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const register = async (e: FormEvent) => {
    e.preventDefault();
    if (!host.trim()) return;
    setBusy(true);
    try {
      await client.meshRegisterPeer(host.trim());
      setHost("");
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const completePair = async (e: FormEvent) => {
    e.preventDefault();
    if (!host.trim() || !pin.trim()) return;
    setBusy(true);
    try {
      await client.meshCompletePairing(host.trim(), pin.trim());
      setPin("");
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const local = cluster?.local;

  return (
    <div className="h-full min-h-0 overflow-auto p-4">
      <header className="mb-4">
        <h2 className="font-display text-2xl text-ink-50">Mesh & Cluster</h2>
        <p className="mt-1 text-sm text-ink-300">
          LAN / Tailscale discovery, pairing, and resource monitor via Phase 4 APIs.
        </p>
      </header>

      {error ? (
        <p className="mb-3 rounded-md bg-red-500/10 px-3 py-2 text-sm text-red-200 ring-1 ring-red-500/30">
          {error}
        </p>
      ) : null}

      <div className="mb-4 flex flex-wrap gap-2">
        <button type="button" className="ide-btn" disabled={busy} onClick={() => void startMesh()}>
          Start mesh
        </button>
        <button type="button" className="ide-btn" disabled={busy} onClick={() => void stopMesh()}>
          Stop
        </button>
        <button type="button" className="ide-btn" disabled={busy} onClick={() => void beginPair()}>
          Offer PIN pair
        </button>
        <button type="button" className="ide-btn" disabled={busy} onClick={() => void refresh()}>
          Refresh
        </button>
      </div>

      <section className="mb-6 grid gap-3 md:grid-cols-3">
        <Stat label="Node" value={status?.label ?? "—"} sub={status?.node_id?.slice(0, 12)} />
        <Stat
          label="Mesh"
          value={status?.enabled ? "enabled" : "off"}
          sub={status?.mesh_allowed ? "tier allows mesh" : "raise tier to LAN+"}
        />
        <Stat
          label="Peers"
          value={`${status?.paired_peers ?? 0} paired`}
          sub={`${status?.discovered_peers ?? 0} discovered`}
        />
      </section>

      {pairing ? (
        <div className="mb-4 rounded-md bg-ember-500/10 px-4 py-3 ring-1 ring-ember-500/30">
          <p className="font-mono text-xs text-ember-400">Pairing PIN (local only)</p>
          <p className="font-display text-3xl tracking-widest text-ink-50">{pairing.pin}</p>
          <p className="mt-1 font-mono text-xs text-ink-400">expires {pairing.expires_at}</p>
        </div>
      ) : null}

      {local ? (
        <section className="mb-6">
          <h3 className="mb-2 font-display text-xl text-ink-100">Local resources</h3>
          <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
            <Stat label="CPU" value={`${local.cpu_usage_pct.toFixed(0)}%`} sub={`${local.cpu_cores} cores`} />
            <Stat
              label="RAM"
              value={`${Math.round((local.ram_total_mb - local.ram_available_mb))} MB used`}
              sub={`${local.ram_total_mb} MB total`}
            />
            <Stat
              label="GPUs"
              value={String(local.gpus.length)}
              sub={local.gpus[0]?.name ?? "none detected"}
            />
            <Stat
              label="Models"
              value={String(local.loaded_models.length)}
              sub={local.loaded_models[0]?.name ?? "—"}
            />
          </div>
        </section>
      ) : null}

      <div className="mb-6 grid gap-6 lg:grid-cols-2">
        <section>
          <h3 className="mb-2 font-display text-xl text-ink-100">Register peer</h3>
          <form onSubmit={(e) => void register(e)} className="flex gap-2">
            <input
              value={host}
              onChange={(e) => setHost(e.target.value)}
              placeholder="host / MagicDNS"
              className="ide-input flex-1"
            />
            <button type="submit" className="ide-btn" disabled={busy}>
              Add
            </button>
          </form>
          <form onSubmit={(e) => void completePair(e)} className="mt-2 flex gap-2">
            <input
              value={pin}
              onChange={(e) => setPin(e.target.value)}
              placeholder="PIN"
              className="ide-input w-28"
            />
            <button type="submit" className="ide-btn" disabled={busy}>
              Complete pair
            </button>
          </form>
        </section>

        <section>
          <h3 className="mb-2 font-display text-xl text-ink-100">Discovered</h3>
          <ul className="space-y-1">
            {discovered.map((d) => (
              <li key={`${d.host}:${d.port}`} className="font-mono text-xs text-ink-300">
                {d.host}:{d.port} {d.peer_id ? `(${d.peer_id.slice(0, 8)})` : ""}
              </li>
            ))}
            {!discovered.length ? (
              <li className="text-sm text-ink-500">No mDNS peers yet</li>
            ) : null}
          </ul>
        </section>
      </div>

      <section className="mb-6">
        <h3 className="mb-2 font-display text-xl text-ink-100">Peers</h3>
        <ul className="space-y-2">
          {peers.map((p) => (
            <li
              key={p.peer_id}
              className="flex items-center justify-between gap-3 rounded-md bg-black/25 px-3 py-2 ring-1 ring-white/10"
            >
              <div>
                <p className="text-sm text-ink-100">{p.label || p.peer_id.slice(0, 12)}</p>
                <p className="font-mono text-[10px] text-ink-500">
                  {p.state} · {p.source} · {p.address}:{p.port}
                </p>
              </div>
              <button
                type="button"
                className="ide-btn"
                onClick={() => void client.meshTrustPeer(p.peer_id, !p.trusted).then(refresh)}
              >
                {p.trusted ? "Revoke" : "Trust"}
              </button>
            </li>
          ))}
          {!peers.length ? <li className="text-sm text-ink-500">No registered peers</li> : null}
        </ul>
      </section>

      {cluster?.active_tasks?.length ? (
        <section>
          <h3 className="mb-2 font-display text-xl text-ink-100">Active tasks</h3>
          <ul className="space-y-1 font-mono text-xs text-ink-300">
            {cluster.active_tasks.map((t) => (
              <li key={t.task_id}>
                {t.label} · {t.state} · {t.kind}
              </li>
            ))}
          </ul>
        </section>
      ) : null}
    </div>
  );
}

function Stat({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="rounded-md bg-black/25 px-3 py-3 ring-1 ring-white/10">
      <p className="font-mono text-[10px] uppercase tracking-wider text-ink-500">{label}</p>
      <p className="mt-1 text-sm text-ink-100">{value}</p>
      {sub ? <p className="mt-0.5 truncate font-mono text-[10px] text-ink-500">{sub}</p> : null}
    </div>
  );
}
