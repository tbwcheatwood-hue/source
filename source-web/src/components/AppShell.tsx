import { useState } from "react";
import { useDaemon } from "../hooks/useDaemon";
import { LAYOUT_DEFAULTS, LAYOUT_LIMITS, useIdeLayout } from "../hooks/useIdeLayout";
import { ActivityBar, IdePanel } from "./ActivityBar";
import { AuditTimeline } from "./AuditTimeline";
import { ChatPanel } from "./ChatPanel";
import { DocsTemplatesPanel } from "./DocsTemplatesPanel";
import { EditorPanel } from "./EditorPanel";
import { MeshClusterPanel } from "./MeshClusterPanel";
import { NetworkTierSwitch } from "./NetworkTierSwitch";
import { PaneSplitter } from "./PaneSplitter";
import { TerminalPanel } from "./TerminalPanel";
import { SettingsProvidersPanel } from "./SettingsProvidersPanel";
import { ToolsMcpPanel } from "./ToolsMcpPanel";

export function AppShell() {
  const daemon = useDaemon();
  const {
    client,
    conn,
    connDetail,
    status,
    tierInfo,
    helloTier,
    error,
    session,
    setSession,
    selectedProviderId,
    setSelectedProviderId,
    workspaceRoot,
    setWorkspaceRoot,
    ensureSession,
    setTier,
    setMode,
  } = daemon;

  const [panel, setPanel] = useState<IdePanel>("editor");
  const { layout, update, reset, effectiveLeft, effectiveRight } = useIdeLayout(client);

  const tier = tierInfo?.tier ?? status?.network_tier ?? helloTier ?? "LOCAL_ONLY";
  const mode = session?.mode ?? status?.mode ?? "plan";

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-mesh text-ink-50">
      <div className="pointer-events-none fixed inset-0 opacity-30 [background-image:linear-gradient(rgba(255,255,255,0.03)_1px,transparent_1px),linear-gradient(90deg,rgba(255,255,255,0.03)_1px,transparent_1px)] [background-size:40px_40px]" />

      <header className="relative z-20 flex shrink-0 items-center gap-4 border-b border-white/10 bg-black/30 px-4 py-2.5 backdrop-blur-md">
        <div className="min-w-0">
          <h1 className="font-display text-2xl font-bold leading-none tracking-tight text-ink-50 md:text-3xl">
            Source
          </h1>
          <p className="mt-0.5 truncate font-mono text-[10px] text-ink-500">
            {status
              ? `v${status.version} · protocol ${status.protocol_version} · ${client.transport}`
              : "connecting…"}
          </p>
        </div>

        <div className="ml-auto flex flex-wrap items-center justify-end gap-3">
          <button
            type="button"
            className="ide-btn"
            title="Reset pane sizes to defaults"
            onClick={() => reset()}
          >
            Reset layout
          </button>
          {layout.rightCollapsed ? (
            <button
              type="button"
              className="ide-btn"
              onClick={() => update({ rightCollapsed: false })}
            >
              Show chat
            </button>
          ) : null}
          <NetworkTierSwitch
            tier={tier}
            pulse={conn === "connecting"}
            onChange={(t) => void setTier(t)}
          />
          <span
            className={`rounded-md px-2 py-1 font-mono text-[10px] ring-1 ${
              conn === "open"
                ? "bg-emerald-500/10 text-emerald-200 ring-emerald-500/30"
                : "bg-white/5 text-ink-400 ring-white/10"
            }`}
            title={connDetail}
          >
            {conn}
          </span>
        </div>
      </header>

      <div className="relative z-10 flex min-h-0 flex-1">
        <ActivityBar
          active={panel}
          onSelect={setPanel}
          width={effectiveLeft}
          collapsed={layout.leftCollapsed}
          onToggleCollapse={() =>
            update((prev) => ({
              ...prev,
              leftCollapsed: !prev.leftCollapsed,
              leftWidth: prev.leftCollapsed
                ? Math.max(prev.leftWidth, LAYOUT_DEFAULTS.leftWidth)
                : prev.leftWidth,
            }))
          }
        />

        <PaneSplitter
          axis="horizontal"
          label="Resize left sidebar"
          onDrag={(delta) =>
            update((prev) => {
              if (prev.leftCollapsed && delta > 0) {
                return {
                  ...prev,
                  leftCollapsed: false,
                  leftWidth: LAYOUT_LIMITS.leftMin + delta,
                };
              }
              const next = prev.leftWidth + delta;
              if (next < LAYOUT_LIMITS.leftMin + 8) {
                return { ...prev, leftCollapsed: true, leftWidth: LAYOUT_LIMITS.leftMin };
              }
              return {
                ...prev,
                leftCollapsed: false,
                leftWidth: Math.min(LAYOUT_LIMITS.leftMax, Math.max(LAYOUT_LIMITS.leftMin, next)),
              };
            })
          }
          onReset={() =>
            update({
              leftWidth: LAYOUT_DEFAULTS.leftWidth,
              leftCollapsed: false,
            })
          }
        />

        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex items-center gap-2 border-b border-white/10 bg-black/15 px-3 py-1.5">
            <label className="flex min-w-0 flex-1 items-center gap-2 font-mono text-[10px] text-ink-500">
              workspace
              <input
                value={workspaceRoot}
                onChange={(e) => setWorkspaceRoot(e.target.value)}
                className="ide-input min-w-0 flex-1 py-1"
              />
            </label>
            <button
              type="button"
              className="ide-btn"
              onClick={() => update({ termOpen: !layout.termOpen })}
              title="Toggle terminal"
            >
              {layout.termOpen ? "Hide term" : "Show term"}
            </button>
          </div>

          <div className="min-h-0 flex-1">
            {panel === "editor" ? (
              <EditorPanel
                client={client}
                session={session}
                workspaceRoot={workspaceRoot}
                ensureSession={ensureSession}
              />
            ) : null}
            {panel === "mesh" ? <MeshClusterPanel client={client} /> : null}
            {panel === "audit" ? (
              <AuditTimeline
                client={client}
                session={session}
                workspaceRoot={workspaceRoot}
                ensureSession={ensureSession}
              />
            ) : null}
            {panel === "docs" ? (
              <DocsTemplatesPanel client={client} workspaceRoot={workspaceRoot} />
            ) : null}
            {panel === "tools" ? <ToolsMcpPanel client={client} /> : null}
            {panel === "settings" ? (
              <SettingsProvidersPanel
                client={client}
                tier={tier}
                session={session}
                selectedProviderId={selectedProviderId}
                onSelectedProviderChange={setSelectedProviderId}
                onSessionChange={setSession}
              />
            ) : null}
          </div>

          {layout.termOpen ? (
            <>
              <PaneSplitter
                axis="vertical"
                label="Resize terminal"
                onDrag={(delta) =>
                  update((prev) => ({
                    ...prev,
                    terminalHeight: Math.min(
                      LAYOUT_LIMITS.termMax,
                      Math.max(LAYOUT_LIMITS.termMin, prev.terminalHeight - delta),
                    ),
                  }))
                }
                onReset={() => update({ terminalHeight: LAYOUT_DEFAULTS.terminalHeight })}
              />
              <div className="shrink-0" style={{ height: layout.terminalHeight }}>
                <TerminalPanel client={client} cwd={workspaceRoot} />
              </div>
            </>
          ) : null}
        </div>

        {!layout.rightCollapsed ? (
          <>
            <PaneSplitter
              axis="horizontal"
              label="Resize Plan · Act panel"
              onDrag={(delta) =>
                update((prev) => {
                  const next = prev.rightWidth - delta;
                  if (next < LAYOUT_LIMITS.rightMin - 20) {
                    return { ...prev, rightCollapsed: true };
                  }
                  return {
                    ...prev,
                    rightWidth: Math.min(
                      LAYOUT_LIMITS.rightMax,
                      Math.max(LAYOUT_LIMITS.rightMin, next),
                    ),
                  };
                })
              }
              onReset={() =>
                update({
                  rightWidth: LAYOUT_DEFAULTS.rightWidth,
                  rightCollapsed: false,
                })
              }
            />
            <ChatPanel
              client={client}
              session={session}
              mode={mode}
              selectedProviderId={selectedProviderId}
              ensureSession={ensureSession}
              setMode={setMode}
              width={effectiveRight}
              onToggleCollapse={() => update({ rightCollapsed: true })}
            />
          </>
        ) : null}
      </div>

      {error ? (
        <div className="relative z-20 border-t border-red-500/30 bg-red-950/80 px-4 py-2 text-sm text-red-100">
          {error}
        </div>
      ) : null}
    </div>
  );
}
