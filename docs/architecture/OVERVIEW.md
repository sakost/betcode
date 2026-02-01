# BetCode Architecture Overview

**Version**: 0.2.0
**Last Updated**: 2026-02-01
**Status**: Design Phase

---

## Foundational Decision: Wrapper, Not Rewrite

BetCode wraps Claude Code (Anthropic's official CLI) as a subprocess rather than
reimplementing its agent engine.

The daemon spawns `claude` with `--output-format stream-json`,
`--input-format stream-json`, and `--permission-prompt-tool stdio`. All agent
intelligence runs inside Claude Code. BetCode communicates via a bidirectional
NDJSON control protocol and builds orchestration, transport, and UI around it.

**BetCode owns**: subprocess lifecycle, session multiplexing, gRPC transport,
relay routing, worktree orchestration, subagent orchestration, TUI/mobile UI,
offline queueing, relay auth, GitLab integration, session persistence.

**Claude Code owns** (via subprocess): agent loop, all tools, MCP lifecycle,
hooks execution, context summarization, CLAUDE.md resolution, plan mode,
skills, permissions evaluation.

Every tool, prompt, permission rule, and MCP server that works with `claude` works
identically with BetCode.

---

## C4 Context Diagram

```
                          EXTERNAL SYSTEMS
  +-----------------------------------------------------------------------+
  |  +------------------+     +------------------+    +----------------+  |
  |  | Anthropic API    |     |   GitLab API     |    |  MCP Servers   |  |
  |  | (Claude models)  |     | (MRs, pipelines) |    | (spawned by CC)|  |
  |  +--------+---------+     +--------+---------+    +-------+--------+  |
  |           ^                        ^                      ^           |
  |           | called by CC           | called by daemon     | by CC     |
  +-----------|------------------------|----------------------|-----------+
              |    BETCODE SYSTEM      |                      |
  +-----------|----- BOUNDARY ---------|----------------------|-----------+
  |  +--------+------------------------+----------------------+--------+  |
  |  |                        betcode-daemon                           |  |
  |  |  [Claude Code subprocess #1] [Claude Code subprocess #2] ...   |  |
  |  |  Session Multiplexer | Worktree Manager | GitLab Client        |  |
  |  +-----+-----------------------------+----------------------------+  |
  |        | local socket                | mTLS tunnel                   |
  |  +-----+------+              +-------+---------+                     |
  |  | betcode-cli |              | betcode-relay   |                     |
  |  | (ratatui)   |              | (gRPC router)   |                     |
  |  +-------------+              +-------+---------+                     |
  +---------------------------------------|------------------------------+
                                          | TLS + JWT
                                +---------+---------+
                                |   betcode_app     |
                                |  (Flutter mobile) |
                                +-------------------+
```

---

## C4 Container Diagram

| Container          | Binary           | Responsibility                                          |
|--------------------|------------------|---------------------------------------------------------|
| **betcode-daemon** | Rust binary      | Spawns Claude Code subprocesses, bridges NDJSON to gRPC, multiplexes sessions, manages worktrees, serves local + tunnel |
| **betcode-relay**  | Rust binary      | Public gRPC router, JWT + mTLS auth, routes to machines, buffers messages for offline daemons |
| **betcode-cli**    | Rust binary      | ratatui TUI, connects via local socket or relay, renders streaming output, permission prompts |
| **betcode_app**    | Flutter app      | Mobile/web client via relay, conversation UI, tool cards, permission dialogs, offline queue |
| **SQLite (daemon)**| Embedded         | Sessions, messages, worktrees, permission grants        |
| **SQLite (relay)** | Embedded         | Users, machines, certificates, message buffer           |
| **SQLite (client)**| Embedded         | Offline queue, cached sessions, machine bookmarks       |

---

## Core Design Decisions

### 1. Wrapper Over Rewrite
Claude Code as black-box agent backend via SDK subprocess protocol. Automatic
feature parity with every `claude` update. Trade-off: hard dependency on `claude`
binary, cannot customize agent internals.

### 2. gRPC Everywhere
Bidirectional streaming for real-time agent output. Strong typing across Rust
(tonic) and Dart (grpc). HTTP/2 multiplexing. Trade-off: requires gRPC-Web proxy
for Flutter web.

### 3. Daemon as Multiplexer
Owns all Claude subprocesses, fans out events to multiple clients. Enables
session continuity across devices. Trade-off: single point of failure per machine.

### 4. Relay as Pure Router
Zero AI workload, only routes gRPC traffic. Cheap to host, horizontally scalable.
No API keys or secrets on relay. Trade-off: daemon must be online for real-time
interaction; message buffer has 24h TTL.

### 5. SQLite for Storage
Embedded, zero-config, cross-platform. WAL mode for concurrency. Trade-off: relay
may need PostgreSQL migration at large scale.

### 6. No PTY Emulation
Structured JSON events via `--output-format stream-json`, not terminal bytes.
Clean data/presentation separation. Trade-off: dependent on Claude Code maintaining
the stream-json format.

---

## Technology Stack

| Component        | Technology            | Rationale                                |
|------------------|-----------------------|------------------------------------------|
| Daemon           | Rust (tokio + tonic)  | Performance, safety, native async + gRPC |
| Relay            | Rust (tokio + tonic)  | Same stack, lightweight                  |
| CLI              | Rust (clap + ratatui) | Native, fast, cross-platform             |
| Mobile/Web       | Flutter (Dart)        | Cross-platform mobile                    |
| Protocol         | gRPC (proto3)         | Bidirectional streaming, strong typing   |
| Storage          | SQLite (sqlx / drift) | Embedded, zero-config                    |
| Agent Backend    | Claude Code CLI       | Full fidelity via SDK subprocess protocol|
| Subprocess Proto | NDJSON (stream-json)  | Structured events, control req/resp      |

---

## Workspace Structure

```
betcode/
├── Cargo.toml                    # Workspace root
├── proto/betcode/v1/             # Shared protobuf definitions
│   ├── agent.proto               #   Core agent conversation service
│   ├── machine.proto             #   Multi-machine management
│   ├── worktree.proto            #   Git worktree management
│   ├── gitlab.proto              #   GitLab integration
│   ├── config.proto              #   Settings and configuration
│   └── tunnel.proto              #   Relay <-> Daemon communication
├── crates/
│   ├── betcode-proto/            # Generated protobuf code (tonic-build)
│   ├── betcode-core/             # Shared types, config parsing, errors
│   ├── betcode-daemon/           # Daemon binary
│   │   └── src/
│   │       ├── subprocess/       #   Claude Code process mgmt + NDJSON protocol
│   │       ├── session/          #   Session multiplexer + event fan-out
│   │       ├── server/           #   gRPC (local socket + relay tunnel)
│   │       ├── worktree/         #   Git worktree lifecycle
│   │       ├── gitlab/           #   GitLab API client
│   │       └── storage/          #   SQLite (sqlx)
│   ├── betcode-relay/            # Relay server binary
│   │   └── src/                  #   Router, tunnel, auth, buffer, storage
│   └── betcode-cli/              # CLI client binary
│       └── src/                  #   Commands, TUI (ratatui), connection
├── betcode_app/                  # Flutter mobile/web app
└── docs/architecture/            # Architecture documentation
```

Claude Code runs as a subprocess — the agent engine lives there, not in BetCode.

---

## Config Compatibility (Drop-In)

Claude Code runs as a subprocess in its normal mode, so it reads and resolves
all project config files itself. BetCode does not need to parse them.

**Project level (read by Claude Code subprocess directly):**
`CLAUDE.md`, `CLAUDE.local.md`, `.claude/settings.json`,
`.claude/settings.local.json`, `.claude/rules/*.md`, `.claude/skills/*/SKILL.md`,
`.claude/commands/*.md`, `.claude/agents/*.md`, `.mcp.json`

**User level (isolated copy in BetCode config directory):**
`$config_dir/settings.json`, `$config_dir/rules/*.md`
-- copied from `~/.claude/` on first run to avoid state corruption.
See [DAEMON.md](./DAEMON.md) for platform-specific config directory paths.

**Auth:** `ANTHROPIC_API_KEY` and `ANTHROPIC_BASE_URL` env vars (same as Claude
Code), plus BetCode-specific relay auth (JWT + mTLS).

---

## Differentiators from Claude Code

| Capability               | Claude Code | BetCode              |
|--------------------------|-------------|----------------------|
| Agent intelligence       | Native      | Inherited (wrapper)  |
| Mobile client            | No*         | Flutter app          |
| Multi-machine access     | No*         | Relay + tunnel       |
| Git worktree management  | No          | First-class          |
| GitLab integration       | GitHub only | GitLab-native        |
| Offline queueing         | No          | Client-side SQLite   |
| Session multiplexing     | Single user | Multi-client         |
| Self-hosted relay        | No*         | Built-in             |

*Happy Engineering provides mobile/relay for Claude Code as a separate product.

---

## Architecture Document Index

| Document                         | Description                                            |
|----------------------------------|--------------------------------------------------------|
| [OVERVIEW.md](./OVERVIEW.md)    | This file. C4 diagrams, decisions, tech stack          |
| [DAEMON.md](./DAEMON.md)        | Daemon process, Claude subprocess mgmt, session mux    |
| [PROTOCOL.md](./PROTOCOL.md)    | Claude SDK stream-json protocol, gRPC API definitions  |
| [TOPOLOGY.md](./TOPOLOGY.md)    | Network topology, relay architecture, connection modes  |
| [CLIENTS.md](./CLIENTS.md)      | CLI (ratatui) and Flutter app architecture             |
| [SCHEMAS.md](./SCHEMAS.md)      | SQLite schemas for daemon, relay, client               |
| [SECURITY.md](./SECURITY.md)    | Auth layers, sandboxing, connection resilience          |
| [ROADMAP.md](./ROADMAP.md)      | Phased implementation plan (wrapper approach)          |
| [SUBAGENTS.md](./SUBAGENTS.md)  | Multi-agent orchestration, DAG scheduling, external API |

