# Implementation Roadmap

**Version**: 0.1.0-alpha.1
**Status**: Implemented
**Approach**: Wrapper (Claude Code as subprocess, not reimplemented)

---

## Architecture: Claude Code as Subprocess

BetCode treats Claude Code as an opaque subprocess. The daemon spawns `claude`
processes, reads NDJSON from stdout, writes to stdin, and bridges control events
(permissions, questions) to connected gRPC clients.

**Benefits**: 100% agent fidelity, automatic upgrades when Claude Code updates,
zero agent maintenance cost, small codebase (~10-15K LOC Rust).

**Trade-offs**: Requires Claude Code installed, one OS process per session, cannot
intercept tool-level behavior, no local-only model path through agent.

---

## Current Progress

**v0.1.0-alpha.1**

Phases 1-3 are complete and most of Phase 4 is implemented. All 8 Rust crates and the Flutter mobile app are functional and passing CI.

### Rust Crates

| Crate | Status | Description |
|-------|--------|-------------|
| betcode-proto | Done | 13 gRPC services, ~70 RPCs, tonic-build codegen |
| betcode-core | Done | Config parsing, NDJSON types, shared errors |
| betcode-crypto | Done | mTLS certificate generation, E2E encryption (X25519 + ChaCha20-Poly1305) |
| betcode-daemon | Done | Subprocess manager, session store, worktree manager, tunnel client, GitLab API |
| betcode-cli | Done | clap CLI, ratatui TUI, worktree/machine/gitlab/auth/repo commands |
| betcode-relay | Done | gRPC router, JWT auth, tunnel service, message buffer, user management |
| betcode-setup | Done | First-run setup wizard |
| betcode-releases | Done | Release artifact packaging |

### Flutter Mobile App ([betcode-app](https://github.com/sakost/betcode-app))

| Feature | Status |
|---------|--------|
| Riverpod + go_router + drift scaffold | Done |
| JWT auth (login, register, refresh) | Done |
| gRPC client with reconnection | Done |
| Conversation UI (streaming, tools, permissions, todos) | Done |
| Machine list + switching | Done |
| Session list + resume + rename | Done |
| Offline sync engine (priority queue, retry, TTL) | Done |
| GitLab screens (pipelines, MRs, issues) | Done |
| Worktree + repo management | Done |
| Settings screen | Done |
| Push notifications | Schema only |

### Phase Status

| Phase | Status | Notes |
|-------|--------|-------|
| Phase 1: Foundation | Complete | All 8 crates |
| Phase 2: Worktrees & Sessions | Complete | Worktrees, input locking, session lifecycle |
| Phase 3: Relay & Mobile | Complete | Relay, tunnel, Flutter app with offline sync |
| Phase 4: Multi-Machine & Polish | Mostly complete | Cross-machine and GitLab done |
| Subagent Orchestration | Partial | ListAgents only; spawn/watch/DAG not started |

---

## Responsibility Matrix

| Concern | Owner | Notes |
|---------|-------|-------|
| Agent loop, tools, context, MCP, hooks, sub-agents, skills, plan mode, CLAUDE.md | Claude Code | Opaque subprocess |
| Permission bridging | BetCode daemon | control_request -> gRPC client |
| Session multiplexing | BetCode daemon | Multiple clients per session |
| Worktree management | BetCode daemon | git worktree + auto-setup |
| Session persistence | BetCode daemon | SQLite + NDJSON archive |
| Remote access | BetCode relay | gRPC routing via reverse tunnel |
| Mobile client | BetCode Flutter | Full conversation UI |
| Desktop TUI | BetCode CLI | ratatui, streaming |
| Config resolution | BetCode daemon | Reads .claude/ natively |

---

## Phase 1: Foundation -- Single Machine CLI [Complete]

**Goal**: Daemon + CLI on one machine. Full agent capability via Claude Code subprocess.

**betcode-proto**: 13 gRPC services (~70 RPCs) including AgentService (Converse,
ListSessions, ResumeSession, CompactSession, CancelTurn, InputLock, KeyExchange,
SessionGrants, RenameSession), ConfigService, CommandService, WorktreeService,
GitRepoService, TunnelService, AuthService, MachineService, GitLabService,
PluginService, Health, BetCodeHealth, VersionService.

