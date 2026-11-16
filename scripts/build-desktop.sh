#!/usr/bin/env bash
# Build Source Tauri desktop binaries (loads the same source-web UI).
# Never abandon the desktop target for HTML-only substitutes.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

INSTALL_DEPS=0
CHECK_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --install-deps) INSTALL_DEPS=1 ;;
    --check-deps) CHECK_ONLY=1 ;;
    -h|--help)
      cat <<'EOF'
Usage: ./scripts/build-desktop.sh [--install-deps] [--check-deps]

Builds the Source IDE Tauri v2 desktop binary.
  --install-deps  On Linux, non-interactively apt-get install WebKitGTK/GTK -dev packages when possible.
  --check-deps    Print dependency status and exact apt packages, then exit.
EOF
      exit 0
      ;;
  esac
done

linux_deps_list() {
  printf '%s' \
    'libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev patchelf pkg-config build-essential curl wget file libssl-dev libayatana-appindicator3-dev'
}

print_linux_deps_help() {
  cat <<EOF
Linux Tauri / WebKitGTK packages required:
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y $(linux_deps_list)

Or re-run:
  ./scripts/build-desktop.sh --install-deps

The Source desktop project under source-desktop/ remains complete; build once deps exist.
Do NOT substitute an HTML-only package for the desktop target.
EOF
}

ensure_linux_deps() {
  if [[ "$(uname -s)" != "Linux" ]]; then
    return 0
  fi
  local missing=()
  local pkg
  for pkg in libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev; do
    if ! dpkg -s "$pkg" >/dev/null 2>&1; then
      missing+=("$pkg")
    fi
  done
  if [[ "${#missing[@]}" -eq 0 ]]; then
    echo "==> Linux Tauri deps present"
    return 0
  fi
  echo "==> Missing packages: ${missing[*]}"
  if [[ "$CHECK_ONLY" -eq 1 ]]; then
    print_linux_deps_help
    exit 2
  fi
  if [[ "$INSTALL_DEPS" -eq 1 ]]; then
    echo "==> Installing Linux Tauri deps (non-interactive apt)"
    if ! command -v sudo >/dev/null 2>&1; then
      echo "sudo not available; install packages manually:" >&2
      print_linux_deps_help >&2
      exit 1
    fi
    sudo DEBIAN_FRONTEND=noninteractive apt-get update -y || true
    # shellcheck disable=SC2046
    if ! sudo DEBIAN_FRONTEND=noninteractive apt-get install -y $(linux_deps_list); then
      echo "==> apt reported errors; checking whether required -dev packages are present…" >&2
      local still=()
      for pkg in libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev; do
        if ! dpkg -s "$pkg" >/dev/null 2>&1; then
          still+=("$pkg")
        fi
      done
      if [[ "${#still[@]}" -gt 0 ]]; then
        echo "Still missing: ${still[*]}" >&2
        print_linux_deps_help >&2
        exit 1
      fi
      echo "==> Required WebKitGTK/GTK -dev packages are installed; continuing"
    fi
    return 0
  fi
  print_linux_deps_help >&2
  echo "==> Continuing build attempt so the failure surfaces real linker/pkg-config errors…" >&2
}

if [[ "$CHECK_ONLY" -eq 1 ]]; then
  ensure_linux_deps
  echo "All checked deps OK"
  exit 0
fi

ensure_linux_deps

echo "==> Building source-web (Tauri frontendDist)"
pnpm install --frozen-lockfile=false
pnpm --filter source-web build

echo "==> Building Tauri desktop (source-desktop)"
if ! pnpm --filter source-desktop tauri:build; then
  cat <<'EOF' >&2

Desktop build failed.
EOF
  print_linux_deps_help >&2
  exit 1
fi

echo "==> Desktop build complete"
echo "    see source-desktop/src-tauri/target/release/ and bundle/"
ls -la source-desktop/src-tauri/target/release/bundle 2>/dev/null || true
