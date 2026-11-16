import { useCallback, useEffect, useRef, useState } from "react";
import type { SourceClient } from "../api/client";

export const IDE_LAYOUT_KV_KEY = "ide:layout:panes";
const LOCAL_KEY = "source.ide.layout.v1";

export const LAYOUT_DEFAULTS = {
  leftWidth: 52,
  rightWidth: 360,
  terminalHeight: 208,
  leftCollapsed: false,
  rightCollapsed: false,
  termOpen: true,
} as const;

export const LAYOUT_LIMITS = {
  leftMin: 48,
  leftMax: 280,
  rightMin: 260,
  rightMax: 640,
  termMin: 120,
  termMax: 480,
} as const;

export type IdeLayoutState = {
  leftWidth: number;
  rightWidth: number;
  terminalHeight: number;
  leftCollapsed: boolean;
  rightCollapsed: boolean;
  termOpen: boolean;
};

function clamp(n: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, n));
}

function normalize(raw: Partial<IdeLayoutState> | null | undefined): IdeLayoutState {
  const base = { ...LAYOUT_DEFAULTS, ...(raw ?? {}) };
  return {
    leftWidth: clamp(Number(base.leftWidth) || LAYOUT_DEFAULTS.leftWidth, LAYOUT_LIMITS.leftMin, LAYOUT_LIMITS.leftMax),
    rightWidth: clamp(
      Number(base.rightWidth) || LAYOUT_DEFAULTS.rightWidth,
      LAYOUT_LIMITS.rightMin,
      LAYOUT_LIMITS.rightMax,
    ),
    terminalHeight: clamp(
      Number(base.terminalHeight) || LAYOUT_DEFAULTS.terminalHeight,
      LAYOUT_LIMITS.termMin,
      LAYOUT_LIMITS.termMax,
    ),
    leftCollapsed: Boolean(base.leftCollapsed),
    rightCollapsed: Boolean(base.rightCollapsed),
    termOpen: base.termOpen !== false,
  };
}

function readLocal(): IdeLayoutState {
  try {
    const raw = localStorage.getItem(LOCAL_KEY);
    if (!raw) return { ...LAYOUT_DEFAULTS };
    return normalize(JSON.parse(raw) as Partial<IdeLayoutState>);
  } catch {
    return { ...LAYOUT_DEFAULTS };
  }
}

function writeLocal(state: IdeLayoutState) {
  try {
    localStorage.setItem(LOCAL_KEY, JSON.stringify(state));
  } catch {
    /* ignore quota */
  }
}

export function useIdeLayout(client: SourceClient | null) {
  const [layout, setLayout] = useState<IdeLayoutState>(() => readLocal());
  const synced = useRef(false);
  const saveTimer = useRef<number | null>(null);

  useEffect(() => {
    if (!client || synced.current) return;
    let cancelled = false;
    void (async () => {
      try {
        const res = await client.stateGet(IDE_LAYOUT_KV_KEY);
        if (cancelled || res.value == null || typeof res.value !== "object") return;
        const next = normalize(res.value as Partial<IdeLayoutState>);
        setLayout(next);
        writeLocal(next);
        synced.current = true;
      } catch {
        /* daemon may be offline — localStorage still works */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [client]);

  const persist = useCallback(
    (next: IdeLayoutState) => {
      writeLocal(next);
      if (!client) return;
      if (saveTimer.current) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => {
        void client.stateSet(IDE_LAYOUT_KV_KEY, next).catch(() => undefined);
      }, 400);
    },
    [client],
  );

  const update = useCallback(
    (patch: Partial<IdeLayoutState> | ((prev: IdeLayoutState) => IdeLayoutState)) => {
      setLayout((prev) => {
        const next = normalize(typeof patch === "function" ? patch(prev) : { ...prev, ...patch });
        persist(next);
        return next;
      });
    },
    [persist],
  );

  const reset = useCallback(() => {
    const next = { ...LAYOUT_DEFAULTS };
    setLayout(next);
    persist(next);
  }, [persist]);

  const effectiveLeft = layout.leftCollapsed ? LAYOUT_LIMITS.leftMin : layout.leftWidth;
  const effectiveRight = layout.rightCollapsed ? 0 : layout.rightWidth;

  return {
    layout,
    update,
    reset,
    effectiveLeft,
    effectiveRight,
  };
}
