import { NetworkTier } from "../types/protocol";

const LABELS: Record<NetworkTier, string> = {
  LOCAL_ONLY: "Local only",
  LOCAL_AND_LAN: "Local + LAN",
  UNRESTRICTED_CONFIGURABLE: "Unrestricted",
};

export function NetworkTierBadge({
  tier,
  pulse,
}: {
  tier: NetworkTier;
  pulse?: boolean;
}) {
  const color =
    tier === "LOCAL_ONLY"
      ? "bg-emerald-500/20 text-emerald-200 ring-emerald-500/40"
      : tier === "LOCAL_AND_LAN"
        ? "bg-sky-500/20 text-sky-100 ring-sky-500/40"
        : "bg-ember-500/20 text-ember-400 ring-ember-500/40";

  return (
    <span
      className={`inline-flex items-center gap-2 rounded-md px-3 py-1.5 text-sm font-medium ring-1 ${color} ${
        pulse ? "animate-pulse" : ""
      }`}
    >
      <span className="h-2 w-2 rounded-full bg-current" />
      {LABELS[tier]}
      <span className="font-mono text-xs opacity-70">{tier}</span>
    </span>
  );
}