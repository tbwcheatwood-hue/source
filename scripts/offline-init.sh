#!/usr/bin/env bash
# Seed ~/.source/ (or $SOURCE_HOME/.source) with templates, caches, offline docs, extensions.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOME_BASE="${SOURCE_HOME:-${HOME}}"
SOURCE_ROOT="${HOME_BASE}/.source"

echo "==> Seeding Source offline layout at ${SOURCE_ROOT}"

mkdir -p \
  "${SOURCE_ROOT}/offline-docs" \
  "${SOURCE_ROOT}/templates" \
  "${SOURCE_ROOT}/extensions" \
  "${SOURCE_ROOT}/cache/rag" \
  "${SOURCE_ROOT}/cache/vendor/npm" \
  "${SOURCE_ROOT}/cache/vendor/pnpm" \
  "${SOURCE_ROOT}/cache/vendor/cargo" \
  "${SOURCE_ROOT}/cache/vendor/wheels" \
  "${SOURCE_ROOT}/checkpoints"

# Starter offline docs so RAG has something to index without WAN.
DOC="${SOURCE_ROOT}/offline-docs/source-overview.md"
if [[ ! -f "${DOC}" ]]; then
  cat > "${DOC}" <<'EOF'
# Source offline docs

Source is a local-first agent IDE.

## Network tiers

- LOCAL_ONLY — loopback only; mesh disabled
- LOCAL_AND_LAN — RFC1918 + Tailscale CGNAT; mesh allowed
- UNRESTRICTED_CONFIGURABLE — allowlist + approval for public hosts

## Plan / Act

Plan mode is read-only. Act mode requires approval gates for File Edits, Shell, Network, MCP, and Cross-Node.

## Mesh

Pair peers with a 6-digit PIN. Register hosts by IP or Tailscale MagicDNS.
EOF
fi

# Always refresh desktop guardrails doc (product rule — agents must not refuse desktop builds).
cat > "${SOURCE_ROOT}/offline-docs/desktop-apps.md" <<'EOF'
# Desktop apps (HARD RULE)

NEVER refuse native/desktop requests with “this environment can’t”, “use Electron later”, or “HTML-only instead”.

When the user asks for a desktop app:

1. Seed offline layout (`offline-init.sh`) so `~/.source/templates/tauri-v2-react` exists.
2. Create via `templates_ops` `create_desktop_app` (or `source/templates/scaffold` with `tauri-v2-react`).
3. Build with `./scripts/build-desktop.sh` / `pnpm tauri build`.
4. If WebKitGTK/-dev packages are missing, install exact apt packages or run `--install-deps`, and still leave the complete Tauri project.

HTML-only is allowed only when the user explicitly asked for a web page.

Linux deps:

```bash
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev patchelf \
  pkg-config build-essential curl wget file libssl-dev \
  libayatana-appindicator3-dev
```
EOF

# Optional sample MCP extension (disabled echo stub via stdio python).
EXT_DIR="${SOURCE_ROOT}/extensions/sample-echo"
if [[ ! -f "${EXT_DIR}/mcp.json" ]]; then
  mkdir -p "${EXT_DIR}"
  cat > "${EXT_DIR}/mcp.json" <<'EOF'
{
  "id": "sample-echo",
  "name": "Sample Echo",
  "transport": "stdio",
  "command": ["python3", "-c", "import sys; print('{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"tools\":[]}}', flush=True); sys.stdin.readline()"],
  "offline_capable": true,
  "requires_wan": false,
  "enabled": false
}
EOF
fi

# Boot the daemon once so TemplateManager seeds builtin templates into ~/.source/templates.
echo "==> Bootstrapping daemon to seed templates…"
TMP="$(mktemp -d)"
cleanup() { rm -rf "${TMP}"; }
trap cleanup EXIT

export SOURCE_HOME="${HOME_BASE}"
cargo build -q -p source-daemon
BIND="127.0.0.1:17421"
"${ROOT}/target/debug/source-daemon" --bind "${BIND}" >/tmp/source-offline-init.log 2>&1 &
PID=$!
ok=0
for _ in $(seq 1 80); do
  if curl -sf "http://${BIND}/health" >/dev/null; then ok=1; break; fi
  sleep 0.25
done
if [[ "${ok}" -ne 1 ]]; then
  echo "daemon failed during offline-init; see /tmp/source-offline-init.log" >&2
  kill "${PID}" 2>/dev/null || true
  exit 1
fi

curl -sf -X POST "http://${BIND}/rpc" \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"source/templates/list","params":{}}' \
  | tee "${TMP}/templates.json"
echo
curl -sf -X POST "http://${BIND}/rpc" \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"source/rag/index","params":{"force":true}}' \
  | tee "${TMP}/rag.json"
echo

kill "${PID}" 2>/dev/null || true
wait "${PID}" 2>/dev/null || true

echo "==> Offline init complete"
echo "    data root: ${SOURCE_ROOT}"
ls -la "${SOURCE_ROOT}/templates" || true
ls -la "${SOURCE_ROOT}/offline-docs" || true
