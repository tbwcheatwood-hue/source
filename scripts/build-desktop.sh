#!/usr/bin/env bash
# Build Tauri desktop binaries (loads the same source-web UI).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Building source-web (Tauri frontendDist)"
pnpm install --frozen-lockfile=false
pnpm --filter source-web build

echo "==> Building Tauri desktop"
if ! pnpm --filter source-desktop tauri:build; then
  cat <<'EOF' >&2

Desktop build failed. On Linux, install WebKitGTK 4.1 + GTK 3, e.g.:
  sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev patchelf
Then re-run: ./scripts/build-desktop.sh

EOF
  exit 1
fi

echo "==> Desktop build complete (see source-desktop/src-tauri/target/release/bundle)"
