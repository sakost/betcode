# BetCode Workflow - Task Details

**Parent**: [IMPLEMENTATION_WORKFLOW.md](./IMPLEMENTATION_WORKFLOW.md)

## Sprint 1.1: Proto Definitions

### Task 1.1.1: agent.proto
**Agent**: `backend-architect` | **LOC**: 150 | **Priority**: P0

```protobuf
service AgentService {
  rpc Converse(stream ClientMessage) returns (stream AgentEvent);
  rpc GetSession(GetSessionRequest) returns (Session);
}
```

Key messages: `UserMessage`, `AgentEvent`, `PermissionRequest`, `PermissionResponse`

### Task 1.1.2: config.proto
**Agent**: `backend-architect` | **LOC**: 50 | **Parallel**: Yes

Settings management: `GetConfig`, `DaemonConfig`, `SessionConfig`

### Task 1.1.3: betcode-proto crate
**Agent**: `backend-architect` | **Depends**: 1.1.1, 1.1.2

Setup `build.rs` with tonic-build for code generation.

---

## Sprint 1.2: Core Library

### Task 1.2.1: NDJSON Parser (Critical Path)
**Agent**: `backend-architect` | **LOC**: 400

Parse Claude Code stream-json format:
- `system.init`, `assistant.message`, `user.message`
- `control_request`, `control_response`
- Tolerant reader (ignore unknown fields)

### Task 1.2.2: Config Resolution
**Agent**: `backend-architect` | **LOC**: 300 | **Parallel**: Yes

Hierarchy: CLI flags > Env vars > Project local > Project > User > Defaults

### Task 1.2.3: Permission Rules
**Agent**: `security-engineer` | **LOC**: 250 | **Parallel**: Yes

Pattern matching: `Bash(git *)`, `Edit(src/**/*.ts)`, `mcp__*`

---

## Sprint 1.3: Daemon Foundation

### Task 1.3.1: Subprocess Manager (Critical Path)
**Agent**: `backend-architect` | **LOC**: 600

State machine: `IDLE → SPAWNING → RUNNING → EXITED_OK | CRASHED`

### Task 1.3.2: NDJSON Bridge
**Agent**: `backend-architect` | **LOC**: 400 | **Depends**: 1.3.1

Bridge stdout → internal events → multiplexer

### Task 1.3.3: Session Store
**Agent**: `backend-architect` | **LOC**: 500 | **Parallel**: Yes

SQLite with sqlx: sessions, messages, permission_grants tables

### Task 1.3.4: gRPC Server
**Agent**: `backend-architect` | **LOC**: 400 | **Parallel**: Yes

Unix socket (Linux/macOS), Named pipe (Windows)

### Task 1.3.5: Multiplexer
**Agent**: `backend-architect` | **LOC**: 350 | **Depends**: 1.3.2

Sequence numbers, client subscriptions, input lock arbitration

---

## Sprint 1.4: Permission Bridge

### Task 1.4.1: Permission Engine
**Agent**: `security-engineer` | **LOC**: 400

Flow: Check deny → Check allow → Check grants → Forward → Timeout

### Task 1.4.2: Pending Permissions
**Agent**: `backend-architect` | **LOC**: 200

TTL: 60s (connected), 7 days (disconnected), activity refresh

---

## Sprint 1.5: CLI Foundation

### Task 1.5.1-1.5.5: CLI Components
**Agent**: `frontend-architect` | **Total LOC**: 1,300

- Connection management
- Ratatui TUI layout
- Streaming display with markdown
- Permission dialogs
- Headless mode

---

## Phase 3: Parallel Tracks

### Track A: Relay Server
**Weeks**: 9-12 | **LOC**: 2,500

Core → Auth (mTLS/JWT) → Buffer → Push notifications

### Track B: Daemon Tunnel
**Weeks**: 9-10 | **LOC**: 600

Outbound mTLS, persistent stream, reconnection

### Track C: Flutter App
**Weeks**: 9-14 | **LOC**: 3,000

Proto → gRPC → Sync engine → Conversation UI → Machine switcher

---

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Claude format changes | Version detection, adapter pattern |
| Subprocess zombies | Health monitor, SIGTERM → SIGKILL |
| Permission deadlocks | Configurable timeout, activity refresh |
| SQLite corruption | WAL mode, integrity checks |
