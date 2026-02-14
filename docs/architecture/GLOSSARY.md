# BetCode Glossary

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14

This glossary defines BetCode-specific terminology to ensure consistent usage
across all documentation.

---

## Core Concepts

### Subprocess
A child process spawned by the daemon. Specifically refers to the `claude` CLI
process that the daemon manages. One subprocess per active session.

**Usage**: "The daemon spawns a subprocess for each session."

**Not**: The daemon itself is not a subprocess (it's a daemon/service).

### Subagent
An independent Claude Code subprocess created for parallel task execution via the
`SubagentService` API. Subagents have their own session, working directory, and
permission context.

**Usage**: "The orchestrator spawned three subagents to work in parallel."

**Distinct from**: Claude Code's internal `Task` tool also creates "subagents" but
these run within a single subprocess. BetCode documentation uses "internal subagent"
or "Task subagent" for these, and "daemon subagent" or just "subagent" for
daemon-orchestrated ones.

### Session
A conversation context managed by the daemon. Each session has:
- A unique ID (from Claude's `system.init` message)
- Associated messages stored in SQLite
- At most one active subprocess at a time
- Optional worktree binding

**Usage**: "Resume the previous session" or "Create a new session."

**Lifecycle**: `NEW -> ACTIVE -> IDLE -> CLOSED`

### Turn
A single request-response cycle within a session. Begins with a `UserMessage`,
ends with `TurnComplete`. A session contains multiple turns.

**Usage**: "The agent completed the turn in 45 seconds."

### Worktree
A git worktree managed by the daemon, providing an isolated working directory with
its own branch checkout. Created via `git worktree add`.

**Usage**: "Create a worktree for the feature branch."

**Not**: The main repository checkout is not called a worktree in BetCode context
(though git considers it one).

### Machine
A developer workstation running the BetCode daemon. Identified by `machine_id`
(from mTLS certificate CN). One daemon per machine.

**Usage**: "Switch to your desktop machine."

---

## Protocol Terms

### NDJSON
Newline-Delimited JSON. The wire format between the daemon and Claude subprocess.
Each line is a complete JSON object.

**Usage**: "Parse the NDJSON stream from stdout."

### Tunnel
The persistent bidirectional gRPC stream between daemon and relay (`TunnelService.OpenTunnel`).
Initiated by the daemon (outbound) to traverse NAT/firewalls.

**Usage**: "The daemon maintains a tunnel to the relay."

### Input Lock
The exclusive right to send user input (messages, permission responses) to a session.
Only one client per session holds the lock. Others observe in read-only mode.

**Usage**: "Request the input lock to take control."

### Sequence Number
A monotonically increasing integer assigned to each `AgentEvent` within a session.
Used for reconnection replay and deduplication.

**Usage**: "Replay events starting from sequence 500."

---

## Architecture Terms

### Daemon
The `betcode-daemon` process running on each developer machine. Manages Claude
subprocesses, serves gRPC, and maintains session state.

**Usage**: "Start the daemon" or "The daemon crashed."

### Relay
The `betcode-relay` server running on public infrastructure. Routes client requests
to daemons via tunnels. Handles authentication and message buffering.

**Usage**: "Connect through the relay."

### Client
An application that connects to the daemon (directly or via relay) to interact with
Claude. BetCode provides two clients: CLI (ratatui) and Flutter app.

**Usage**: "The Flutter client shows the conversation."

### Multiplexer
The daemon component that fans out Claude subprocess output to multiple connected
clients simultaneously.

**Usage**: "The multiplexer broadcasts events to all clients."

### Permission Bridge
The daemon component that intercepts `control_request` messages from Claude,
evaluates them against permission rules, and either auto-responds or forwards
to the client.

**Usage**: "The permission bridge auto-allowed the read operation."

---

## Permission Terms

### Permission Grant
A user decision to allow or deny a tool execution, persisted for the session
duration. Stored in `permission_grants` table.

**Usage**: "The session has a grant for Bash(git *)."

### Auto-Approve
A subagent configuration (`auto_approve_permissions = true`) that automatically
allows tool executions without user interaction. Requires explicit `allowed_tools`.

**Usage**: "The subagent runs with auto-approve for Read and Grep."

### Permission Rule
A pattern-based rule that auto-allows or auto-denies tool executions.
Format: `"ToolName(pattern)"`.

**Usage**: "Add a permission rule for Bash(cargo *)."

---

## State Terms

### IDLE
Session state when no Claude subprocess is running. The session can be resumed
by sending a new message.

### ACTIVE
Session state when a Claude subprocess is running and processing a turn.

### COMPACTING
Session state during context compaction. No new messages accepted until complete.

### DEGRADED
System health state when non-critical components are unavailable but the system
remains functional.

---

## Abbreviations

| Abbreviation | Full Term |
|--------------|-----------|
| mTLS | Mutual TLS (both sides present certificates) |
| JWT | JSON Web Token |
| TTL | Time To Live |
| DAG | Directed Acyclic Graph |
| RPC | Remote Procedure Call |
| NDJSON | Newline-Delimited JSON |
| FCM | Firebase Cloud Messaging |
| APNs | Apple Push Notification service |
| WAL | Write-Ahead Logging (SQLite) |
| TUI | Text User Interface |
| CSR | Certificate Signing Request |

---

## Related Documents

| Document | Description |
|----------|-------------|
| [OVERVIEW.md](./OVERVIEW.md) | System overview, C4 diagrams, tech stack |
| [DAEMON.md](./DAEMON.md) | Daemon internals, subprocess management |
| [PROTOCOL_L1.md](./PROTOCOL_L1.md) | Claude SDK stream-json protocol |
| [PROTOCOL_L2.md](./PROTOCOL_L2.md) | BetCode gRPC API definitions |
| [SUBAGENTS.md](./SUBAGENTS.md) | Multi-agent orchestration |
