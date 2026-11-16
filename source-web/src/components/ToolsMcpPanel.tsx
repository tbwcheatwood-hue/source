import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import type { SourceClient } from "../api/client";
import type {
  ApprovalRecord,
  McpServerInfo,
  McpTransport,
  McpUpsertParams,
  NetworkTier,
  ToolDefinition,
} from "../types/protocol";

type Draft = {
  id: string;
  name: string;
  transport: McpTransport;
  commandText: string;
  url: string;
  offline_capable: boolean;
  requires_wan: boolean;
  enabled: boolean;
  envText: string;
};

const EMPTY_DRAFT: Draft = {
  id: "",
  name: "",
  transport: "stdio",
  commandText: "",
  url: "",
  offline_capable: true,
  requires_wan: false,
  enabled: true,
  envText: "",
};

function tierBlocksServer(server: McpServerInfo, tier: NetworkTier): boolean {
  if (tier === "UNRESTRICTED_CONFIGURABLE") return false;
  return server.requires_wan || !server.offline_capable;
}

function parseCommand(text: string): string[] {
  return text
    .trim()
    .split(/\s+/)
    .map((s) => s.trim())
    .filter(Boolean);
}

function parseEnv(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) continue;
    out[trimmed.slice(0, eq).trim()] = trimmed.slice(eq + 1).trim();
  }
  return out;
}

function envToText(env: Record<string, string> | undefined): string {
  if (!env) return "";
  return Object.entries(env)
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");
}

function serverToDraft(s: McpServerInfo): Draft {
  return {
    id: s.id,
    name: s.name,
    transport: (s.transport as McpTransport) || "stdio",
    commandText: (s.command || []).join(" "),
    url: s.url || "",
    offline_capable: s.offline_capable,
    requires_wan: s.requires_wan,
    enabled: s.enabled,
    envText: envToText(s.env),
  };
}

function draftToParams(d: Draft): McpUpsertParams {
  const base: McpUpsertParams = {
    id: d.id.trim(),
    name: d.name.trim(),
    transport: d.transport,
    offline_capable: d.offline_capable,
    requires_wan: d.requires_wan,
    enabled: d.enabled,
    env: parseEnv(d.envText),
  };
  if (d.transport === "stdio") {
    base.command = parseCommand(d.commandText);
    base.url = null;
  } else {
    base.url = d.url.trim();
    base.command = null;
  }
  return base;
}