**betcode-core**: NDJSON stream parser (all Claude Code message types: assistant,
tool_use, tool_result, result, system, control_request, input_request), typed Rust
structs with serde, config types and resolution (.claude/settings.json hierarchy),
permission rule parsing (Claude Code "Tool(pattern)" format), shared error types.

**betcode-daemon**: Claude subprocess manager (spawn with --output-format stream-json,
--resume, stdin/stdout I/O bridge, health monitoring), permission bridge
(control_request -> auto-respond or forward to client, timeout 60s default), session
store (SQLite: sessions, messages, permission_grants), local gRPC server (Unix socket),
config resolver (merge hierarchy: CLI > env > project local > project > user),
daemon lifecycle (signal handling, graceful shutdown, PID file, tracing).

**betcode-cli**: clap parsing (chat, -p, --resume, --model, session, daemon
subcommands), ratatui TUI (input pane, streaming markdown output, tool call
indicators, permission prompt dialogs, status bar), local daemon connection
(auto-detect socket, offer to start), headless mode (-p with text streaming),
tab-completion (slash commands, agent mentions, file paths).

---

## Phase 2: Worktrees and Session Management [Complete]

**Goal**: Git worktree orchestration, multi-session support, input locking.

**Worktree manager**: WorktreeManager with create/remove/list/get, name validation
(path traversal protection), auto-setup script execution on creation with rollback
on failure, worktree modes (global, local, custom), .gitignore management for local
mode. GitRepoService for repository registration/scanning.

**Per-worktree sessions**: sessions.worktree_id FK with cascade cleanup,
bind_session_to_worktree/get_worktree_sessions queries, WorktreeDetail includes
session_count.

**Session lifecycle**: ResumeSession RPC with from_sequence replay, CompactSession
RPC (keeps newest 50%, minimum 10 messages), ListSessions with pagination and
working_directory filter, RenameSession RPC, session metadata (tokens, cost,
last_message_preview, status).

**Input locking**: SessionState.input_lock_holder with atomic DB transactions,
acquire_input_lock RPC, connected_clients table (client_id, session_id, client_type,
has_input_lock, last_heartbeat), auto-release on disconnect.

**CLI additions**: `betcode worktree create/list/get/remove`, `betcode repo
register/unregister/list/get/update`.

---

## Phase 3: Relay and Mobile [Complete]

**Goal**: Remote access via relay + Flutter mobile app.

**betcode-relay**: TunnelService (bidirectional streaming with frame routing),
JWT auth for clients (jsonwebtoken HMAC-SHA256, access + refresh tokens, Bearer
interceptor), server-side TLS (dev self-signed or custom certs), connection
registry (machine_id -> tunnel with dual-channel unary/streaming support), request
routing with request_id correlation and timeout enforcement, message buffering
(SQLite, 1hr TTL, priority ordering, auto-drain on reconnect), user
registration/login (argon2id password hashing), relay database (users, tokens,
machines, message_buffer, certificates tables).

**Daemon additions**: TunnelClient with reverse tunnel to relay, reconnection
state machine (exponential backoff 1s-60s), TunnelRequestHandler routing all
services through tunnel, E2E encryption (X25519 key exchange + ChaCha20-Poly1305),
HTTP/2 keepalive (30s interval, 10s timeout).

