# BetCode Implementation Workflow

**Version**: 0.1.0 | **Generated**: 2026-02-03 | **Total LOC**: ~20,000

## Current State

- **Documentation**: Complete (42 architecture docs)
- **Implementation**: None (design phase only)
- **Critical Path**: Proto → Core → Daemon → CLI

## Phase Overview

| Phase | Weeks | Components | LOC | Parallel? |
|-------|-------|------------|-----|-----------|
| **P0: Scaffold** | 1 day | Workspace, Cargo.toml | 50 | No |
| **P1: Foundation** | 1-6 | Proto, Core, Daemon, CLI | 8,000 | Limited |
| **P2: Worktrees** | 7-8 | Git worktree manager | 1,000 | Yes |
| **P3: Relay+Mobile** | 9-14 | Relay, Flutter app | 8,000 | 3 tracks |
| **P4: Polish** | 15-16 | GitLab, observability | 2,000 | Yes |

## Phase 1: Foundation (Critical Path)

```
Sprint 1.1: Proto (Week 1)
├── agent.proto (150 LOC) → gRPC service definitions
├── config.proto (50 LOC) → settings messages
└── betcode-proto crate → tonic-build setup

Sprint 1.2: Core Library (Week 2)
├── ndjson.rs (400 LOC) → Claude stream-json parser
├── config.rs (300 LOC) → config hierarchy resolution
├── permissions.rs (250 LOC) → rule matching engine
└── error.rs (100 LOC) → error catalog

Sprint 1.3: Daemon Foundation (Weeks 3-4)
├── subprocess/manager.rs (600 LOC) → process lifecycle
├── subprocess/bridge.rs (400 LOC) → NDJSON → events
├── storage/*.rs (500 LOC) → SQLite persistence
├── server/local.rs (400 LOC) → Unix socket gRPC
└── session/multiplexer.rs (350 LOC) → event fan-out

Sprint 1.4: Permission Bridge (Week 5)
├── permission/engine.rs (400 LOC) → decision flow
└── permission/pending.rs (200 LOC) → pending map + TTL

Sprint 1.5: CLI Foundation (Week 6)
├── connection.rs (200 LOC) → daemon connection
├── ui/layout.rs (400 LOC) → ratatui TUI
├── ui/stream.rs (300 LOC) → streaming display
├── ui/permission.rs (250 LOC) → permission dialog
└── headless.rs (150 LOC) → non-interactive mode
```

**Validation Gate**: Full conversation flow works end-to-end

## Phase 3: Parallel Tracks (Concurrency: 3)

```
Track A: Relay Server (Rust)
├── Connection registry
├── Tunnel protocol (bidi gRPC)
├── Message buffer (7-day TTL)
├── JWT + mTLS auth
└── Push notifications (FCM/APNs)

Track B: Daemon Tunnel Client (Rust)
├── Outbound mTLS connection
├── Persistent stream
└── Reconnection backoff

Track C: Flutter App (Dart)
├── Proto codegen
├── gRPC client + auth
├── Offline sync engine
├── Conversation UI
└── Machine switcher
```

## Agent Assignments

| Agent | Scope |
|-------|-------|
| `backend-architect` | Daemon, relay, proto, core |
| `frontend-architect` | CLI TUI, Flutter app |
| `security-engineer` | Permissions, auth, mTLS |
| `devops-architect` | Worktrees, observability |

## Validation Gates

| Gate | Criteria |
|------|----------|
| P1.1 | Proto compiles |
| P1.2 | Core tests pass (90%+) |
| P1.3 | Daemon spawns Claude |
| P1.5 | CLI conversation works |
| P3 | Mobile connects via relay |

## Next Steps

1. Create branch: `git checkout -b feature/phase-1-foundation`
2. Start Sprint 1.1: Proto definitions
3. See [WORKFLOW_DETAILS.md](./WORKFLOW_DETAILS.md) for full task breakdown

## Related Docs

- [OVERVIEW.md](./architecture/OVERVIEW.md) - System design
- [DAEMON.md](./architecture/DAEMON.md) - Daemon architecture
- [PROTOCOL_L1.md](./architecture/PROTOCOL_L1.md) - NDJSON protocol
- [ROADMAP.md](./architecture/ROADMAP.md) - Phase overview
