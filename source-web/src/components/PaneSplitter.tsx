import { useCallback, useEffect, useRef } from "react";

type Axis = "horizontal" | "vertical";

/**
 * Drag handle between IDE panes. Double-click resets via onReset.
 * Horizontal = left|right resize (changes width along X).
 * Vertical = top|bottom resize (changes height along Y).
 */
export function PaneSplitter({
  axis,
  onDrag,
  onReset,
  disabled,
  label,
}: {
  axis: Axis;
  onDrag: (deltaPx: number) => void;
  onReset?: () => void;
  disabled?: boolean;
  label: string;
}) {
  const dragging = useRef(false);
  const last = useRef(0);

  const onPointerMove = useCallback(
    (e: PointerEvent) => {
      if (!dragging.current) return;
      const pos = axis === "horizontal" ? e.clientX : e.clientY;
      const delta = pos - last.current;
      last.current = pos;
      if (delta !== 0) onDrag(delta);
    },
    [axis, onDrag],
  );

  const endDrag = useCallback(() => {
    if (!dragging.current) return;
    dragging.current = false;
    document.body.classList.remove("ide-resizing-x", "ide-resizing-y");
    window.removeEventListener("pointermove", onPointerMove);
    window.removeEventListener("pointerup", endDrag);
  }, [onPointerMove]);

  useEffect(() => () => endDrag(), [endDrag]);

  if (disabled) return null;

  return (
    <div
      role="separator"
      aria-orientation={axis === "horizontal" ? "vertical" : "horizontal"}
      aria-label={label}
      title={`${label} — drag to resize, double-click to reset`}
      tabIndex={0}
      className={
        axis === "horizontal"
          ? "group relative z-30 w-1.5 shrink-0 cursor-col-resize bg-transparent hover:bg-ember-500/25 active:bg-ember-500/40"
          : "group relative z-30 h-1.5 shrink-0 cursor-row-resize bg-transparent hover:bg-ember-500/25 active:bg-ember-500/40"
      }
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        dragging.current = true;
        last.current = axis === "horizontal" ? e.clientX : e.clientY;
        document.body.classList.add(axis === "horizontal" ? "ide-resizing-x" : "ide-resizing-y");
        window.addEventListener("pointermove", onPointerMove);
        window.addEventListener("pointerup", endDrag);
      }}
      onDoubleClick={() => onReset?.()}
      onKeyDown={(e) => {
        const step = e.shiftKey ? 24 : 8;
        if (axis === "horizontal") {
          if (e.key === "ArrowLeft") {
            e.preventDefault();
            onDrag(-step);
          } else if (e.key === "ArrowRight") {
            e.preventDefault();
            onDrag(step);
          }
        } else if (e.key === "ArrowUp") {
          e.preventDefault();
          onDrag(-step);
        } else if (e.key === "ArrowDown") {
          e.preventDefault();
          onDrag(step);
        } else if (e.key === "Home" || e.key === "Enter") {
          e.preventDefault();
          onReset?.();
        }
      }}
    >
      <span
        className={
          axis === "horizontal"
            ? "pointer-events-none absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-white/10 group-hover:bg-ember-500/50"
            : "pointer-events-none absolute inset-x-0 top-1/2 h-px -translate-y-1/2 bg-white/10 group-hover:bg-ember-500/50"
        }
      />
    </div>
  );
}
