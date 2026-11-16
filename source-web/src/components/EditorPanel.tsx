import Editor, { DiffEditor } from "@monaco-editor/react";
import { useCallback, useEffect, useState } from "react";
import "../lib/monaco";
import type { SourceClient } from "../api/client";
import type { SessionMeta } from "../types/protocol";

function languageForPath(path: string): string {
  const lower = path.toLowerCase();
  if (lower.endsWith(".ts") || lower.endsWith(".tsx")) return "typescript";
  if (lower.endsWith(".js") || lower.endsWith(".jsx")) return "javascript";
  if (lower.endsWith(".rs")) return "rust";
  if (lower.endsWith(".py")) return "python";
  if (lower.endsWith(".go")) return "go";
  if (lower.endsWith(".json")) return "json";
  if (lower.endsWith(".md")) return "markdown";
  if (lower.endsWith(".toml")) return "ini";
  if (lower.endsWith(".css")) return "css";
  if (lower.endsWith(".html")) return "html";
  if (lower.endsWith(".yml") || lower.endsWith(".yaml")) return "yaml";
  if (lower.endsWith(".sh")) return "shell";
  return "plaintext";
}

export function EditorPanel({
  client,
  session,
  workspaceRoot,
  ensureSession,
}: {
  client: SourceClient;
  session: SessionMeta | null;
  workspaceRoot: string;
  ensureSession: () => Promise<SessionMeta>;
}) {
  const [path, setPath] = useState("");
  const [entries, setEntries] = useState<string[]>([]);
  const [content, setContent] = useState("// Open a file from the workspace list");
  const [original, setOriginal] = useState<string | null>(null);
  const [diffMode, setDiffMode] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refreshList = useCallback(async () => {
    setBusy(true);
    setMsg(null);
    try {
      const sess = session ?? (await ensureSession());
      const result = await client.fsOp(sess.id, "list", workspaceRoot || ".");
      if (!result.ok) throw new Error(result.error || result.output);
      const lines = result.output
        .split("\n")
        .map((l) => l.trim())
        .filter(Boolean)
        .map((l) => {
          const tab = l.indexOf("\t");
          return tab >= 0 ? l.slice(tab + 1) : l;
        });
      setEntries(lines);
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [client, session, workspaceRoot, ensureSession]);

  useEffect(() => {
    void refreshList();
  }, [refreshList]);

  const openFile = async (rel: string) => {
    setBusy(true);
    setMsg(null);
    try {
      const sess = session ?? (await ensureSession());
      const full = rel.startsWith("/") || rel.includes(":") ? rel : `${workspaceRoot.replace(/\/$/, "")}/${rel}`;
      const result = await client.fsOp(sess.id, "read", full);
      if (!result.ok) throw new Error(result.error || result.output);
      setPath(full);
      setContent(result.output);
      setOriginal(result.output);
      setDirty(false);
      setDiffMode(false);
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const saveFile = async () => {
    if (!path) return;
    setBusy(true);
    setMsg(null);
    try {
      const sess = await ensureSession();
      // Switch to act if needed for writes — invoke may require FileEdits approval.
      await client.setMode(sess.id, "act");
      const result = await client.fsOp(sess.id, "write", path, { content });
      if (!result.ok) throw new Error(result.error || result.output);
      setOriginal(content);
      setDirty(false);
      setMsg("Saved");
    } catch (e) {
      setMsg(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-white/10 px-3 py-2">
        <button
          type="button"
          className="ide-btn"
          onClick={() => void refreshList()}
          disabled={busy}
        >
          Refresh
        </button>
        <button
          type="button"
          className="ide-btn"
          onClick={() => void saveFile()}
          disabled={!path || !dirty || busy}
        >
          Save
        </button>
        <button
          type="button"
          className="ide-btn"
          onClick={() => setDiffMode((v) => !v)}
          disabled={!original}
        >
          {diffMode ? "Edit" : "Diff"}
        </button>
        <span className="truncate font-mono text-xs text-ink-400">{path || "no file"}</span>
        {dirty ? <span className="text-xs text-ember-400">modified</span> : null}
        {msg ? <span className="text-xs text-ink-300">{msg}</span> : null}
      </div>
      <div className="flex min-h-0 flex-1">
        <aside className="w-52 shrink-0 overflow-auto border-r border-white/10 bg-black/20 p-2">
          <p className="mb-2 px-1 font-mono text-[10px] uppercase tracking-wider text-ink-500">
            Workspace
          </p>
          <ul className="space-y-0.5">
            {entries.map((e) => (
              <li key={e}>
                <button
                  type="button"
                  className="w-full truncate rounded px-1.5 py-1 text-left font-mono text-xs text-ink-200 hover:bg-white/10"
                  onClick={() => void openFile(e)}
                >
                  {e}
                </button>
              </li>
            ))}
            {!entries.length ? (
              <li className="px-1.5 text-xs text-ink-500">Empty or unreachable</li>
            ) : null}
          </ul>
        </aside>
        <div className="min-w-0 flex-1">
          {diffMode && original !== null ? (
            <DiffEditor
              height="100%"
              theme="vs-dark"
              language={languageForPath(path)}
              original={original}
              modified={content}
              options={{
                readOnly: true,
                renderSideBySide: true,
                minimap: { enabled: false },
                fontFamily: '"IBM Plex Mono", ui-monospace, monospace',
                fontSize: 13,
              }}
            />
          ) : (
            <Editor
              height="100%"
              theme="vs-dark"
              language={languageForPath(path)}
              value={content}
              onChange={(v) => {
                setContent(v ?? "");
                setDirty(true);
              }}
              options={{
                minimap: { enabled: false },
                fontFamily: '"IBM Plex Mono", ui-monospace, monospace',
                fontSize: 13,
                automaticLayout: true,
                scrollBeyondLastLine: false,
              }}
            />
          )}
        </div>
      </div>
    </div>
  );
}
