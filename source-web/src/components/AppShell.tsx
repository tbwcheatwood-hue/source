import { useDaemon } from "../hooks/useDaemon";
import { NetworkTier } from "../types/protocol";
import { NetworkTierBadge } from "./NetworkTierBadge";

const TIERS: NetworkTier[] = [
  "LOCAL_ONLY",
  "LOCAL_AND_LAN",
  "UNRESTRICTED_CONFIGURABLE",
];

export function AppShell() {
  const { conn, connDetail, status, tierInfo, helloTier, error, setTier } = useDaemon();
  const tier = tierInfo?.tier ?? status?.network_tier ?? helloTier ?? "LOCAL_ONLY";

  return (
    <div className="min-h-screen bg-mesh text-ink-50">
      <div className="pointer-events-none fixed inset-0 opacity-40 [background-image:linear-gradient(rgba(255,255,255,0.03)_1px,transparent_1px),linear-gradient(90deg,rgba(255,255,255,0.03)_1px,transparent_1px)] [background-size:48px_48px]" />
      <header className="relative z-10 border-b border-white/10 px-6 py-5 backdrop-blur-sm">
        <div className="mx-auto flex max-w-5xl items-end justify-between gap-4">
          <div>
            <p className="font-display text-4xl font-bold tracking-tight text-ink-50 md:text-5xl">
              Source
            </p>
            <p className="mt-1 max-w-xl font-sans text-sm text-ink-300">
              Local-first agent IDE shell — Phase 1–2 daemon bridge
            </p>
          </div>
          <NetworkTierBadge tier={tier} pulse={conn === "connecting"} />
        </div>
      </header>

      <main className="relative z-10 mx-auto grid max-w-5xl gap-6 px-6 py-10">
        <section className="grid gap-3">
          <h2 className="font-display text-2xl text-ink-100">Daemon connection</h2>
          <p className="text-sm text-ink-300">
            Live WebSocket + JSON-RPC to <span className="font-mono">source-daemon</span>. Full IDE
            surfaces arrive in Phase 5; this shell proves the protocol path.
          </p>
          <dl className="grid gap-2 font-mono text-sm md:grid-cols-2">
            <div className="rounded-lg bg-black/25 px-4 py-3 ring-1 ring-white/10">
              <dt className="text-ink-400">WebSocket</dt>
              <dd className="mt-1 text-ink-100">
                {conn}
                {connDetail ? ` — ${connDetail}` : ""}
              </dd>
            </div>
            <div className="rounded-lg bg-black/25 px-4 py-3 ring-1 ring-white/10">
              <dt className="text-ink-400">Protocol</dt>
              <dd className="mt-1 text-ink-100">
                {status?.protocol_version ?? "—"} / daemon {status?.version ?? "—"}
              </dd>
            </div>
            <div className="rounded-lg bg-black/25 px-4 py-3 ring-1 ring-white/10">
              <dt className="text-ink-400">Uptime</dt>
              <dd className="mt-1 text-ink-100">{status ? `${status.uptime_secs}s` : "—"}</dd>
            </div>
            <div className="rounded-lg bg-black/25 px-4 py-3 ring-1 ring-white/10">
              <dt className="text-ink-400">Data root</dt>
              <dd className="mt-1 truncate text-ink-100">{status?.data_root ?? "—"}</dd>
            </div>
          </dl>
          {error ? (
            <p className="rounded-md bg-red-500/10 px-3 py-2 text-sm text-red-200 ring-1 ring-red-500/30">
              {error}
            </p>
          ) : null}
        </section>

        <section className="grid gap-3">
          <h2 className="font-display text-2xl text-ink-100">Network policy</h2>
          <p className="text-sm text-ink-300">
            Three-tier engine enforced in the daemon HTTP/WS client and tool gate — not prompts.
          </p>
          <div className="flex flex-wrap gap-2">
            {TIERS.map((t) => (
              <button
                key={t}
                type="button"
                onClick={() => void setTier(t)}
                className={`rounded-md px-3 py-2 text-sm ring-1 transition ${
                  tier === t
                    ? "bg-ember-500/20 text-ember-400 ring-ember-500/50"
                    : "bg-white/5 text-ink-200 ring-white/10 hover:bg-white/10"
                }`}
              >
                {t}
              </button>
            ))}
          </div>
          {tierInfo ? (
            <pre className="overflow-auto rounded-lg bg-black/30 p-4 font-mono text-xs text-ink-200 ring-1 ring-white/10">
              {JSON.stringify(tierInfo, null, 2)}
            </pre>
          ) : null}
        </section>

        <section className="grid gap-3">
          <h2 className="font-display text-2xl text-ink-100">Providers</h2>
          <ul className="grid gap-2 md:grid-cols-2">
            {(status?.providers ?? []).map((p) => (
              <li
                key={p.id}
                className="rounded-lg bg-black/25 px-4 py-3 ring-1 ring-white/10"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="font-medium text-ink-100">{p.display_name}</span>
                  <span
                    className={`text-xs font-mono ${p.enabled ? "text-emerald-300" : "text-ink-400"}`}
                  >
                    {p.enabled ? "enabled" : "dormant"}
                  </span>
                </div>
                <p className="mt-1 truncate font-mono text-xs text-ink-400">{p.base_url}</p>
              </li>
            ))}
          </ul>
        </section>
      </main>
    </div>
  );
}