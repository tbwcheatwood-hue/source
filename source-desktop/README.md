# Source Desktop (Tauri v2)

Native shell for the Source IDE. Loads `source-web` via Tauri and bridges JSON-RPC to `source-daemon`. On launch, the shell health-checks the daemon and auto-starts the bundled `source-daemon` sidecar when needed.

## Dev

### Linux / macOS

```bash
# terminal A
cargo run -p source-daemon -- --bind 127.0.0.1:7420
# terminal B
pnpm --filter source-desktop tauri:dev
```

### Windows 11

```powershell
# From repo root
.\scripts\run-desktop.ps1
# Or manually:
cargo run -p source-daemon -- --bind 127.0.0.1:7420
pnpm --filter source-desktop tauri:dev
```

`tauri:dev` builds/stages `source-daemon` as a sidecar first.

## Production binary

### Linux

```bash
./scripts/build-desktop.sh --check-deps
./scripts/build-desktop.sh --install-deps   # WebKitGTK/GTK -dev
./scripts/build-desktop.sh
```

Artifacts (Cargo **workspace** `target/`):

- `target/release/source-desktop`
- `target/release/bundle/{deb,rpm,appimage}/`
- Sidecar staged at: `source-desktop/src-tauri/binaries/source-daemon-<triple>`

### Windows 11

```powershell
.\scripts\build-desktop.ps1 -CheckDeps
.\scripts\build-desktop.ps1
```

Prerequisites:

- Rust **MSVC** toolchain (`rustup default stable-x86_64-pc-windows-msvc`)
- Visual Studio Build Tools (C++ workload + Windows SDK)
- Node 20+, pnpm 9+, git
- WebView2 Runtime (preinstalled on Windows 11; installer can bootstrap if missing)

Artifacts (Cargo **workspace** `target/`):

- `target\release\source-desktop.exe`
- `target\release\bundle\nsis\` (setup installer)
- `target\release\bundle\msi\`
- Sidecar staged at: `source-desktop\src-tauri\binaries\source-daemon-x86_64-pc-windows-msvc.exe`

Git Bash users can also run `./scripts/build-desktop.sh` (still prefer the `.ps1` on Windows).

IPC commands: `bridge_info`, `daemon_rpc`, `daemon_status`, `daemon_network_tier`, `daemon_set_network_tier`, `daemon_health`.
