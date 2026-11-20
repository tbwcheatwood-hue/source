#!/usr/bin/env bash
# Build Source Tauri desktop binaries (loads the same source-web UI).
# Stages source-daemon as a Tauri sidecar for auto-start.
# Never abandon the desktop target for HTML-only substitutes.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

INSTALL_DEPS=0
CHECK_ONLY=0
SKIP_DAEMON=0
for arg in "$@"; do
  case "$arg" in
    --install-deps) INSTALL_DEPS=1 ;;
    --check-deps) CHECK_ONLY=1 ;;
    --skip-daemon) SKIP_DAEMON=1 ;;
    -h|--help)
      cat <<'EOF'
Usage: ./scripts/build-desktop.sh [--install-deps] [--check-deps] [--skip-daemon]

Builds the Source IDE Tauri v2 desktop binary (+ source-daemon sidecar).
  --install-deps  On Linux, non-interactively apt-get install WebKitGTK/GTK -dev packages when possible.
  --check-deps    Print dependency status, then exit.
  --skip-daemon   Reuse an already-staged sidecar binary under source-desktop/src-tauri/binaries/.

Windows 11: prefer .\scripts\build-desktop.ps1 (NSIS/MSI). This bash script also works from Git Bash.
EOF
      exit 0
      ;;
  esac
done

OS="$(uname -s 2>/dev/null || echo unknown)"

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

print_windows_deps_help() {
  cat <<'EOF'
Windows 11 prerequisites:
  - Rust MSVC toolchain: rustup default stable-x86_64-pc-windows-msvc
  - Visual Studio Build Tools (Desktop development with C++ / Windows SDK)
  - Node.js 20+ and pnpm 9+
  - WebView2 Runtime (preinstalled on Windows 11)
  - git

Preferred build entrypoint:
  .\scripts\build-desktop.ps1
  .\scripts\build-desktop.ps1 -CheckDeps
EOF
}

ensure_linux_deps() {
  if [[ "$OS" != "Linux" ]]; then
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

check_windows_cli_deps() {
  local missing=0
  for bin in rustc cargo node pnpm git; do
    if command -v "$bin" >/dev/null 2>&1; then
      echo "  [ok] $bin"
    else
      echo "  [MISSING] $bin"
      missing=1
    fi
  done
  if [[ "$missing" -eq 1 ]]; then
    print_windows_deps_help >&2
    return 1
  fi
  return 0
}

host_triple() {
  rustc -vV | awk '/^host:/{print $2; exit}'
}

stage_daemon_sidecar() {
  local triple sidecar src
  triple="$(host_triple)"
  mkdir -p source-desktop/src-tauri/binaries
  case "$OS" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
      sidecar="source-desktop/src-tauri/binaries/source-daemon-${triple}.exe"
      src="target/release/source-daemon.exe"
      ;;
    *)
      sidecar="source-desktop/src-tauri/binaries/source-daemon-${triple}"
      src="target/release/source-daemon"
      ;;
  esac

  if [[ "$SKIP_DAEMON" -eq 1 ]]; then
    if [[ ! -f "$sidecar" ]]; then
      echo "Sidecar missing at $sidecar (omit --skip-daemon)" >&2
      exit 1
    fi
    echo "==> Reusing staged sidecar $sidecar"
    return 0
  fi

  echo "==> Building source-daemon (sidecar)"
  cargo build -p source-daemon --release
  if [[ ! -f "$src" ]]; then
    echo "expected daemon binary at $src" >&2
    exit 1
  fi
  cp -f "$src" "$sidecar"
  chmod +x "$sidecar" 2>/dev/null || true
  echo "    staged $sidecar"
}

if [[ "$CHECK_ONLY" -eq 1 ]]; then
  case "$OS" in
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
      echo "==> Windows dependency check (Git Bash)"
      check_windows_cli_deps
      print_windows_deps_help
      echo "All checked CLI deps OK"
      ;;
    Linux)
      ensure_linux_deps
      echo "All checked deps OK"
      ;;
    *)
      echo "==> Non-Linux host ($OS): ensure Rust/Node/pnpm/git and platform WebView deps"
      for bin in rustc cargo node pnpm git; do
        command -v "$bin" >/dev/null 2>&1 && echo "  [ok] $bin" || echo "  [MISSING] $bin"
      done
      ;;
  esac
  exit 0
fi

case "$OS" in
  Linux) ensure_linux_deps ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    echo "==> Windows host detected — prefer .\\scripts\\build-desktop.ps1 for NSIS/MSI"
    check_windows_cli_deps || true
    ;;
esac

stage_daemon_sidecar

echo "==> Building source-web (Tauri frontendDist)"
pnpm install --frozen-lockfile=false
pnpm --filter source-web build

echo "==> Building Tauri desktop (source-desktop)"
build_ok=0
case "$OS" in
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    # Match PowerShell script: ship installers Windows users expect.
    if pnpm --filter source-desktop exec tauri build -- --bundles nsis,msi; then
      build_ok=1
    fi
    ;;
  *)
    if pnpm --filter source-desktop tauri:build; then
      build_ok=1
    fi
    ;;
esac

if [[ "$build_ok" -ne 1 ]]; then
  cat <<'EOF' >&2

Desktop build failed.
EOF
  if [[ "$OS" == "Linux" ]]; then
    print_linux_deps_help >&2
  else
    print_windows_deps_help >&2
  fi
  exit 1
fi

echo "==> Desktop build complete"
echo "    see source-desktop/src-tauri/target/release/ and bundle/"
ls -la source-desktop/src-tauri/target/release/bundle 2>/dev/null || true
