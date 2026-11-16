import { useCallback, useEffect, useState } from "react";
import type { SourceClient } from "../api/client";
import type { AuditEvent, CheckpointMeta, SessionMeta } from "../types/protocol";

export function AuditTimeline({
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
  const [events, setEvents] = useState<AuditEvent[]>([]);
  const [checkpoints, setCheckpoints] = useState<CheckpointMeta[]>([]);
  const [label, setLabel] = useState("manual");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const sess = session ?? (await ensureSession());
      const [audit, cps] = await Promise.all([
        client.listAudit(sess.id, 80),
        client.listCheckpoints(sess.id),
      ]);
      setEvents(audit);
      setCheckpoints(cps);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client, session, ensureSession]);

  useEffect(() => {
    void refresh();
    const id = window.setInterval(() => void refresh(), 6000);
    return () => window.clearInterval(id);
  }, [refresh]);

  const createCp = async () => {
    setBusy(true);
    try {
      const sess = await ensureSession();
      await client.createCheckpoint(sess.id, workspaceRoot || sess.workspace_root, label || "checkpoint");
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const rollback = async (cp: CheckpointMeta) => {
    setBusy(true);
    try {
      const sess = await ensureSession();
      await client.rollback(sess.id, cp.id, workspaceRoot || sess.workspace_root);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="h-full min-h-0 overflow-auto p-4">
      <header className="mb-4">
        <h2 className="font-display text-2xl text-ink-50">Audit / Rollback</h2>
        <p className="mt-1 text-sm text-ink-300">
          Hash-chained audit log and shadow-git checkpoints from the daemon.
        </p>
      </header>

      {error ? (
        <p className="mb-3 rounded-md bg-red-500/10 px-3 py-2 text-sm text-red-200 ring-1 ring-red-500/30">
          {error}
        </p>
      ) : null}

      <div className="mb-6 flex flex-wrap items-end gap-2">
        <label className="grid gap-1 text-xs text-ink-400">
          Checkpoint label
          <input
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            className="ide-input"
          />
        </label>
        <button type="button" className="ide-btn" disabled={busy} onClick={() => void createCp()}>
          Create checkpoint
        </button>
        <button type="button" className="ide-btn" disabled={busy} onClick={() => void refresh()}>
          Refresh
        </button>
      </div>

      <section className="mb-8">
        <h3 className="mb-2 font-display text-xl text-ink-100">Checkpoints</h3>
        <ul className="space-y-2">
          {checkpoints.map((cp) => (
            <li
              key={cp.id}
              className="flex items-center justify-between gap-3 rounded-md bg-black/25 px-3 py-2 ring-1 ring-white/10"
            >
              <div>
                <p className="text-sm text-ink-100">
                  #{cp.step_index} {cp.label}
                </p>
                <p className="font-mono text-[10px] text-ink-500">
                  {cp.git_commit.slice(0, 10)} · {new Date(cp.created_at).toLocaleString()}
                </p>
              </div>
              <button
                type="button"
                className="ide-btn"
                disabled={busy}
                onClick={() => void rollback(cp)}
              >
                Rollback
              </button>
            </li>
          ))}
          {!checkpoints.length ? (
            <li className="text-sm text-ink-500">No checkpoints yet</li>
          ) : null}
        </ul>
      </section>

      <section>
        <h3 className="mb-2 font-display text-xl text-ink-100">Timeline</h3>
        <ol className="relative space-y-0 border-l border-white/15 pl-4">
          {events.map((ev) => (
            <li key={ev.id} className="relative pb-4">
              <span className="absolute -left-[1.15rem] top-1.5 h-2.5 w-2.5 rounded-full bg-ember-500/80 ring-2 ring-ink-950" />
              <p className="font-mono text-[10px] uppercase tracking-wider text-ink-500">
                {ev.kind} · {new Date(ev.created_at).toLocaleString()}
              </p>
              <p className="text-sm text-ink-100">{ev.summary}</p>
              <p className="mt-0.5 truncate font-mono text-[10px] text-ink-600">
                {ev.content_hash.slice(0, 16)}
              </p>
            </li>
          ))}
          {!events.length ? <li className="text-sm text-ink-500">No audit events</li> : null}
        </ol>
      </section>
    </div>
  );
}
