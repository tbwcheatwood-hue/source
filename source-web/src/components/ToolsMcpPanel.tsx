import { useCallback, useEffect, useState } from "react";
import type { SourceClient } from "../api/client";
import type { ApprovalRecord, McpServerInfo, ToolDefinition } from "../types/protocol";

export function ToolsMcpPanel({ client }: { client: SourceClient }) {
  const [tools, setTools] = useState<ToolDefinition[]>([]);
  const [servers, setServers] = useState<McpServerInfo[]>([]);
  const [approvals, setApprovals] = useState<ApprovalRecord[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [t, m, a] = await Promise.all([
        client.listTools(),
        client.mcpList(),
        client.listPendingApprovals(),
      ]);
      setTools(t);
      setServers(m);
      setApprovals(a);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client]);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => void refresh(), 7000);
    return () => window.clearInterval(id);
  }, [refresh]);

  const reloadMcp = async () => {
    setBusy(true);
    try {
      setServers(await client.mcpReload());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const resolve = async (id: string, decision: "approved" | "denied") => {
    setBusy(true);
    try {
      await client.resolveApproval(id, decision);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="h-full min-h-0 overflow-auto p-4">
      <header className="mb-4 flex flex-wrap items-end justify-between gap-2">
        <div>
          <h2 className="font-display text-2xl text-ink-50">Tools / MCP</h2>
          <p className="mt-1 text-sm text-ink-300">
            Built-in offline tools and hot-reloaded MCP servers from ~/.source/extensions.
          </p>
        </div>
        <div className="flex gap-2">
          <button type="button" className="ide-btn" disabled={busy} onClick={() => void reloadMcp()}>
            Reload MCP
          </button>
          <button type="button" className="ide-btn" disabled={busy} onClick={() => void refresh()}>
            Refresh
          </button>
        </div>
      </header>

      {error ? (
        <p className="mb-3 rounded-md bg-red-500/10 px-3 py-2 text-sm text-red-200 ring-1 ring-red-500/30">
          {error}
        </p>
      ) : null}

      {approvals.length ? (
        <section className="mb-6">
          <h3 className="mb-2 font-display text-xl text-ink-100">Pending approvals</h3>
          <ul className="space-y-2">
            {approvals.map((a) => (
              <li
                key={a.id}
                className="flex items-center justify-between gap-3 rounded-md bg-ember-500/10 px-3 py-2 ring-1 ring-ember-500/30"
              >
                <div>
                  <p className="text-sm text-ink-100">{a.summary}</p>
                  <p className="font-mono text-[10px] text-ink-500">{a.category}</p>
                </div>
                <div className="flex gap-1">
                  <button
                    type="button"
                    className="ide-btn"
                    disabled={busy}
                    onClick={() => void resolve(a.id, "approved")}
                  >
                    Allow
                  </button>
                  <button
                    type="button"
                    className="ide-btn"
                    disabled={busy}
                    onClick={() => void resolve(a.id, "denied")}
                  >
                    Deny
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </section>
      ) : null}

      <section className="mb-8">
        <h3 className="mb-2 font-display text-xl text-ink-100">Built-in tools</h3>
        <ul className="grid gap-2 md:grid-cols-2">
          {tools.map((t) => (
            <li key={t.name} className="rounded-md bg-black/25 px-3 py-2 ring-1 ring-white/10">
              <div className="flex items-center justify-between gap-2">
                <p className="font-mono text-sm text-ink-100">{t.name}</p>
                <span
                  className={`font-mono text-[10px] ${
                    t.requires_wan ? "text-ember-400" : "text-emerald-300"
                  }`}
                >
                  {t.requires_wan ? "WAN" : "offline"}
                </span>
              </div>
              <p className="mt-1 text-xs text-ink-400">{t.description}</p>
              <p className="mt-1 font-mono text-[10px] text-ink-600">
                {(t.categories || []).join(", ") || "no categories"}
              </p>
            </li>
          ))}
          {!tools.length ? <li className="text-sm text-ink-500">No tools registered</li> : null}
        </ul>
      </section>

      <section>
        <h3 className="mb-2 font-display text-xl text-ink-100">MCP servers</h3>
        <ul className="space-y-3">
          {servers.map((s) => (
            <li key={s.id} className="rounded-md bg-black/25 px-3 py-3 ring-1 ring-white/10">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <p className="text-sm text-ink-100">{s.name}</p>
                  <p className="font-mono text-[10px] text-ink-500">
                    {s.id} · {s.transport} · {s.running ? "running" : "idle"} ·{" "}
                    {s.requires_wan ? "WAN" : "offline"}
                  </p>
                </div>
                <span
                  className={`rounded px-2 py-0.5 font-mono text-[10px] ring-1 ${
                    s.enabled
                      ? "bg-emerald-500/10 text-emerald-200 ring-emerald-500/30"
                      : "bg-white/5 text-ink-400 ring-white/10"
                  }`}
                >
                  {s.enabled ? "enabled" : "disabled"}
                </span>
              </div>
              {s.tools?.length ? (
                <ul className="mt-2 space-y-1 border-t border-white/5 pt-2">
                  {s.tools.map((tool) => (
                    <li key={tool.name} className="font-mono text-xs text-ink-300">
                      {tool.name}
                      <span className="text-ink-600"> — {tool.description}</span>
                    </li>
                  ))}
                </ul>
              ) : null}
            </li>
          ))}
          {!servers.length ? (
            <li className="text-sm text-ink-500">
              No MCP manifests under ~/.source/extensions — drop mcp.json files and reload.
            </li>
          ) : null}
        </ul>
      </section>
    </div>
  );
}
