#!/usr/bin/env bash
# Build this Tauri v2 desktop app. Prefer this over HTML-only substitutes.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

INSTALL_DEPS=0
for arg in "$@"; do
  case "$arg" in
    --install-deps) INSTALL_DEPS=1 ;;
    -h|--help)
      cat <<'EOF'
Usage: ./scripts/build-desktop.sh [--install-deps]

Builds the Tauri v2 desktop binary for this project.
  --install-deps  On Linux, apt-get install WebKitGTK/GTK -dev packages (non-interactive).

Windows 11: prefer .\scripts\build-desktop.ps1
EOF
      exit 0
      ;;
  esac
done

linux_deps() {
  cat <<'EOF'
libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev patchelf pkg-config build-essential curl wget file libssl-dev libayatana-appindicator3-dev
EOF
}

OS="$(uname -s 2>/dev/null || echo unknown)"

if [[ "$OS" == "Linux" ]]; then
  missing=0
  for pkg in libwebkit2gtk-4.1-dev libgtk-3-dev; do
    if ! dpkg -s "$pkg" >/dev/null 2>&1; then
      missing=1
      break
    fi
  done
  if [[ "$missing" -eq 1 ]]; then
    if [[ "$INSTALL_DEPS" -eq 1 ]]; then
      echo "==> Installing Linux Tauri deps (non-interactive apt)"
      sudo DEBIAN_FRONTEND=noninteractive apt-get update -y || true
      # shellcheck disable=SC2046
      if ! sudo DEBIAN_FRONTEND=noninteractive apt-get install -y $(linux_deps); then
        if ! dpkg -s libwebkit2gtk-4.1-dev libgtk-3-dev >/dev/null 2>&1; then
          echo "Required WebKitGTK/GTK packages still missing after apt." >&2
          exit 1
        fi
        echo "==> Required -dev packages present despite apt noise; continuing"
      fi
    else
      cat <<EOF >&2
Missing Linux WebKitGTK/GTK -dev packages.
Install with:
  sudo apt-get install -y $(linux_deps)
Or re-run: ./scripts/build-desktop.sh --install-deps
The project remains a complete Tauri desktop app; build once deps exist.
EOF
    fi
  fi
fi

case "$OS" in
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    echo "==> Windows host: prefer .\\scripts\\build-desktop.ps1 for NSIS/MSI"
    ;;
esac

echo "==> Installing JS deps"
if command -v pnpm >/dev/null 2>&1; then
  pnpm install --frozen-lockfile=false
  echo "==> tauri build"
  case "$OS" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT) pnpm exec tauri build --bundles nsis,msi ;;
    *) pnpm tauri build ;;
  esac
else
  npm install
  echo "==> tauri build"
  case "$OS" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT) npx tauri build --bundles nsis,msi ;;
    *) npx tauri build ;;
  esac
fi

echo "==> Desktop build complete"
echo "    artifacts under src-tauri/target/release/bundle"
