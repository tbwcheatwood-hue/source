# Source (Phase 1–3)

Local-first agent IDE monorepo. **Phases 1–3**: architecture, shared protocol, daemon with network policy / LLM router / context compactor / shadow git / Plan·Act, plus offline tool suite / app factory / MCP host, and web + Tauri shells wired to the daemon.

## Layout

| Path | Role |
|------|------|
| `source-core` | Shared Rust types, JSON-RPC protocol 1.1.0, network policy, context models |
| `source-daemon` | Tokio + Axum + SQLite daemon (`~/.source/`), offline tools, RAG, MCP host, PTY, browser |
| `source-web` | React + TypeScript + Vite + Tailwind app shell (tier + WS client) |
| `source-desktop` | Tauri v2 shell with real IPC → daemon JSON-RPC |

## Prerequisites

- Rust **1.100+** (see `rust-toolchain.toml`)
- Node **20+** and **pnpm** 9+
- `git` (shadow checkpoints)
- Chromium/Chrome for `headless_browser` / `app_verifier`
- Linux desktop build also needs WebKitGTK 4.1 / GTK 3 (Tauri)

## Build & test (Phase 1–3)

```bash
# Rust
cargo check -p source-core -p source-daemon
cargo test  -p source-core -p source-daemon
cargo check -p source-desktop   # needs WebKitGTK on Linux

# Web
pnpm install
pnpm --filter source-web typecheck
pnpm --filter source-web build
```

## Run the daemon

```bash
cargo run -p source-daemon -- --bind 127.0.0.1:7420
# optional static hosting of the web build:
# cargo run -p source-daemon -- --static-dir source-web/dist
```

Data directory defaults to `~/.source/` (`SOURCE_HOME` overrides the home used for that path).

### API surface

- `GET /health`
- `GET /api/status`
- `GET /api/network/tier`
- `POST /rpc` — JSON-RPC 2.0
- `WS /ws` — same JSON-RPC methods + `source/hello` notification

Key methods: `source/status`, `source/network/{getTier,setTier,evaluate}`, `source/session/*`, `source/mode/set`, `source/llm/*`, `source/context/*`, `source/checkpoint/*`, `source/approval/*`, `source/tools/*`, `source/audit/list`, plus Phase 3: `source/pty/*`, `source/rag/*`, `source/templates/*`, `source/mcp/*`, `source/browser/*`.

### Phase 3 offline suite

Built-ins: `fs_ops`, `terminal_exec`, `local_rag_docs`, `headless_browser`, `app_verifier`, `sqlite_db_inspector`, `git_ops`, `archive_and_system`, `symbol_search`. Templates seed under `~/.source/templates/`; MCP extensions under `~/.source/extensions/`; docs under `~/.source/offline-docs/`.

## Web shell

```bash
# terminal A
cargo run -p source-daemon -- --bind 127.0.0.1:7420
# terminal B
pnpm --filter source-web dev
```

Open `http://127.0.0.1:5173`. The shell shows the live network tier, WS connection state, and provider list.

## Desktop shell

```bash
# daemon must be running
pnpm --filter source-desktop tauri:dev
```

IPC commands: `bridge_info`, `daemon_rpc`, `daemon_status`, `daemon_network_tier`, `daemon_set_network_tier`, `daemon_health`.

## Network tiers (enforced in daemon)

1. **LOCAL_ONLY** — loopback only (`127.0.0.1` / `::1`)
2. **LOCAL_AND_LAN** — RFC 1918 + CGNAT `100.64.0.0/10` (Tailscale/WireGuard)
3. **UNRESTRICTED_CONFIGURABLE** — allowlist + approval for public hosts

Cloud LLM connectors stay dormant until **enabled** and the tier permits egress.

## Plan / Act

- **Plan** — read-only tools only
- **Act** — File Edits, Shell, Network, MCP, Cross-Node require approval gates

## Not in this phase

Phases 4–6 (full IDE UX, multi-node, etc.) are intentionally unimplemented.