# BetCode Daemon Architecture

**Version**: 0.2.0
**Last Updated**: 2026-02-01
**Status**: Design Phase

## Overview

The betcode-daemon is the core orchestration component. It runs as a
persistent background service on each developer machine.

**The daemon does NOT implement an agent engine.** It spawns Claude Code
CLI (`claude`) as a child process and bridges its NDJSON stdio protocol
to gRPC clients. All AI reasoning, tool execution, and conversation
management happen inside the Claude Code process itself. The daemon is
a supervisor and multiplexer.

```
+--------------+       +-----------------+       +----------------+
| Flutter/CLI  | gRPC  | betcode-daemon  | stdio | claude (CLI)   |
| (client)     |<----->| (Rust, tonic)   |<----->| (subprocess)   |
|              |       |                 |       |                |
| Renders UI   |       | Multiplexes I/O |       | Agent engine   |
| Handles input|       | Stores history  |       | Tool execution |
| Offline queue|       | Manages sessions|       | Permission eval|
+--------------+       +-----------------+       +----------------+
```

**Why subprocess delegation**: (1) guaranteed parity with Claude Code,
(2) `npm update` upgrades capabilities without daemon changes,
(3) reduced surface area -- the daemon stays small and auditable,
(4) correctness by construction -- Anthropic maintains the agent loop.

## Crate Structure

```
betcode-daemon/src/
  main.rs                    # Entry point, signal handling, tokio runtime
  server/
    grpc.rs                  # tonic gRPC service implementations
    local.rs                 # Named pipe (Windows) / Unix socket for local CLI
    tunnel.rs                # Reverse tunnel to relay (mTLS, bidi stream)
  subprocess/
    process.rs               # Claude subprocess lifecycle (spawn, kill, restart)
    protocol.rs              # NDJSON stream-json parser/writer
    multiplexer.rs           # Fan-out events to connected clients
    permission_bridge.rs     # Bridge control_request to gRPC PermissionRequest
  session/
    manager.rs               # Session state, input lock, client tracking
    store.rs                 # SQLite persistence for messages/sessions
  worktree/
    manager.rs               # Git worktree lifecycle (create/switch/remove)
    setup.rs                 # Auto-setup (dependency install, env hooks)
  config/
    resolver.rs              # Config resolution hierarchy (Claude Code compat)
    import.rs                # First-run ~/.claude/ -> config dir import
  gitlab/
    client.rs                # GitLab API client (optional feature)
  storage/
    sqlite.rs                # Connection pool and migrations
```

## Claude Subprocess Management

### Spawn command

One `claude` process per active session:

```
claude -p "$prompt" \
  --output-format stream-json \
  --input-format stream-json \
  --permission-prompt-tool stdio \
  --include-partial-messages \
  --resume $session_id \
  --model $model \
  --allowedTools $pre_approved_tools
```

Flags: `-p` = initial prompt, `stream-json` = NDJSON on stdin/stdout,
`--permission-prompt-tool stdio` = route permission prompts through
stdio instead of TTY, `--include-partial-messages` = stream token
deltas, `--resume` = continue existing session, `--allowedTools` =
pre-approved tool list (comma-separated).

### Process lifecycle

```
IDLE (no process)
  | client sends message
  v
SPAWNING --> RUNNING --> EXITED
              |  |         |
              |  +-- stdout: NDJSON --> multiplexer --> gRPC clients
              |  +-- stdin:  NDJSON <-- user messages, control_response
              |
              +-- exit 0 (turn complete) --> IDLE
              +-- exit != 0 (crash)      --> RESTARTING (--resume)
```

Working directory: resolved from the session's associated worktree.
Environment: inherited from daemon, must include `ANTHROPIC_API_KEY`.
Windows: uses `CREATE_NO_WINDOW` to suppress console flash.

### Crash recovery

