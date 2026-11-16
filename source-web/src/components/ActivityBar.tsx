export type IdePanel =
  | "editor"
  | "mesh"
  | "audit"
  | "docs"
  | "tools";

const ITEMS: { id: IdePanel; label: string; glyph: string }[] = [
  { id: "editor", label: "Editor", glyph: "⟨⟩" },
  { id: "mesh", label: "Mesh & Cluster", glyph: "◎" },
  { id: "audit", label: "Audit / Rollback", glyph: "◷" },
  { id: "docs", label: "Docs & Templates", glyph: "☰" },
  { id: "tools", label: "Tools / MCP", glyph: "⚙" },
];

export function ActivityBar({
  active,
  onSelect,
}: {
  active: IdePanel;
  onSelect: (panel: IdePanel) => void;
}) {
  return (
    <nav
      className="flex w-12 shrink-0 flex-col items-center gap-1 border-r border-white/10 bg-black/25 py-3"
      aria-label="IDE panels"
    >
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
            className={`flex h-10 w-10 items-center justify-center rounded-md font-mono text-sm transition ${
              selected
                ? "bg-ember-500/20 text-ember-400 ring-1 ring-ember-500/40"
                : "text-ink-400 hover:bg-white/5 hover:text-ink-100"
            }`}
          >
            {item.glyph}
          </button>
        );
      })}
    </nav>
  );
}
