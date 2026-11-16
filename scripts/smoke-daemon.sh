#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
TMP="$(mktemp -d)"
PID=""
cleanup() {
  if [[ -n "${PID}" ]]; then kill "${PID}" 2>/dev/null || true; wait "${PID}" 2>/dev/null || true; fi
  rm -rf "$TMP"
}
trap cleanup EXIT

cargo build -q -p source-daemon
export SOURCE_HOME="$TMP"
"$ROOT/target/debug/source-daemon" --bind 127.0.0.1:17420 &
PID=$!

ok=0
for _ in $(seq 1 80); do
  if curl -sf http://127.0.0.1:17420/health >/dev/null; then ok=1; break; fi
  sleep 0.25
done
if [[ "$ok" -ne 1 ]]; then
  echo "daemon failed to become healthy" >&2
  exit 1
fi

curl -sf http://127.0.0.1:17420/health | tee "$TMP/health.json"
echo
curl -sf -X POST http://127.0.0.1:17420/rpc \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"source/network/getTier","params":{}}' | tee "$TMP/tier.json"
echo
curl -sf -X POST http://127.0.0.1:17420/rpc \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"source/status","params":{}}' | tee "$TMP/status.json"
echo
echo "smoke ok"