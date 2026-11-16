# Source (Phase 1–6)

Local-first agent IDE. Daemon + shared protocol (Phases 1–4), **unified dark IDE** (Phase 5), and **packaging / offline preview** (Phase 6).

## Layout

| Path | Role |
|------|------|
| `source-core` | Shared Rust types, JSON-RPC protocol **1.3.0**, network policy, mesh/cluster models |
| `source-daemon` | Tokio + Axum + SQLite daemon (`~/.source/`), tools, RAG, MCP, mesh, static UI hosting |
| `source-web` | React + Monaco + xterm IDE (Web + Tauri frontend) |
| `source-desktop` | Tauri v2 shell; IPC → daemon; loads `source-web` |
| `scripts/` | smoke, offline-init, web-server build, desktop build, e2e-offline |

## Prerequisites

- Rust **1.100+** (see `rust-toolchain.toml`)
- Node **20+** and **pnpm** 9+
- `git` (shadow checkpoints)
- Chromium/Chrome for `headless_browser` / `app_verifier`
- Optional: NVIDIA + `nvidia-smi` for cluster GPU metrics
- Linux desktop: WebKitGTK 4.1 / GTK 3 (Tauri). Install with `./scripts/build-desktop.sh --install-deps` or see `.sourcerules`.

## Verify (preview gate)

```bash
cargo check -p source-core -p source-daemon
cargo test  -p source-core -p source-daemon
pnpm install
pnpm --filter source-web typecheck
pnpm --filter source-web build
./scripts/smoke-daemon.sh
./scripts/e2e-offline.sh
```

## Local preview (Web Server mode)

```bash
# one-time offline seed
./scripts/offline-init.sh

# build UI + serve from daemon
./scripts/build-web-server.sh
cargo run -p source-daemon -- --bind 127.0.0.1:7420 --static-dir source-web/dist
```

Open **http://127.0.0.1:7420/** — the IDE, JSON-RPC (`/rpc`), and WebSocket (`/ws`) share that origin.

Dev hot-reload alternative:

```bash
# terminal A
cargo run -p source-daemon -- --bind 127.0.0.1:7420
# terminal B
pnpm --filter source-web dev
```

Open `http://127.0.0.1:5173` (Vite proxies `/rpc` + `/ws`).

## Desktop preview

```bash
cargo run -p source-daemon -- --bind 127.0.0.1:7420
pnpm --filter source-desktop tauri:dev
```

Production binary (needs WebKitGTK on Linux):

```bash
./scripts/build-desktop.sh --check-deps   # print / verify apt packages
./scripts/build-desktop.sh --install-deps # non-interactive apt when sudo allows
./scripts/build-desktop.sh
```

**Product rule:** when a user asks for a desktop app, Source ships a finished Tauri v2 project (`templates_ops` → `create_desktop_app` / template `tauri-v2-react`). Never refuse with “can’t build desktop here” or substitute HTML-only unless they asked for web-only. See `.sourcerules`.

IPC commands: `bridge_info`, `daemon_rpc`, `daemon_status`, `daemon_network_tier`, `daemon_set_network_tier`, `daemon_health`. The web UI auto-uses Tauri IPC when `__TAURI_INTERNALS__` is present.

## IDE surfaces (Phase 5)

- Monaco editor + Diff Viewer (workspace via `fs_ops`)
- xterm.js terminal (`source/pty/*`)
- Chat / Plan·Act panel (`source/mode/set`, `source/llm/complete` via selected local provider)
- LLM Settings for Ollama + LM Studio (`source/llm/{list,upsert,probe,listModels,setSelected}Provider`)
- Network Tier switch always in the header (`LOCAL_ONLY` allows localhost backends)
- Mesh & Cluster monitor (`source/mesh/*`, `source/cluster/resources`)
- Audit / Rollback timeline (`source/audit/list`, `source/checkpoint/*`)
- Offline Docs & Templates (`source/rag/*`, `source/templates/*`, vendor cache)
- Tools / MCP manager (`source/tools/list`, `source/mcp/*`, approvals)

## Network tiers

1. **LOCAL_ONLY** — loopback; mesh disabled
2. **LOCAL_AND_LAN** — RFC 1918 + CGNAT `100.64.0.0/10`; mesh allowed
3. **UNRESTRICTED_CONFIGURABLE** — allowlist + approval for public hosts

## Data directory

Defaults to `~/.source/` (`SOURCE_HOME` overrides the home used for that path). Seed with `./scripts/offline-init.sh`.
