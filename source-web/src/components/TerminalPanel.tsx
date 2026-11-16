import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { useEffect, useRef, useState } from "react";
import type { SourceClient } from "../api/client";

export function TerminalPanel({
  client,
  cwd,
}: {
  client: SourceClient;
  cwd: string;
}) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const ptyIdRef = useRef<string | null>(null);
  const [status, setStatus] = useState("idle");

  useEffect(() => {
    if (!hostRef.current) return;
    const term = new Terminal({
      cursorBlink: true,
      fontFamily: '"IBM Plex Mono", ui-monospace, monospace',
      fontSize: 13,
      theme: {
        background: "#0f1613",
        foreground: "#e3ebe6",
        cursor: "#e8a35c",
        selectionBackground: "#3c5d4c88",
      },
      convertEol: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(hostRef.current);
    fit.fit();
    termRef.current = term;

    let closed = false;
    let pollTimer: number | undefined;
    let resizeObs: ResizeObserver | undefined;

    const boot = async () => {
      try {
        setStatus("opening");
        const cols = term.cols;
        const rows = term.rows;
        const opened = await client.ptyOpen({
          cwd: cwd || undefined,
          cols,
          rows,
        });
        if (closed) {
          await client.ptyClose(opened.pty_id);
          return;
        }
        ptyIdRef.current = opened.pty_id;
        setStatus(`pty ${opened.pty_id.slice(0, 8)}`);
        term.writeln("\x1b[90m# Source PTY connected\x1b[0m");

        term.onData((data) => {
          const id = ptyIdRef.current;
          if (!id) return;
          void client.ptyWrite(id, data).catch((e) => {
            term.writeln(`\r\n\x1b[31mwrite error: ${e}\x1b[0m`);
          });
        });

        const poll = async () => {
          const id = ptyIdRef.current;
          if (!id || closed) return;
          try {
            const chunk = await client.ptyRead(id, { max_bytes: 65536, wait_ms: 50 });
            const data = typeof chunk.data === "string" ? chunk.data : "";
            if (data) term.write(data);
            if (chunk.alive === false) {
              setStatus("exited");
              return;
            }
          } catch {
            /* transient */
          }
          pollTimer = window.setTimeout(() => void poll(), 80);
        };
        void poll();

        resizeObs = new ResizeObserver(() => {
          fit.fit();
          const id = ptyIdRef.current;
          if (id) void client.ptyResize(id, term.cols, term.rows);
        });
        resizeObs.observe(hostRef.current!);
      } catch (e) {
        setStatus("error");
        term.writeln(`\x1b[31m${e instanceof Error ? e.message : String(e)}\x1b[0m`);
      }
    };

    void boot();

    return () => {
      closed = true;
      if (pollTimer) window.clearTimeout(pollTimer);
      resizeObs?.disconnect();
      const id = ptyIdRef.current;
      ptyIdRef.current = null;
      if (id) void client.ptyClose(id);
      term.dispose();
      termRef.current = null;
    };
  }, [client, cwd]);

  return (
    <div className="flex h-full min-h-0 flex-col border-t border-white/10 bg-[#0f1613]">
      <div className="flex items-center justify-between px-3 py-1.5">
        <span className="font-mono text-[10px] uppercase tracking-wider text-ink-500">
          Terminal
        </span>
        <span className="font-mono text-[10px] text-ink-400">{status}</span>
      </div>
      <div ref={hostRef} className="min-h-0 flex-1 px-2 pb-2" />
    </div>
  );
}
