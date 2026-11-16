import { useCallback, useEffect, useMemo, useState } from "react";
import type { SourceClient } from "../api/client";
import type {
  NetworkTier,
  ProviderConfig,
  ProviderProbeResult,
  SessionMeta,
} from "../types/protocol";

const OLLAMA_ID = "ollama-local";
const LMSTUDIO_ID = "lmstudio-local";

const DEFAULTS: Record<string, Partial<ProviderConfig>> = {
  [OLLAMA_ID]: {
    id: OLLAMA_ID,
    kind: "ollama",
    display_name: "Ollama (local)",
    base_url: "http://127.0.0.1:11434",
    model: "llama3.2",
    api_key_env: null,
    enabled: true,
    extra_headers: {},
  },
  [LMSTUDIO_ID]: {
    id: LMSTUDIO_ID,
    kind: "lm_studio",
    display_name: "LM Studio (local)",
    base_url: "http://127.0.0.1:1234/v1",
    model: "local-model",
    api_key_env: null,
    enabled: true,
    extra_headers: {},
  },
};

function tierImpact(tier: NetworkTier): { title: string; body: string } {
  switch (tier) {
    case "LOCAL_ONLY":
      return {
        title: "LOCAL_ONLY",
        body: "Loopback only (127.0.0.1 / ::1). Ollama and LM Studio on localhost are allowed; LAN and cloud endpoints are blocked.",
      };
    case "LOCAL_AND_LAN":
      return {
        title: "LOCAL_AND_LAN",
        body: "Loopback plus private LAN / Tailscale CGNAT. Local providers work; public cloud still needs UNRESTRICTED_CONFIGURABLE (and enable).",
      };
    case "UNRESTRICTED_CONFIGURABLE":
      return {
        title: "UNRESTRICTED_CONFIGURABLE",
        body: "Allowlist + approval gate for public hosts. Local providers remain available; cloud connectors can egress when enabled.",
      };
  }
}

