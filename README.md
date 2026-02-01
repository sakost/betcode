# BetCode

**Multi-client, multi-machine infrastructure for Claude Code.**

BetCode wraps [Claude Code](https://docs.anthropic.com/en/docs/claude-code) (Anthropic's official CLI) as a subprocess and builds orchestration, transport, persistence, and UI layers around it. Access your coding agent from a terminal TUI, a mobile app, or across machines via a self-hosted relay -- all with full agent fidelity.

> **Disclaimer**: BetCode is an independent, community-driven project. It is **not** affiliated with, endorsed by, or sponsored by Anthropic or the Claude team. Claude and Claude Code are products of Anthropic. BetCode simply wraps the publicly available Claude Code CLI.

---

## Status

**Design Phase** -- Architecture is fully documented; implementation has not started yet. See the [Roadmap](#roadmap) for the planned phases.

---

## Why BetCode?

Claude Code is a powerful coding agent, but it runs as a single-user CLI on one machine. BetCode adds the missing infrastructure:

| Capability | Claude Code | BetCode |
|---|---|---|
| Agent intelligence | Native | Inherited (wrapper) |
| Mobile client | No | Flutter app |
| Multi-machine access | No | Self-hosted relay + mTLS tunnel |
| Git worktree management | No | First-class |
| GitLab integration | GitHub only | GitLab-native |
| Offline queueing | No | Client-side SQLite |
| Session multiplexing | Single user | Multi-client |
| Self-hosted relay | No | Built-in |

Because BetCode runs Claude Code as a subprocess, **every tool, MCP server, hook, skill, and prompt that works with `claude` works identically with BetCode**. Updates to Claude Code are automatically available.

---

## Architecture

```
                          EXTERNAL SYSTEMS
  +-----------------------------------------------------------------------+
  |  +------------------+     +------------------+    +----------------+  |
  |  | Anthropic API    |     |   GitLab API     |    |  MCP Servers   |  |
  |  | (Claude models)  |     | (MRs, pipelines) |    | (spawned by CC)|  |
  +--+--------+---------+-----+--------+---------+----+-------+--------+-+
               |                        |                      |
  +------------|----- BETCODE ----------|----------------------|---------+
  |  +---------+------------------------+----------------------+------+  |
  |  |                        betcode-daemon                          |  |
  |  |  [Claude Code subprocess #1] [Claude Code subprocess #2] ...  |  |
  |  |  Session Multiplexer | Worktree Manager | GitLab Client       |  |
  |  +-----+-----------------------------+--------------------------+   |
  |        | local socket                | mTLS tunnel                  |
  |  +-----+------+              +-------+---------+                    |
  |  | betcode-cli |              | betcode-relay   |                    |
  |  | (ratatui)   |              | (gRPC router)   |                    |
  |  +-------------+              +-------+---------+                    |
  +---------------------------------------|-----------------------------+
                                          | TLS + JWT
                                +---------+---------+
                                |   betcode_app     |
                                |  (Flutter mobile) |
                                +-------------------+
```

### Components

| Component | Language | Role |
|---|---|---|
| **betcode-daemon** | Rust (tokio + tonic) | Spawns Claude Code subprocesses, bridges NDJSON to gRPC, multiplexes sessions, manages worktrees |
| **betcode-relay** | Rust (tokio + tonic) | Public gRPC router with JWT + mTLS auth, routes traffic to machines, buffers messages for offline daemons |
| **betcode-cli** | Rust (clap + ratatui) | Terminal TUI client, streaming markdown, permission prompts, headless mode |
| **betcode_app** | Flutter (Dart) | Mobile/web client, conversation UI, tool cards, offline queue |

### Design Decisions

- **Wrapper, not rewrite** -- Claude Code runs as a black-box subprocess. Zero agent maintenance, automatic feature parity with every `claude` update.
- **gRPC everywhere** -- Bidirectional streaming for real-time agent output. Strong typing across Rust and Dart.
- **Daemon as multiplexer** -- Multiple clients can observe or interact with the same session.
- **Relay as pure router** -- No API keys, no AI workload. Cheap to host, horizontally scalable.
- **SQLite for storage** -- Embedded, zero-config, cross-platform. WAL mode for concurrency.
- **No PTY emulation** -- Structured JSON events via `--output-format stream-json`, clean data/presentation separation.

---

## Planned Workspace Structure

```
betcode/
├── Cargo.toml                    # Workspace root
├── proto/betcode/v1/             # Shared protobuf definitions
├── crates/
│   ├── betcode-proto/            # Generated protobuf code (tonic-build)
│   ├── betcode-core/             # Shared types, config parsing, errors
│   ├── betcode-daemon/           # Daemon binary
│   ├── betcode-relay/            # Relay server binary
│   └── betcode-cli/              # CLI client binary
├── betcode_app/                  # Flutter mobile/web app
└── docs/architecture/            # Architecture documentation
```

---

## Prerequisites

- **Claude Code** must be installed on each machine that runs the daemon. BetCode does not bundle or replace it.
- **Rust** (stable, edition 2024) for building the daemon, relay, and CLI.
- **Flutter** (optional) for building the mobile/web client.
- **Anthropic API key** (`ANTHROPIC_API_KEY` env var) -- same as Claude Code.

---

## Roadmap

### Phase 1: Foundation -- Single Machine CLI
Daemon + CLI on one machine. NDJSON subprocess bridge, permission forwarding, session management. 4 Rust crates, ~8,000-12,000 LOC.

### Phase 2: Worktrees and Session Management
Git worktree orchestration, per-worktree sessions, input locking, multi-session support.

### Phase 3: Relay and Mobile
Remote access via self-hosted relay with JWT + mTLS auth. Flutter mobile/web app with offline sync.

### Phase 4: Multi-Machine and Polish
Cross-machine switching, GitLab integration, LAN discovery via mDNS, push notifications, production hardening.

See [docs/architecture/ROADMAP.md](docs/architecture/ROADMAP.md) for the full plan.

---

## Documentation

Detailed architecture documentation lives in [`docs/architecture/`](docs/architecture/):

| Document | Description |
|---|---|
| [OVERVIEW.md](docs/architecture/OVERVIEW.md) | System overview, C4 diagrams, tech stack |
| [DAEMON.md](docs/architecture/DAEMON.md) | Daemon internals, subprocess management |
| [PROTOCOL.md](docs/architecture/PROTOCOL.md) | Protocol layer reference |
| [PROTOCOL_L1.md](docs/architecture/PROTOCOL_L1.md) | Claude SDK NDJSON protocol |
| [PROTOCOL_L2.md](docs/architecture/PROTOCOL_L2.md) | BetCode gRPC API (proto definitions) |
| [PROTOCOL_BRIDGE.md](docs/architecture/PROTOCOL_BRIDGE.md) | Protocol bridging, reconnection |
| [TOPOLOGY.md](docs/architecture/TOPOLOGY.md) | Network topology, relay architecture |
| [CLIENTS.md](docs/architecture/CLIENTS.md) | Flutter and CLI client architecture |
| [SCHEMAS.md](docs/architecture/SCHEMAS.md) | SQLite schema designs |
| [SECURITY.md](docs/architecture/SECURITY.md) | Auth, authorization, sandboxing |
| [SUBAGENTS.md](docs/architecture/SUBAGENTS.md) | Multi-agent orchestration, DAG scheduling |
| [ROADMAP.md](docs/architecture/ROADMAP.md) | Phased implementation plan |

---

## Config Compatibility

BetCode is a drop-in wrapper. Claude Code runs as a subprocess in its normal mode and reads all project config files itself:

- `CLAUDE.md`, `CLAUDE.local.md`
- `.claude/settings.json`, `.claude/settings.local.json`
- `.claude/rules/*.md`, `.claude/skills/*/SKILL.md`
- `.claude/commands/*.md`, `.claude/agents/*.md`
- `.mcp.json`

No migration or config conversion is needed.

---

## Technology Stack

| Layer | Technology | Purpose |
|---|---|---|
| Daemon & Relay | Rust (tokio + tonic) | Async I/O, gRPC, performance |
| CLI | Rust (clap + ratatui) | Native TUI, fast startup |
| Mobile/Web | Flutter (Dart) | Cross-platform client |
| Protocol | gRPC (protobuf v3) | Bidirectional streaming, strong typing |
| Storage | SQLite (sqlx / drift) | Embedded, zero-config, cross-platform |
| Agent Backend | Claude Code CLI | Full agent fidelity via subprocess |

---

## License

Licensed under the [Apache License 2.0](LICENSE).

```
Copyright 2026 Konstantin Sazhenov
```

---

## Disclaimer

This project is **not** affiliated with, endorsed by, or sponsored by [Anthropic](https://www.anthropic.com/). "Claude" and "Claude Code" are trademarks or products of Anthropic, PBC. BetCode is an independent open-source project that wraps the publicly available Claude Code CLI. Use of Claude Code is subject to Anthropic's own terms of service.
