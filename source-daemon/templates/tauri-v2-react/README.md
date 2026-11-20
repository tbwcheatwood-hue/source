# Source Tauri v2 + React desktop app

Complete offline desktop scaffold (not HTML-only). Includes a calculator-class React UI and a full `src-tauri` host.

## Develop

```bash
pnpm install
pnpm tauri:dev
```

## Production binary

### Linux / macOS

```bash
./scripts/build-desktop.sh
# Linux without WebKitGTK/GTK -dev packages:
./scripts/build-desktop.sh --install-deps
```

### Windows 11

```powershell
.\scripts\build-desktop.ps1
```

Requires Rust MSVC toolchain, Visual Studio Build Tools (C++ + Windows SDK), Node 20+, pnpm, and WebView2 (preinstalled on Windows 11).

### Linux packages

```bash
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev \
  patchelf pkg-config build-essential curl wget file \
  libssl-dev libayatana-appindicator3-dev
```

Do **not** replace this desktop target with a standalone HTML file unless the user explicitly asked for a web page.