export function ToolsMcpPanel({ client }: { client: SourceClient }) {
  const [tools, setTools] = useState<ToolDefinition[]>([]);
  const [servers, setServers] = useState<McpServerInfo[]>([]);
  const [approvals, setApprovals] = useState<ApprovalRecord[]>([]);
  const [tier, setTier] = useState<NetworkTier>("LOCAL_ONLY");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [showForm, setShowForm] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [t, m, a, tierRes] = await Promise.all([
        client.listTools(),
        client.mcpListAll(),
        client.listPendingApprovals(),
        client.getNetworkTier(),
      ]);
      setTools(t);
      setServers(m);
      setApprovals(a);
      setTier(tierRes.tier);
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
      await client.mcpReload();
      await refresh();
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

  const startCreate = () => {
    setEditingId(null);
    setDraft({
      ...EMPTY_DRAFT,
      id: `mcp-${Date.now().toString(36)}`,
    });
    setShowForm(true);
  };

  const startEdit = (s: McpServerInfo) => {
    setEditingId(s.id);
    setDraft(serverToDraft(s));
    setShowForm(true);
  };

  const saveDraft = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const params = draftToParams(draft);
      if (!params.id || !params.name) throw new Error("id and name are required");
      if (params.transport === "stdio" && (!params.command || params.command.length === 0)) {
        throw new Error("stdio servers need a command");
      }
      if (params.transport !== "stdio" && !params.url) {
        throw new Error("SSE / Streamable HTTP servers need a URL");
      }
      await client.mcpUpsert(params);
      setShowForm(false);
      setEditingId(null);
      setDraft(EMPTY_DRAFT);
      await refresh();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const toggleEnabled = async (s: McpServerInfo) => {
    setBusy(true);
    try {
      await client.mcpSetEnabled(s.id, !s.enabled);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const removeServer = async (s: McpServerInfo) => {
    if (!window.confirm(`Remove MCP server “${s.name}” (${s.id})?`)) return;
    setBusy(true);
    try {
      await client.mcpDelete(s.id);
      if (editingId === s.id) {
        setShowForm(false);
        setEditingId(null);
      }
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const sortedServers = useMemo(
    () => [...servers].sort((a, b) => a.name.localeCompare(b.name)),
    [servers],
  );

  return (
    <div className="h-full min-h-0 overflow-auto p-4">
      <header className="mb-4 flex flex-wrap items-end justify-between gap-2">
        <div>
          <h2 className="font-display text-2xl text-ink-50">Tools / MCP</h2>
          <p className="mt-1 text-sm text-ink-300">
            Built-in offline tools and MCP servers from ~/.source/extensions — add, edit, enable, or
            hot-reload.
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button type="button" className="ide-btn" disabled={busy} onClick={startCreate}>
            Add MCP server
          </button>
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

      {showForm ? (
        <section className="mb-6 rounded-md bg-black/30 px-4 py-3 ring-1 ring-white/10">
          <div className="mb-3 flex items-center justify-between gap-2">
            <h3 className="font-display text-xl text-ink-100">
              {editingId ? `Edit ${editingId}` : "New MCP server"}
            </h3>
            <button
              type="button"
              className="ide-btn"
              onClick={() => {
                setShowForm(false);
                setEditingId(null);
              }}
            >
              Cancel
            </button>
          </div>
          <form className="grid gap-3 md:grid-cols-2" onSubmit={(e) => void saveDraft(e)}>
            <label className="flex flex-col gap-1 font-mono text-[10px] text-ink-500">
              id
              <input
                className="ide-input"
                value={draft.id}
                disabled={Boolean(editingId)}
                onChange={(e) => setDraft((d) => ({ ...d, id: e.target.value }))}
                required
              />
            </label>
            <label className="flex flex-col gap-1 font-mono text-[10px] text-ink-500">
              name
              <input
                className="ide-input"
                value={draft.name}
                onChange={(e) => setDraft((d) => ({ ...d, name: e.target.value }))}
                required
              />
            </label>
            <label className="flex flex-col gap-1 font-mono text-[10px] text-ink-500">
              transport
              <select
                className="ide-input"
                value={draft.transport}
                onChange={(e) =>
                  setDraft((d) => ({
                    ...d,
                    transport: e.target.value as McpTransport,
                  }))
                }
              >
                <option value="stdio">stdio</option>
                <option value="sse">sse</option>
                <option value="streamable_http">streamable_http</option>
              </select>
            </label>
            {draft.transport === "stdio" ? (
              <label className="flex flex-col gap-1 font-mono text-[10px] text-ink-500 md:col-span-1">
                command (argv, space-separated)
                <input
                  className="ide-input"
                  value={draft.commandText}
                  onChange={(e) => setDraft((d) => ({ ...d, commandText: e.target.value }))}
                  placeholder="npx -y @modelcontextprotocol/server-filesystem /tmp"
                  required
                />
              </label>
            ) : (
              <label className="flex flex-col gap-1 font-mono text-[10px] text-ink-500">
                url
                <input
                  className="ide-input"
                  value={draft.url}
                  onChange={(e) => setDraft((d) => ({ ...d, url: e.target.value }))}
                  placeholder="http://127.0.0.1:8080/mcp"
                  required
                />
              </label>
            )}
            <label className="flex flex-col gap-1 font-mono text-[10px] text-ink-500 md:col-span-2">
              env (KEY=value per line)
              <textarea
                className="ide-input min-h-[72px] resize-y"
                value={draft.envText}
                onChange={(e) => setDraft((d) => ({ ...d, envText: e.target.value }))}
              />
            </label>
            <div className="flex flex-wrap gap-4 md:col-span-2">
              <label className="flex items-center gap-2 text-xs text-ink-300">
                <input
                  type="checkbox"
                  checked={draft.offline_capable}
                  onChange={(e) => setDraft((d) => ({ ...d, offline_capable: e.target.checked }))}
                />
                offline_capable
              </label>
              <label className="flex items-center gap-2 text-xs text-ink-300">
                <input
                  type="checkbox"
                  checked={draft.requires_wan}
                  onChange={(e) => setDraft((d) => ({ ...d, requires_wan: e.target.checked }))}
                />
                requires_wan
              </label>
              <label className="flex items-center gap-2 text-xs text-ink-300">
                <input
                  type="checkbox"
                  checked={draft.enabled}
                  onChange={(e) => setDraft((d) => ({ ...d, enabled: e.target.checked }))}
                />
                enabled
              </label>
            </div>
            <div className="md:col-span-2">
              <button type="submit" className="ide-btn" disabled={busy}>
                {editingId ? "Save changes" : "Create server"}
              </button>
            </div>
          </form>
        </section>
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
                  {t.requires_wan ? "requires_wan" : "offline_capable"}
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
        <p className="mb-3 font-mono text-[10px] text-ink-500">
          Network tier: {tier}. Servers filtered from tool use when blocked by tier still appear here
          for management.
        </p>
        <ul className="space-y-3">
          {sortedServers.map((s) => {
            const blocked = tierBlocksServer(s, tier);
            return (
              <li key={s.id} className="rounded-md bg-black/25 px-3 py-3 ring-1 ring-white/10">
                <div className="flex flex-wrap items-start justify-between gap-2">
                  <div className="min-w-0">
                    <p className="text-sm text-ink-100">{s.name}</p>
                    <p className="font-mono text-[10px] text-ink-500">
                      {s.id} · {s.transport} · {s.running ? "running" : "idle"} ·{" "}
                      {s.offline_capable ? "offline_capable" : "not offline"} ·{" "}
                      {s.requires_wan ? "requires_wan" : "lan/local ok"}
                      {blocked ? " · blocked by tier" : ""}
                    </p>
                    <p className="mt-0.5 truncate font-mono text-[10px] text-ink-600" title={s.path}>
                      {s.path}
                    </p>
                  </div>
                  <div className="flex flex-wrap items-center gap-1">
                    <span
                      className={`rounded px-2 py-0.5 font-mono text-[10px] ring-1 ${
                        s.enabled
                          ? "bg-emerald-500/10 text-emerald-200 ring-emerald-500/30"
                          : "bg-white/5 text-ink-400 ring-white/10"
                      }`}
                    >
                      {s.enabled ? "enabled" : "disabled"}
                    </span>
                    <span
                      className={`rounded px-2 py-0.5 font-mono text-[10px] ring-1 ${
                        s.running
                          ? "bg-emerald-500/10 text-emerald-200 ring-emerald-500/30"
                          : "bg-white/5 text-ink-400 ring-white/10"
                      }`}
                    >
                      {s.running ? "connected" : "offline"}
                    </span>
                    <button
                      type="button"
                      className="ide-btn"
                      disabled={busy}
                      onClick={() => void toggleEnabled(s)}
                    >
                      {s.enabled ? "Disable" : "Enable"}
                    </button>
                    <button
                      type="button"
                      className="ide-btn"
                      disabled={busy}
                      onClick={() => startEdit(s)}
                    >
                      Edit
                    </button>
                    <button
                      type="button"
                      className="ide-btn"
                      disabled={busy}
                      onClick={() => void removeServer(s)}
                    >
                      Remove
                    </button>
                  </div>
                </div>
                {s.tools?.length ? (
                  <ul className="mt-2 space-y-1 border-t border-white/5 pt-2">
                    {s.tools.map((tool) => (
                      <li key={tool.name} className="font-mono text-xs text-ink-300">
                        {tool.name}
                        <span className="text-ink-600">
                          {" "}
                          — {tool.description || "no description"} ·{" "}
                          {tool.requires_wan ? "requires_wan" : "offline_capable"}
                        </span>
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="mt-2 font-mono text-[10px] text-ink-600">No tools discovered yet</p>
                )}
              </li>
            );
          })}
          {!sortedServers.length ? (
            <li className="text-sm text-ink-500">
              No MCP servers yet — use Add MCP server, or drop mcp.json under ~/.source/extensions and
              Reload.
            </li>
          ) : null}
        </ul>
      </section>
    </div>
  );
}
