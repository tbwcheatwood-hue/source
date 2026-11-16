import { FormEvent, useCallback, useEffect, useState } from "react";
import type { SourceClient } from "../api/client";
import type { RagHit, RagStatus, TemplateInfo, VendorCacheStatus } from "../types/protocol";

export function DocsTemplatesPanel({
  client,
  workspaceRoot,
}: {
  client: SourceClient;
  workspaceRoot: string;
}) {
  const [templates, setTemplates] = useState<TemplateInfo[]>([]);
  const [vendor, setVendor] = useState<VendorCacheStatus | null>(null);
  const [rag, setRag] = useState<RagStatus | null>(null);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<RagHit[]>([]);
  const [scaffoldId, setScaffoldId] = useState("");
  const [scaffoldName, setScaffoldName] = useState("my-app");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [t, v, r] = await Promise.all([
        client.templateList(),
        client.cacheVendorStatus(),
        client.ragStatus(),
      ]);
      setTemplates(t);
      setVendor(v);
      setRag(r);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const indexDocs = async () => {
    setBusy(true);
    try {
      setRag(await client.ragIndex(true));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const runQuery = async (e: FormEvent) => {
    e.preventDefault();
    if (!query.trim()) return;
    setBusy(true);
    try {
      const res = await client.ragQuery(query.trim());
      setHits(res.hits ?? (Array.isArray(res) ? (res as RagHit[]) : []));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const scaffold = async (e: FormEvent) => {
    e.preventDefault();
    if (!scaffoldId) return;
    setBusy(true);
    try {
      const dest = `${workspaceRoot.replace(/\/$/, "")}/${scaffoldName || "app"}`;
      await client.templateScaffold(scaffoldId, dest, scaffoldName || undefined);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const fmtBytes = (n: number) => {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  };

  return (
    <div className="h-full min-h-0 overflow-auto p-4">
      <header className="mb-4">
        <h2 className="font-display text-2xl text-ink-50">Docs & Templates</h2>
        <p className="mt-1 text-sm text-ink-300">
          Offline RAG corpus, app factory templates, and vendor cache status.
        </p>
      </header>

      {error ? (
        <p className="mb-3 rounded-md bg-red-500/10 px-3 py-2 text-sm text-red-200 ring-1 ring-red-500/30">
          {error}
        </p>
      ) : null}

      <section className="mb-8">
        <div className="mb-2 flex items-center justify-between gap-2">
          <h3 className="font-display text-xl text-ink-100">Offline docs (RAG)</h3>
          <button type="button" className="ide-btn" disabled={busy} onClick={() => void indexDocs()}>
            Reindex
          </button>
        </div>
        <pre className="mb-3 overflow-auto rounded-md bg-black/30 p-3 font-mono text-xs text-ink-300 ring-1 ring-white/10">
          {JSON.stringify(rag ?? {}, null, 2)}
        </pre>
        <form onSubmit={(e) => void runQuery(e)} className="flex gap-2">
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search offline docs…"
            className="ide-input flex-1"
          />
          <button type="submit" className="ide-btn" disabled={busy}>
            Query
          </button>
        </form>
        <ul className="mt-3 space-y-2">
          {hits.map((h, i) => (
            <li key={i} className="rounded-md bg-black/25 px-3 py-2 ring-1 ring-white/10">
              <p className="font-mono text-xs text-ember-400">
                {h.path || h.title || `hit-${i}`}
                {typeof h.score === "number" ? ` · ${h.score.toFixed(3)}` : ""}
              </p>
              <p className="mt-1 text-sm text-ink-200">{h.snippet || JSON.stringify(h)}</p>
            </li>
          ))}
        </ul>
      </section>

      <section className="mb-8">
        <h3 className="mb-2 font-display text-xl text-ink-100">Templates</h3>
        <ul className="mb-3 space-y-2">
          {templates.map((t) => (
            <li
              key={t.id}
              className={`cursor-pointer rounded-md px-3 py-2 ring-1 ${
                scaffoldId === t.id
                  ? "bg-ember-500/15 ring-ember-500/40"
                  : "bg-black/25 ring-white/10"
              }`}
              onClick={() => setScaffoldId(t.id)}
            >
              <p className="text-sm text-ink-100">
                {t.title}{" "}
                <span className="font-mono text-[10px] text-ink-500">
                  {t.builtin ? "builtin" : "custom"}
                </span>
              </p>
              <p className="text-xs text-ink-400">{t.description}</p>
            </li>
          ))}
          {!templates.length ? (
            <li className="text-sm text-ink-500">No templates — run offline-init</li>
          ) : null}
        </ul>
        <form onSubmit={(e) => void scaffold(e)} className="flex flex-wrap gap-2">
          <input
            value={scaffoldName}
            onChange={(e) => setScaffoldName(e.target.value)}
            placeholder="project name"
            className="ide-input"
          />
          <button type="submit" className="ide-btn" disabled={busy || !scaffoldId}>
            Scaffold into workspace
          </button>
        </form>
      </section>

      <section>
        <h3 className="mb-2 font-display text-xl text-ink-100">Vendor cache</h3>
        {vendor ? (
          <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
            <CacheStat label="npm" bytes={vendor.npm_bytes} path={vendor.npm_dir} fmt={fmtBytes} />
            <CacheStat label="pnpm" bytes={vendor.pnpm_bytes} path={vendor.pnpm_dir} fmt={fmtBytes} />
            <CacheStat label="cargo" bytes={vendor.cargo_bytes} path={vendor.cargo_dir} fmt={fmtBytes} />
            <CacheStat label="wheels" bytes={vendor.wheels_bytes} path={vendor.wheels_dir} fmt={fmtBytes} />
          </div>
        ) : (
          <p className="text-sm text-ink-500">Loading…</p>
        )}
      </section>
    </div>
  );
}

function CacheStat({
  label,
  bytes,
  path,
  fmt,
}: {
  label: string;
  bytes: number;
  path: string;
  fmt: (n: number) => string;
}) {
  return (
    <div className="rounded-md bg-black/25 px-3 py-3 ring-1 ring-white/10">
      <p className="font-mono text-[10px] uppercase tracking-wider text-ink-500">{label}</p>
      <p className="mt-1 text-sm text-ink-100">{fmt(bytes)}</p>
      <p className="mt-0.5 truncate font-mono text-[10px] text-ink-600">{path}</p>
    </div>
  );
}