**Flutter app** ([betcode-app](https://github.com/sakost/betcode-app)):
Riverpod + go_router + drift + flutter_secure_storage scaffold, JWT auth with
token refresh and expiry detection, gRPC client with exponential backoff
reconnection (100ms-30s), conversation screen (streaming text deltas, tool call
cards with status/duration, permission bottom sheets, todo list panel with
progress, user question dialogs, plan mode banner, agent bar for multi-agent),
machine list + switching with persistence, session list + resume + rename +
caching, offline sync engine (priority queue with 7-day TTL, UUIDv7 idempotency,
max 5 retries with jitter), worktree and repo management screens, settings screen.

---

## Phase 4: Multi-Machine and Polish [In Progress]

**Goal**: Cross-machine switching, GitLab integration, LAN mode, production readiness.

**Cross-machine** [Done]: machine_id routing via relay, CLI `betcode machine
register/list/switch/status`, Flutter machine list + switching with persistence,
MachineService gRPC (RegisterMachine, ListMachines, RemoveMachine, GetMachine).

**GitLab integration** [Done]: GitLabService gRPC in daemon (ListMergeRequests,
GetMergeRequest, ListPipelines, GetPipeline, ListIssues, GetIssue), Flutter GitLab
tab (pipelines, MRs, issues -- read-only), CLI `betcode gitlab mr/pipeline/issue
list/get` commands.

**Remaining work**:

- **Direct LAN mode**: mDNS discovery, explicit config, mTLS reuse, automatic
  prefer LAN over relay.
- **Push notifications**: drift schema exists (NotificationCache table) but not
  wired to FCM/APNs or agent events.
- **Mutual TLS for daemons**: Server TLS exists but daemons authenticate via JWT,
  not client certificates. Certificates table exists in relay DB but unused for auth.
- **Configurable buffer TTL/cap**: Message buffer uses fixed 1hr TTL with no
  per-machine capacity cap (roadmap specified 24h TTL, 1000 msg cap).
- **Opt-in metrics**: Not started.
- **Session delete**: Flutter shows "coming soon" stub.
- **GitLab write operations**: Currently read-only (no create/edit/approve MRs).
- **CLI JSON output** (low priority for now): Headless mode outputs plain text
  only; no structured JSON/stream-json output format.

---

## Subagent Orchestration (Cross-Phase) [Partial]

BetCode exposes agent listing via CommandService and a plugin system for
extending daemon capabilities. Full subagent orchestration (spawning and
coordinating multiple Claude Code subprocesses) is planned but not yet
implemented.

**Implemented**:
- ListAgents RPC with AgentKind enum (ClaudeInternal, DaemonOrchestrated,
  TeamMember) and AgentStatus (Idle, Working, Done, Failed)
- Plugin infrastructure (PluginManager lifecycle, PluginService gRPC interface
  with Register/Execute/HealthCheck, socket-based communication)

**Remaining work**:
- SpawnSubagent / WatchSubagent RPCs for subprocess orchestration
- Team management (creation, member assignment, distributed sessions)
- OrchestrationPlan, DAG scheduler, context sharing
- Agent persistence (DB tables for agent metadata and session bindings)

See [SUBAGENTS.md](./SUBAGENTS.md) for the complete design.

---

## What We Do NOT Build

Claude Code handles internally: agent engine (ReAct), LLM provider abstraction, all
tool implementations (Read/Write/Edit/Bash/Glob/Grep/WebFetch/WebSearch/NotebookEdit/
Task/TodoWrite/ToolSearch/Skill), context summarization, MCP server lifecycle, hook
execution, sub-agent system, skills/commands dispatch, plan mode state machine,
CLAUDE.md loading and @import resolution, deferred tool loading, system prompt
construction. Every Claude Code update is automatically available to BetCode users.

## Scope Summary

| Metric | Value |
|--------|-------|
| Rust crates | 8 (proto, core, crypto, daemon, cli, relay, setup, releases) |
| gRPC services | 13 services, ~70 RPCs |
| Flutter app | Separate repo ([betcode-app](https://github.com/sakost/betcode-app)) |
| Agent fidelity risk | Zero (Claude Code handles it) |

## Risk Register

| Risk | Prob | Impact | Mitigation |
|------|------|--------|------------|
| NDJSON format changes | Med | High | Pin version, version-detect, adapter layer |
| --output-format stream-json removed | Low | Critical | Documented public interface; monitor releases |
| Subprocess overhead | Low | Med | Normal for dev workstations (1-5 sessions) |
| Permission bridge latency | Low | Med | Auto-respond for configured rules |
| Claude Code not installed | Med | High | Detect on start, clear install instructions |
| Unknown NDJSON message types | Med | Low | Log and skip, never crash |

## Related Documents

| Document | Description |
|----------|-------------|
| [OVERVIEW.md](./OVERVIEW.md) | System overview, tech stack, C4 diagrams |
| [DAEMON.md](./DAEMON.md) | Daemon internals, subprocess management |
| [PROTOCOL.md](./PROTOCOL.md) | Claude SDK protocol, gRPC API definitions |
| [TOPOLOGY.md](./TOPOLOGY.md) | Topology, connection modes, relay |
| [CLIENTS.md](./CLIENTS.md) | Flutter and CLI architecture |
| [SCHEMAS.md](./SCHEMAS.md) | SQLite schemas |
| [SECURITY.md](./SECURITY.md) | Auth, authorization, sandboxing |
| [SUBAGENTS.md](./SUBAGENTS.md) | Multi-agent orchestration, DAG scheduling |
