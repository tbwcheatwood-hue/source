# Source Desktop (Tauri v2)

Native shell for the Source IDE. Loads `source-web` via Tauri and bridges JSON-RPC to `source-daemon`.

## Dev

```bash
# terminal A
cargo run -p source-daemon -- --bind 127.0.0.1:7420
# terminal B
pnpm --filter source-desktop tauri:dev
```

## Production binary

From the monorepo root:

```bash
./scripts/build-desktop.sh --check-deps
./scripts/build-desktop.sh --install-deps   # Linux WebKitGTK/GTK -dev
./scripts/build-desktop.sh
```

Artifacts (workspace cargo target):

- `target/release/source-desktop`
- `target/release/bundle/{deb,rpm,appimage}/`

IPC commands: `bridge_info`, `daemon_rpc`, `daemon_status`, `daemon_network_tier`, `daemon_set_network_tier`, `daemon_health`.