function ProviderEditor({
  title,
  providerId,
  kind,
  client,
  tier,
  selectedId,
  session,
  onSelected,
  onSession,
}: {
  title: string;
  providerId: string;
  kind: string;
  client: SourceClient;
  tier: NetworkTier;
  selectedId: string | null;
  session: SessionMeta | null;
  onSelected: (id: string) => void;
  onSession: (s: SessionMeta) => void;
}) {
  const [config, setConfig] = useState<ProviderConfig>(
    () => (DEFAULTS[providerId] as ProviderConfig) ?? {
      id: providerId,
      kind,
      display_name: title,
      base_url: "",
      model: "",
      api_key_env: null,
      enabled: false,
      extra_headers: {},
    },
  );
  const [models, setModels] = useState<string[]>([]);
  const [probe, setProbe] = useState<ProviderProbeResult | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [savedFlash, setSavedFlash] = useState(false);

  const load = useCallback(async () => {
    try {
      const list = await client.listProviders();
      const found = list.find((p) => p.id === providerId);
      if (found) {
        setConfig({
          ...found,
          extra_headers: found.extra_headers ?? {},
        });
      } else if (DEFAULTS[providerId]) {
        setConfig(DEFAULTS[providerId] as ProviderConfig);
      }
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client, providerId]);

  useEffect(() => {
    void load();
  }, [load]);

  const patch = (partial: Partial<ProviderConfig>) => {
    setConfig((c) => ({ ...c, ...partial }));
  };

  const save = async () => {
    setBusy("save");
    setError(null);
    try {
      const saved = await client.upsertProvider({
        ...config,
        id: providerId,
        kind: config.kind || kind,
        extra_headers: config.extra_headers ?? {},
      });
      setConfig(saved);
      setSavedFlash(true);
      window.setTimeout(() => setSavedFlash(false), 1200);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const toggleEnabled = async () => {
    setBusy("enable");
    setError(null);
    try {
      // Persist current fields first so enable doesn't discard edits.
      await client.upsertProvider({
        ...config,
        id: providerId,
        kind: config.kind || kind,
        enabled: config.enabled,
        extra_headers: config.extra_headers ?? {},
      });
      if (config.enabled) {
        await client.disableProvider(providerId);
        patch({ enabled: false });
      } else {
        await client.enableProvider(providerId);
        patch({ enabled: true });
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const refreshModels = async () => {
    setBusy("models");
    setError(null);
    try {
      await client.upsertProvider({
        ...config,
        id: providerId,
        kind: config.kind || kind,
        extra_headers: config.extra_headers ?? {},
      });
      const result = await client.listModels(providerId);
      setModels(result.models);
      if (result.models.length && !result.models.includes(config.model)) {
        // Keep current model; user picks explicitly.
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const testConnection = async () => {
    setBusy("probe");
    setError(null);
    try {
      const result = await client.probeProvider({
        provider_id: providerId,
        kind: config.kind || kind,
        base_url: config.base_url,
        api_key_env: config.api_key_env,
      });
      setProbe(result);
      if (result.models.length) setModels(result.models);
      if (!result.ok && result.error) setError(result.error);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const selectForChat = async () => {
    setBusy("select");
    setError(null);
    try {
      await client.upsertProvider({
        ...config,
        id: providerId,
        kind: config.kind || kind,
        enabled: true,
        extra_headers: config.extra_headers ?? {},
      });
      if (!config.enabled) {
        await client.enableProvider(providerId);
        patch({ enabled: true });
      }
      await client.setSelectedProvider(providerId);
      onSelected(providerId);
      if (session) {
        const updated = await client.setSessionProvider(session.id, providerId);
        onSession(updated);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const modelOptions = useMemo(() => {
    const set = new Set(models);
    if (config.model) set.add(config.model);
    return Array.from(set);
  }, [models, config.model]);

  const isSelected = selectedId === providerId;

  return (
    <section className="grid gap-3 rounded-lg bg-black/20 p-4 ring-1 ring-white/10">
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div>
          <h3 className="font-display text-xl text-ink-100">{title}</h3>
          <p className="mt-0.5 font-mono text-[11px] text-ink-500">{providerId}</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            type="button"
            className={`ide-btn ${config.enabled ? "text-emerald-300" : ""}`}
            disabled={busy !== null}
            onClick={() => void toggleEnabled()}
          >
            {config.enabled ? "Enabled" : "Disabled"}
          </button>
          <button
            type="button"
            className={`ide-btn ${isSelected ? "bg-ember-500/20 text-ember-400 ring-ember-500/40" : ""}`}
            disabled={busy !== null}
            onClick={() => void selectForChat()}
          >
            {isSelected ? "Selected for Chat" : "Use in Chat / Plan·Act"}
          </button>
        </div>
      </div>

      <label className="grid gap-1 text-xs text-ink-400">
        Base URL
        <input
          className="ide-input w-full py-2 text-sm"
          value={config.base_url}
          onChange={(e) => patch({ base_url: e.target.value })}
          spellCheck={false}
        />
      </label>

      {providerId === LMSTUDIO_ID ? (
        <label className="grid gap-1 text-xs text-ink-400">
          API key env var (optional)
          <input
            className="ide-input w-full py-2 text-sm"
            value={config.api_key_env ?? ""}
            placeholder="LM_STUDIO_API_KEY"
            onChange={(e) =>
              patch({ api_key_env: e.target.value.trim() ? e.target.value.trim() : null })
            }
            spellCheck={false}
          />
        </label>
      ) : null}

      <div className="flex flex-wrap items-end gap-2">
        <label className="grid min-w-[12rem] flex-1 gap-1 text-xs text-ink-400">
          Selected model
          <select
            className="ide-input w-full py-2 text-sm"
            value={config.model}
            onChange={(e) => patch({ model: e.target.value })}
          >
            {modelOptions.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </label>
        <label className="grid min-w-[10rem] flex-1 gap-1 text-xs text-ink-400">
          Or type model id
          <input
            className="ide-input w-full py-2 text-sm"
            value={config.model}
            onChange={(e) => patch({ model: e.target.value })}
            spellCheck={false}
          />
        </label>
      </div>

      <div className="flex flex-wrap gap-2">
        <button
          type="button"
          className="ide-btn"
          disabled={busy !== null}
          onClick={() => void save()}
        >
          {busy === "save" ? "Saving…" : savedFlash ? "Saved" : "Save"}
        </button>
        <button
          type="button"
          className="ide-btn"
          disabled={busy !== null}
          onClick={() => void refreshModels()}
        >
          {busy === "models" ? "Refreshing…" : "Refresh models"}
        </button>
        <button
          type="button"
          className="ide-btn"
          disabled={busy !== null}
          onClick={() => void testConnection()}
        >
          {busy === "probe" ? "Testing…" : "Test connection"}
        </button>
      </div>

      {probe ? (
        <div
          className={`rounded-md px-3 py-2 text-xs ring-1 ${
            probe.ok
              ? "bg-emerald-500/10 text-emerald-100 ring-emerald-500/30"
              : "bg-red-500/10 text-red-100 ring-red-500/30"
          }`}
        >
          <p className="font-mono">
            {probe.ok ? "ok" : "failed"} · {probe.latency_ms}ms · tier {probe.tier} · network{" "}
            {probe.network_allowed ? "allowed" : "blocked"}
          </p>
          {probe.models.length ? (
            <p className="mt-1 text-ink-300">{probe.models.length} model(s) discovered</p>
          ) : null}
          {probe.error ? <p className="mt-1 opacity-90">{probe.error}</p> : null}
        </div>
      ) : null}

      {error ? (
        <p className="rounded-md bg-red-500/10 px-3 py-2 text-sm text-red-200 ring-1 ring-red-500/30">
          {error}
        </p>
      ) : null}

      <p className="font-mono text-[10px] text-ink-500">
        Persist via source/llm/upsertProvider · probe via PolicyHttpClient · tier={tier}
      </p>
    </section>
  );
}

export function SettingsProvidersPanel({
  client,
  tier,
  session,
  selectedProviderId,
  onSelectedProviderChange,
  onSessionChange,
}: {
  client: SourceClient;
  tier: NetworkTier;
  session: SessionMeta | null;
  selectedProviderId: string | null;
  onSelectedProviderChange?: (id: string) => void;
  onSessionChange?: (s: SessionMeta) => void;
}) {
  const impact = tierImpact(tier);
  const [selectedId, setSelectedId] = useState<string | null>(selectedProviderId);
  const [sessionState, setSessionState] = useState(session);

  useEffect(() => {
    setSessionState(session);
  }, [session]);

  useEffect(() => {
    setSelectedId(selectedProviderId);
  }, [selectedProviderId]);

  useEffect(() => {
    void (async () => {
      try {
        const sel = await client.getSelectedProvider();
        const id = sel.provider_id ?? null;
        setSelectedId(id);
        if (id) onSelectedProviderChange?.(id);
      } catch {
        /* daemon may be reconnecting */
      }
    })();
  }, [client, onSelectedProviderChange]);

  const handleSelected = (id: string) => {
    setSelectedId(id);
    onSelectedProviderChange?.(id);
  };

  return (
    <div className="h-full min-h-0 overflow-auto p-4">
      <header className="mb-4">
        <h2 className="font-display text-2xl text-ink-50">Local LLM settings</h2>
        <p className="mt-1 max-w-2xl text-sm text-ink-300">
          Configure Ollama and LM Studio. Settings persist in the daemon SQLite{" "}
          <span className="font-mono">providers</span> table and drive Chat / Plan·Act completions.
        </p>
      </header>

      <div className="mb-4 rounded-lg bg-ember-500/10 px-4 py-3 ring-1 ring-ember-500/25">
        <p className="font-display text-lg text-ember-400">Network tier: {impact.title}</p>
        <p className="mt-1 text-sm text-ink-200">{impact.body}</p>
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <ProviderEditor
          title="Ollama"
          providerId={OLLAMA_ID}
          kind="ollama"
          client={client}
          tier={tier}
          selectedId={selectedId}
          session={sessionState}
          onSelected={handleSelected}
          onSession={(s) => {
            setSessionState(s);
            onSessionChange?.(s);
          }}
        />
        <ProviderEditor
          title="LM Studio"
          providerId={LMSTUDIO_ID}
          kind="lm_studio"
          client={client}
          tier={tier}
          selectedId={selectedId}
          session={sessionState}
          onSelected={handleSelected}
          onSession={(s) => {
            setSessionState(s);
            onSessionChange?.(s);
          }}
        />
      </div>
    </div>
  );
}