Non-zero exit: log stderr, backoff (500ms doubling, cap 30s), respawn
with `--resume $session_id`. Claude Code's internal session store
preserves conversation state. After 5 crashes in 60s, stop retrying
and notify clients with `CRASHED` status.

## NDJSON Protocol

### Messages from claude stdout

| Type | Description |
|------|-------------|
| `system` | Session init: `session_id`, `tools`, `model` |
| `content_block_delta` | Streaming text or tool input delta |
| `content_block_start/stop` | Content block boundaries |
| `message_start/delta/stop` | Message-level events, `stop_reason`, `usage` |
| `result` | Final turn result with `cost_usd` and `usage` |
| `control_request` | Permission prompt or user question (requires `control_response` on stdin) |

### Control messages

`control_request` from claude (permission):
```json
{"type":"control_request","request_id":"req_001",
 "request":{"subtype":"can_use_tool","tool_name":"Bash",
  "input":{"command":"git push"}}}
```

`control_request` from claude (user question):
```json
{"type":"control_request","request_id":"req_002",
 "request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion",
  "input":{"questions":[{"question":"Which branch?",
   "options":["main","develop"],"multi_select":false}]}}}
```

`control_response` to claude (allow):
```json
{"type":"control_response",
 "response":{"subtype":"success","request_id":"req_001",
  "response":{"behavior":"allow"}}}
```

`control_response` to claude (deny):
```json
{"type":"control_response",
 "response":{"subtype":"success","request_id":"req_001",
  "response":{"behavior":"deny","message":"User denied permission."}}}
```

User message to claude stdin:
```json
{"type":"user",
 "message":{"role":"user","content":"Now fix the failing tests."},
 "session_id":"abc123-def456-..."}
```

See [PROTOCOL_L1.md](./PROTOCOL_L1.md) for the complete message schema reference.

## Multiplexer

Reads every NDJSON line from claude stdout, then in parallel:
(a) persists to SQLite with monotonic sequence number,
(b) translates to gRPC `AgentEvent` and broadcasts to all subscribed clients.

```
claude stdout --> subprocess/protocol.rs (parse) --> subprocess/multiplexer.rs
                                            |-> store.rs (SQLite INSERT)
                                            |-> client_1 (gRPC stream)
                                            |-> client_2 (gRPC stream)
```

Translation: `content_block_delta` -> `TextDelta`, `content_block_start`
(tool_use) -> `ToolCallStart`, `control_request` -> `PermissionRequest`
or `UserQuestion`, `result` -> `UsageReport`, `system` -> `SessionInfo`.

**Client attachment**: New client connects -> replay full history from
SQLite -> switch to live stream. Every client sees the full conversation.

**Backpressure**: Bounded channel per client (1024 events). Overflow marks
client as `lagging`; on catchup, client resyncs from SQLite. The claude
process is never blocked by slow clients.

## Permission Bridge

Claude's `--permission-prompt-tool stdio` emits permissions as
`control_request` on stdout. The daemon pre-screens before forwarding:

1. **Deny rules** match -> write deny to claude stdin immediately
2. **Allow rules** match -> write allow to claude stdin immediately
3. **Session grants** match -> write allow immediately
4. **No rule** -> forward `PermissionRequest` to client with input lock,
   wait for `PermissionResponse`, write `control_response` to claude

Rule format (Claude Code native):
`"Bash(git *)"`, `"Edit(src/**/*.ts)"`, `"mcp__github__*"`, `"Bash"`

The daemon does NOT execute hooks or parse CLAUDE.md. Claude Code handles
both internally. The daemon only reads permission rules for pre-screening.

## Session Management

### State: `NEW -> ACTIVE -> IDLE -> CLOSED`

`ACTIVE`: claude process running, I/O flowing.
`IDLE`: no process, clients can connect (history from SQLite), next
message triggers new spawn.

### Input lock

