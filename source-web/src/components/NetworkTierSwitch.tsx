import { NetworkTier } from "../types/protocol";
import { NetworkTierBadge } from "./NetworkTierBadge";

const TIERS: NetworkTier[] = [
  "LOCAL_ONLY",
  "LOCAL_AND_LAN",
  "UNRESTRICTED_CONFIGURABLE",
];

const SHORT: Record<NetworkTier, string> = {
  LOCAL_ONLY: "Local",
  LOCAL_AND_LAN: "LAN",
  UNRESTRICTED_CONFIGURABLE: "Open",
};

export function NetworkTierSwitch({
  tier,
  onChange,
  pulse,
}: {
  tier: NetworkTier;
  onChange: (tier: NetworkTier) => void;
  pulse?: boolean;
}) {
  return (
    <div className="flex items-center gap-3">
      <NetworkTierBadge tier={tier} pulse={pulse} />
      <div
        className="inline-flex overflow-hidden rounded-md ring-1 ring-white/15"
        role="group"
        aria-label="Network tier"
      >
        {TIERS.map((t) => (
          <button
            key={t}
            type="button"
            title={t}
            onClick={() => onChange(t)}
            className={`px-2.5 py-1.5 font-mono text-xs transition ${
              tier === t
                ? "bg-ember-500/25 text-ember-400"
                : "bg-black/20 text-ink-300 hover:bg-white/10 hover:text-ink-100"
            }`}
          >
            {SHORT[t]}
          </button>
        ))}
      </div>
    </div>
  );
}
