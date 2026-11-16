# Source (Phase 1–2)

Local-first agent IDE monorepo. **Phases 1–2 only**: architecture, shared protocol, daemon with network policy / LLM router / context compactor / shadow git / Plan·Act, plus web + Tauri shells wired to the daemon.

## Layout

| Path | Role |
|------|------|
| `source-core` | Shared Rust types, JSON-RPC protocol, network policy types, context models |
| `source-daemon` | Tokio + Axum + SQLite daemon (`~/.source/`), HTTP JSON-RPC + WebSocket |
| `source-web` | React + TypeScript + Vite + Tailwind app shell (tier + WS client) |
| `source-desktop` | Tauri v2 shell with real IPC → daemon JSON-RPC |

## Prerequisites

- Rust **1.100+** (see `rust-toolchain.toml`)
- Node **20+** and **pnpm** 9+
- `git` (shadow checkpoints)
- Linux desktop build also needs WebKitGTK 4.1 / GTK 3 (Tauri)

## Build & test (Phase 1–2)

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

Key methods: `source/status`, `source/network/{getTier,setTier,evaluate}`, `source/session/*`, `source/mode/set`, `source/llm/*`, `source/context/*`, `source/checkpoint/*`, `source/approval/*`, `source/tools/*`, `source/audit/list`.

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

Phases 3–6 (full IDE, RAG retrieval, multi-node, etc.) are intentionally unimplemented. Stable interfaces Phase 2 needs are exported and fully wired.