One client at a time sends input. Transfer mechanisms:
- **Automatic**: lock holder disconnects -> next waiting client gets it
- **Explicit**: `RequestInputLock` RPC, current holder has 10s to respond
- **Timeout**: no input for 5 min with waiters -> lock offered to next

### Persistence

All messages persisted to SQLite via the multiplexer. Serves history
replay, crash recovery (daemon restart), and session resume.

Tables: `sessions` (metadata, usage), `messages` (role, JSON content,
sequence), `permission_grants` (session-scoped allow decisions).

## Worktree Orchestration

```
betcode worktree create feature/auth
  -> git worktree add ../betcode-feature-auth feature/auth
  -> run setup hooks (npm install, cargo build, etc.)
  -> register in SQLite (id, path, branch)
  -> new sessions spawn claude with cwd = worktree path
```

| Operation | Effect |
|-----------|--------|
| `create <branch>` | `git worktree add`, run setup, register |
| `switch <id>` | Update active worktree, spawn claude in new dir |
| `remove <id>` | Close sessions, `git worktree remove`, deregister |

Setup hooks: `.betcode/worktree-setup.sh` or `settings.json`:
```json
{"worktree":{"setup_commands":["npm install","cargo build"]}}
```

Each worktree can have multiple sessions. Each session belongs to one
worktree. Switching worktrees resumes or creates a session in the target.

## Daemon Lifecycle

```
START -> load config -> first-run import check -> init SQLite (migrations)
  -> restore session state -> start servers:
     +-- local gRPC (socket/pipe)
     +-- relay tunnel (if configured, mTLS)
     +-- health monitor
  -> RUNNING (accept connections, spawn claude on demand)

SHUTDOWN (SIGTERM/SIGINT) -> stop accepting connections
  -> signal claude processes (SIGTERM, wait 5s, SIGKILL)
  -> flush SQLite -> notify clients -> close tunnel -> close server -> EXIT
```

SIGHUP = reload config without restart.

## Startup Reconciliation

On startup, the daemon reconciles stale state left by a previous unclean
shutdown (crash, SIGKILL, power loss):

1. **Sessions**: All rows with `status = 'active'` are moved to `'idle'`.
   No Claude subprocess is running after a daemon restart — the session
   resumes on the next client message via `--resume`.
2. **Connected clients**: All rows in `connected_clients` are deleted.
   No gRPC streams survive a daemon restart.
3. **Input locks**: `sessions.input_lock_client` is set to NULL for all rows.
   Locks are re-acquired when clients reconnect.
4. **Worktrees**: Filesystem paths in `worktrees` are validated. Rows
   pointing to non-existent directories are marked with a `stale` flag
   (logged as warnings, not auto-deleted).
5. **Subagents**: All rows in `subagents` with `status IN ('pending', 'running')`
   are moved to `'failed'` with `result_summary = 'daemon restarted'`.
   Orchestrations are similarly failed.

This reconciliation runs inside a single SQLite transaction before the
daemon begins accepting connections.

## Config Resolution

Hierarchy (highest to lowest):

1. CLI flags (`--model`, `--allowedTools`)
2. `.claude/settings.local.json` (personal project, gitignored)
3. `.claude/settings.json` (team project, committed)
4. `$BETCODE_CONFIG_DIR/settings.json` (user global)

**Config directory** (resolved in order):
- `$BETCODE_CONFIG_DIR` env var (explicit override)
- Linux: `$XDG_CONFIG_HOME/betcode` (default `~/.config/betcode`)
- macOS: `~/Library/Application Support/betcode`
- Windows: `%USERPROFILE%\.betcode`

First-run: if config directory missing, offer import from `~/.claude/`
(settings.json, rules/). Copies, not symlinks, so both tools run without
interference.

## Local Server Transport

- **Unix**: `/run/user/$UID/betcode/daemon.sock` (fallback: `$BETCODE_CONFIG_DIR/daemon.sock`)
- **Windows**: `\\.\pipe\betcode-daemon-$USERNAME`
- **Discovery**: CLI checks socket/pipe; if absent, offers `betcode daemon start`

