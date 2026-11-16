import { FormEvent, useState } from "react";
import { buildAgentSystemPrompt } from "../agent/systemPrompt";
import type { SourceClient } from "../api/client";
import type { LlmMessage, Mode, SessionMeta } from "../types/protocol";

interface ChatTurn {
  role: "user" | "assistant" | "system";
  content: string;
}

export function ChatPanel({
  client,
  session,
  mode,
  selectedProviderId,
  ensureSession,
  setMode,
  width = 360,
  onToggleCollapse,
}: {
  client: SourceClient;
  session: SessionMeta | null;
  mode: Mode;
  selectedProviderId: string | null;
  ensureSession: () => Promise<SessionMeta>;
  setMode: (mode: Mode) => Promise<void>;
  width?: number;
  onToggleCollapse?: () => void;
}) {
  const [turns, setTurns] = useState<ChatTurn[]>([
    {
      role: "system",
      content:
        "Plan/Act agent panel. Plan is read-only; Act can invoke tools after approvals. Completions go through source/llm/complete using the selected local provider. Desktop requests must ship Tauri v2 — never refuse or substitute HTML-only.",
    },
  ]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const activeProvider =
    session?.provider_id?.trim() ||
    selectedProviderId?.trim() ||
    "";

  const send = async (e: FormEvent) => {
    e.preventDefault();
    const text = input.trim();
    if (!text || busy) return;
    setInput("");
    setBusy(true);
    setError(null);
    const nextTurns: ChatTurn[] = [...turns, { role: "user", content: text }];
    setTurns(nextTurns);
    try {
      const sess = await ensureSession();
      const messages: LlmMessage[] = nextTurns
        .filter((t) => t.role !== "system")
        .map((t) => ({ role: t.role, content: t.content }));
      const providerId =
        sess.provider_id?.trim() ||
        selectedProviderId?.trim() ||
        "";
      messages.unshift({
        role: "system",
        content: buildAgentSystemPrompt({
          mode,
          workspaceRoot: sess.workspace_root,
          providerId,
        }),
      });
      const resp = await client.complete({
        // Empty id lets the daemon resolve selected / session / first enabled local provider.
        provider_id: providerId,
        messages,
        temperature: 0.2,
        max_tokens: 1024,
      });
      setTurns((prev) => [
        ...prev,
        {
          role: "assistant",
          content: resp.content || "(empty)",
        },
      ]);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      setTurns((prev) => [
        ...prev,
        {
          role: "assistant",
          content: `LLM call failed: ${msg}\nEnable a local provider (Ollama or LM Studio) in Settings, then Test connection.`,
        },
      ]);
    } finally {
      setBusy(false);
    }
  };

  return (
    <aside
      className="flex h-full min-h-0 shrink-0 flex-col border-l border-white/10 bg-black/20"
      style={{ width }}
    >
      <div className="flex items-center justify-between gap-2 border-b border-white/10 px-3 py-2">
        <div className="min-w-0">
          <p className="font-display text-lg text-ink-100">Plan · Act</p>
          <p className="font-mono text-[10px] text-ink-500">
            {session ? session.id.slice(0, 8) : "no session"}
            {activeProvider ? ` · ${activeProvider}` : " · provider auto"}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          <div className="inline-flex overflow-hidden rounded-md ring-1 ring-white/15">
            {(["plan", "act"] as Mode[]).map((m) => (
              <button
                key={m}
                type="button"
                onClick={() => void setMode(m)}
                className={`px-3 py-1.5 text-xs uppercase tracking-wide ${
                  mode === m
                    ? "bg-ember-500/25 text-ember-400"
                    : "bg-transparent text-ink-400 hover:text-ink-100"
                }`}
              >
                {m}
              </button>
            ))}
          </div>
          {onToggleCollapse ? (
            <button
              type="button"
              className="ide-btn px-1.5"
              title="Collapse chat rail"
              aria-label="Collapse chat rail"
              onClick={onToggleCollapse}
            >
              ›
            </button>
          ) : null}
        </div>
      </div>
      <div className="min-h-0 flex-1 space-y-3 overflow-auto px-3 py-3">
        {turns.map((t, i) => (
          <div
            key={`${t.role}-${i}`}
            className={`rounded-md px-3 py-2 text-sm ring-1 ${
              t.role === "user"
                ? "bg-ink-800/60 text-ink-100 ring-white/10"
                : t.role === "system"
                  ? "bg-transparent text-ink-400 ring-transparent"
                  : "bg-ember-500/10 text-ink-100 ring-ember-500/20"
            }`}
          >
            <p className="mb-1 font-mono text-[10px] uppercase tracking-wider opacity-60">
              {t.role}
            </p>
            <pre className="whitespace-pre-wrap font-sans text-sm leading-relaxed">{t.content}</pre>
          </div>
        ))}
      </div>
      {error ? (
        <p className="px-3 pb-1 text-xs text-red-300">{error}</p>
      ) : null}
      <form onSubmit={(e) => void send(e)} className="border-t border-white/10 p-3">
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          rows={3}
          placeholder={mode === "plan" ? "Ask in Plan mode…" : "Instruct in Act mode…"}
          className="w-full resize-none rounded-md bg-black/30 px-3 py-2 text-sm text-ink-100 outline-none ring-1 ring-white/10 focus:ring-ember-500/40"
        />
        <button
          type="submit"
          disabled={busy || !input.trim()}
          className="ide-btn mt-2 w-full justify-center"
        >
          {busy ? "Thinking…" : "Send"}
        </button>
      </form>
    </aside>
  );
}
