#!/usr/bin/env bash
# Build source-web static assets for daemon-hosted Web Server mode.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Installing JS deps (pnpm)"
pnpm install --frozen-lockfile=false

echo "==> Typecheck + build source-web"
pnpm --filter source-web typecheck
pnpm --filter source-web build

DIST="${ROOT}/source-web/dist"
test -f "${DIST}/index.html"

echo "==> Web assets ready at ${DIST}"
echo
echo "Preview (standalone web server mode):"
echo "  cargo run -p source-daemon -- --bind 127.0.0.1:7420 --static-dir source-web/dist"
echo "  open http://127.0.0.1:7420/"
