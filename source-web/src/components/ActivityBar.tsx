export type IdePanel =
  | "editor"
  | "mesh"
  | "audit"
  | "docs"
  | "tools"
  | "settings";

const ITEMS: { id: IdePanel; label: string; glyph: string }[] = [
  { id: "editor", label: "Editor", glyph: "⟨⟩" },
  { id: "mesh", label: "Mesh & Cluster", glyph: "◎" },
  { id: "audit", label: "Audit / Rollback", glyph: "◷" },
  { id: "docs", label: "Docs & Templates", glyph: "☰" },
  { id: "tools", label: "Tools / MCP", glyph: "⚙" },
  { id: "settings", label: "LLM Settings", glyph: "◧" },
];

export function ActivityBar({
  active,
  onSelect,
  width,
  collapsed,
  onToggleCollapse,
}: {
  active: IdePanel;
  onSelect: (panel: IdePanel) => void;
  width: number;
  collapsed?: boolean;
  onToggleCollapse?: () => void;
}) {
  const showLabels = !collapsed && width >= 120;

  return (
    <nav
      className="flex h-full shrink-0 flex-col border-r border-white/10 bg-black/25 py-3"
      style={{ width }}
      aria-label="IDE panels"
    >
      <div className={`mb-2 flex items-center px-2 ${showLabels ? "justify-between" : "justify-center"}`}>
        {showLabels ? (
          <span className="font-mono text-[10px] uppercase tracking-wider text-ink-500">Sidebar</span>
        ) : null}
        {onToggleCollapse ? (
          <button
            type="button"
            className="ide-btn px-1.5 py-1"
            title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
            onClick={onToggleCollapse}
          >
            {collapsed ? "›" : "‹"}
          </button>
        ) : null}
      </div>
      <div className={`flex flex-col gap-1 ${showLabels ? "px-2" : "items-center"}`}>
        {ITEMS.map((item) => {
          const selected = active === item.id;
          return (
            <button
              key={item.id}
              type="button"
              title={item.label}
              aria-label={item.label}
              aria-current={selected ? "page" : undefined}
              onClick={() => onSelect(item.id)}
              className={`flex items-center transition ${
                showLabels
                  ? `h-9 w-full gap-2 rounded-md px-2 text-left font-mono text-xs ${
                      selected
                        ? "bg-ember-500/20 text-ember-400 ring-1 ring-ember-500/40"
                        : "text-ink-400 hover:bg-white/5 hover:text-ink-100"
                    }`
                  : `h-10 w-10 justify-center rounded-md font-mono text-sm ${
                      selected
                        ? "bg-ember-500/20 text-ember-400 ring-1 ring-ember-500/40"
                        : "text-ink-400 hover:bg-white/5 hover:text-ink-100"
                    }`
              }`}
            >
              <span className="shrink-0">{item.glyph}</span>
              {showLabels ? <span className="truncate">{item.label}</span> : null}
            </button>
          );
        })}
      </div>
    </nav>
  );
}
