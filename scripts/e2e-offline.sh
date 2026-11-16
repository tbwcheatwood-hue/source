#!/usr/bin/env bash
# Automated verification: daemon serves built web UI + core APIs work offline.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

TMP="$(mktemp -d)"
PID=""
cleanup() {
  if [[ -n "${PID}" ]]; then kill "${PID}" 2>/dev/null || true; wait "${PID}" 2>/dev/null || true; fi
  rm -rf "${TMP}"
}
trap cleanup EXIT

echo "==> Building daemon + web"
cargo build -q -p source-daemon
pnpm install --frozen-lockfile=false >/tmp/source-e2e-pnpm.log 2>&1 || {
  echo "pnpm install failed; see /tmp/source-e2e-pnpm.log" >&2
  exit 1
}
pnpm --filter source-web build >/tmp/source-e2e-web.log 2>&1 || {
  echo "web build failed; see /tmp/source-e2e-web.log" >&2
  exit 1
}

export SOURCE_HOME="${TMP}"
mkdir -p "${TMP}/.source/offline-docs" "${TMP}/.source/templates" "${TMP}/.source/extensions"
cat > "${TMP}/.source/offline-docs/readme.md" <<'EOF'
# Offline E2E
Source daemon serves the IDE without WAN.
EOF

BIND="127.0.0.1:17422"
DIST="${ROOT}/source-web/dist"
"${ROOT}/target/debug/source-daemon" --bind "${BIND}" --static-dir "${DIST}" \
  >/tmp/source-e2e-daemon.log 2>&1 &
PID=$!

ok=0
for _ in $(seq 1 100); do
  if curl -sf "http://${BIND}/health" >/dev/null; then ok=1; break; fi
  sleep 0.25
done
if [[ "${ok}" -ne 1 ]]; then
  echo "daemon failed to become healthy; see /tmp/source-e2e-daemon.log" >&2
  exit 1
fi

fail() { echo "e2e FAIL: $*" >&2; exit 1; }

echo "==> /health"
curl -sf "http://${BIND}/health" | tee "${TMP}/health.json"
echo

echo "==> static index.html"
curl -sf "http://${BIND}/" -o "${TMP}/index.html" || fail "GET /"
grep -qi '<div id="root"' "${TMP}/index.html" || fail "missing #root"
grep -qi 'script type="module"' "${TMP}/index.html" || fail "missing module script"
echo "index ok ($(wc -c < "${TMP}/index.html") bytes)"

rpc() {
  local id="$1" method="$2" params="${3:-"{}"}" out="${4:-${TMP}/rpc-${id}.json}"
  local req="${TMP}/req-${id}.json" code
  printf '%s\n' "${params}" > "${TMP}/params-${id}.json"
  python3 -c '
import json, sys
path, rid, method, params_path = sys.argv[1:5]
params = json.load(open(params_path))
body = {"jsonrpc": "2.0", "id": int(rid), "method": method, "params": params}
open(path, "w").write(json.dumps(body, separators=(",", ":")))
' "${req}" "${id}" "${method}" "${TMP}/params-${id}.json"
  code="$(curl -sS -o "${out}" -w '%{http_code}' -X POST "http://${BIND}/rpc" \
    -H 'content-type: application/json' \
    --data-binary @"${req}")" \
    || fail "RPC ${method} curl transport"
  [[ "${code}" == "200" ]] || fail "RPC ${method} HTTP ${code}: $(head -c 400 "${out}")"
  python3 -c '
import json, sys
d = json.load(open(sys.argv[1]))
if "error" in d:
    raise SystemExit("json-rpc error: %s" % d["error"])
if "result" not in d:
    raise SystemExit("missing result: %s" % d)
' "${out}" || fail "RPC result error in ${out}"
  echo "ok ${method}"
}

echo "==> core APIs (offline)"
rpc 1 source/status '{}' "${TMP}/status.json"
rpc 2 source/network/getTier '{}' "${TMP}/tier.json"
rpc 3 source/tools/list '{}' "${TMP}/tools.json"
rpc 4 source/templates/list '{}' "${TMP}/templates.json"
rpc 5 source/rag/index '{"force":true}' "${TMP}/rag.json"
rpc 6 source/mcp/list '{}' "${TMP}/mcp.json"
rpc 7 source/mesh/status '{}' "${TMP}/mesh.json"
rpc 8 source/session/create "{\"title\":\"e2e\",\"workspace_root\":\"${TMP}\",\"mode\":\"plan\"}" "${TMP}/session.json"

SESSION_ID="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["result"]["id"])' "${TMP}/session.json")"
rpc 9 source/audit/list "{\"session_id\":\"${SESSION_ID}\",\"limit\":10}" "${TMP}/audit.json"
rpc 10 source/network/setTier '{"tier":"LOCAL_AND_LAN"}' "${TMP}/set_tier.json"

echo "==> cluster REST"
curl -sf "http://${BIND}/api/cluster/resources" -o "${TMP}/cluster.json" || fail "cluster resources"
python3 -c 'import json; json.load(open("'"${TMP}/cluster.json"'"))' || fail "cluster json"

JS_PATH="$(python3 - <<'PY'
import pathlib,re
html=pathlib.Path("source-web/dist/index.html").read_text()
m=re.search(r'src="(/assets/[^"]+\.js)"', html)
print(m.group(1) if m else "")
PY
)"
if [[ -n "${JS_PATH}" ]]; then
  echo "==> static asset ${JS_PATH}"
  curl -sf "http://${BIND}${JS_PATH}" -o /dev/null || fail "asset ${JS_PATH}"
fi

echo
echo "e2e-offline ok"
echo "  UI:    http://${BIND}/"
echo "  health http://${BIND}/health"
