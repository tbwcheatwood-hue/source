# Source Tauri v2 + React desktop app

Complete offline desktop scaffold (not HTML-only). Includes a calculator-class React UI and a full `src-tauri` host.

## Develop

```bash
pnpm install
pnpm tauri:dev
```

## Production binary

```bash
./scripts/build-desktop.sh
# Linux without WebKitGTK/GTK -dev packages:
./scripts/build-desktop.sh --install-deps
```

### Linux packages

```bash
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev \
  patchelf pkg-config build-essential curl wget file \
  libssl-dev libayatana-appindicator3-dev
```

Do **not** replace this desktop target with a standalone HTML file unless the user explicitly asked for a web page.