## Error Handling

| Scenario | Response |
|----------|----------|
| Claude exits 0 | Normal; session -> IDLE |
| Claude exits non-zero | Restart with --resume, backoff |
| Claude hangs (5 min no output) | SIGTERM -> SIGKILL -> restart |
| Invalid JSON on stdout | Log warning, skip line, continue |
| Client stream drops | Remove from multiplexer, release input lock |
| All clients disconnect | Claude continues until turn completes |
| SQLite write failure | Log error, continue streaming (degrade) |

## Observability

The daemon emits structured telemetry for debugging, performance analysis,
and operational monitoring. All telemetry is opt-in and local-first.

### Structured Logging

JSON-formatted log lines to stderr (or a configured file sink) via the
`tracing` crate with `tracing-subscriber`. Log levels:

| Level | Content |
|-------|---------|
| ERROR | Subprocess crashes, SQLite failures, unrecoverable errors |
| WARN  | Permission denials, reconnection attempts, stale state |
| INFO  | Session lifecycle, client connections, worktree operations |
| DEBUG | NDJSON message flow, permission engine decisions |
| TRACE | Raw NDJSON lines, gRPC frame details |

Every log entry includes: `timestamp`, `level`, `session_id` (if applicable),
`span` (hierarchical context), and `target` (module path).

### OpenTelemetry Integration

Optional OpenTelemetry export via `tracing-opentelemetry` and `opentelemetry-otlp`.
Enabled by config (`observability.otlp_endpoint`) or env var (`OTEL_EXPORTER_OTLP_ENDPOINT`).

**Traces**: Each agent turn is a root span. Child spans for: subprocess spawn,
NDJSON parse, permission bridge evaluation, tool execution (as observed by
the daemon), gRPC event broadcast, SQLite writes.

**Metrics** (exported as OTLP or exposed via Prometheus endpoint):

| Metric | Type | Description |
|--------|------|-------------|
| `betcode_sessions_active` | Gauge | Currently active sessions |
| `betcode_subprocess_spawns_total` | Counter | Claude process spawn count |
| `betcode_subprocess_crashes_total` | Counter | Non-zero exit count |
| `betcode_messages_processed_total` | Counter | NDJSON lines processed |
| `betcode_permission_decisions_total` | Counter | By decision type (auto_allow, auto_deny, user_allow, user_deny, timeout) |
| `betcode_grpc_clients_connected` | Gauge | Connected client count |
| `betcode_turn_duration_seconds` | Histogram | End-to-end turn latency |
| `betcode_tokens_total` | Counter | By direction (input, output, cache_read) |

**Disabled by default**. No telemetry data leaves the machine unless
explicitly configured. The daemon never sends telemetry to Anthropic
or any third party.

## Subagent Orchestration

The daemon supports spawning multiple independent Claude Code subprocesses
for parallel task execution. This enables building external orchestrators
(separate projects) that coordinate multiple agents working on different
aspects of a feature simultaneously, each in its own worktree.

See [SUBAGENTS.md](./SUBAGENTS.md) for the full SubagentService gRPC API,
DAG scheduler, subprocess pool, permission delegation, and the external
orchestrator integration pattern.

## Related Documents

- [OVERVIEW.md](./OVERVIEW.md) -- system overview, C4 diagrams
- [PROTOCOL.md](./PROTOCOL.md) -- Claude SDK protocol, gRPC API
- [TOPOLOGY.md](./TOPOLOGY.md) -- relay and connection modes
- [SCHEMAS.md](./SCHEMAS.md) -- SQLite table definitions
- [SECURITY.md](./SECURITY.md) -- auth, sandboxing, resilience
- [CLIENTS.md](./CLIENTS.md) -- Flutter and CLI client architecture
- [SUBAGENTS.md](./SUBAGENTS.md) -- multi-agent orchestration
