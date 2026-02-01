# Implementation Roadmap

**Version**: 0.2.0
**Last Updated**: 2026-02-01
**Status**: Design Phase
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

## Phase 1: Foundation -- Single Machine CLI

**Goal**: Daemon + CLI on one machine. Full agent capability via Claude Code subprocess.
**Scope**: 4 crates (proto, core, daemon, cli), ~8,000-12,000 LOC Rust.

**betcode-proto**: AgentService proto (Converse, ListSessions, ResumeSession,
CompactSession, CancelTurn), ConfigService proto, tonic-build codegen, shared
message types (AgentEvent, AgentRequest, TextDelta, ToolCallStart, ToolCallResult,
PermissionRequest, PermissionResponse, StatusChange).

**betcode-core**: NDJSON stream parser (all Claude Code message types: assistant,
tool_use, tool_result, result, system, control_request, input_request), typed Rust
structs with serde, config types and resolution (.claude/settings.json hierarchy),
permission rule parsing (Claude Code "Tool(pattern)" format), shared error types.

**betcode-daemon**: Claude subprocess manager (spawn with --output-format stream-json,
--resume, stdin/stdout I/O bridge, health monitoring), permission bridge
(control_request -> auto-respond or forward to client, timeout 60s default), session
store (SQLite: sessions, messages, permission_grants), local gRPC server (named pipe
on Windows, Unix socket on Linux/macOS), config resolver (merge hierarchy: CLI > env >
project local > project > user), first-run import (~/.claude/ -> config dir), daemon
lifecycle (signal handling, graceful shutdown, PID file, tracing).

**betcode-cli**: clap parsing (chat, -p, --resume, --model, session, daemon
subcommands), ratatui TUI (input pane, streaming markdown output, tool call
indicators, permission prompt dialogs, status bar), local daemon connection
(auto-detect socket, offer to start), headless mode (-p with json/stream-json output).

**Deliverable**: `betcode` drops into any project with CLAUDE.md + .claude/settings.json.
All 17+ tools, MCP, hooks, sub-agents, skills work because it IS Claude Code.

---

## Phase 2: Worktrees and Session Management

**Goal**: Git worktree orchestration, multi-session support, input locking.

**Worktree manager**: create/remove/list/switch worktrees, auto-setup hooks (npm
install, cargo build, etc.), health checks for prunable/locked/broken worktrees.

**Per-worktree sessions**: each worktree gets own Claude subprocess with correct CWD,
worktrees table in SQLite, session-worktree binding, cascade cleanup on deletion.

**Session lifecycle**: resume via --resume with claude_session_id, compact via RPC,
session list with metadata (tokens, last message, duration, worktree).

**Input locking**: one client controls input per session, lock on first UserMessage,
release on turn completion or disconnect, other clients get read-only event stream,
connected_clients tracking with heartbeat cleanup.

**CLI additions**: `betcode worktree create/list/switch/remove`, session list with
worktree info.

---

## Phase 3: Relay and Mobile

**Goal**: Remote access via relay + Flutter mobile app.

**betcode-relay** (new crate): TunnelService (bidirectional stream), JWT auth for
clients, mTLS auth for daemons, connection registry (machine_id -> tunnel), request
routing through tunnel with request_id correlation, message buffering for offline
daemons (SQLite, 24h TTL, 1000 msg cap), user registration/login, relay database
(users, tokens, machines, message_buffer, certificates).

**Daemon additions**: reverse tunnel client (mTLS to relay), reconnection state
machine (exponential backoff 1s-60s), certificate management (generate keypair,
CSR, auto-renewal).

**Flutter app**: Riverpod + go_router + drift + flutter_secure_storage scaffold,
JWT auth flow, gRPC client with reconnection, conversation screen (streaming text,
tool cards, permission dialogs, task list), machine list + switching, session
list + resume, offline sync engine (queue requests, replay on reconnect).

**Deliverable**: Use BetCode from phone via relay to any registered machine.

---

## Phase 4: Multi-Machine and Polish

**Goal**: Cross-machine switching, GitLab integration, LAN mode, production readiness.

**Cross-machine**: machine_id routing via relay, CLI `betcode machine list/switch`,
Flutter machine switching, persisted active machine.

**GitLab integration**: reqwest API client in daemon (MRs, pipelines, issues),
GitLabService gRPC, Flutter GitLab screens, CLI `betcode gitlab` commands.

**Direct LAN mode**: mDNS discovery, explicit config, mTLS reuse, automatic prefer
LAN over relay.

**Flutter web**: PWA build, same codebase, gRPC-web transport.

**Polish**: connection pooling, keepalive tuning, structured logging with trace IDs,
error codes and diagnostic dumps, opt-in metrics, push notifications for completions.

---

## Subagent Orchestration (Cross-Phase)

BetCode exposes a `SubagentService` gRPC API for spawning and coordinating
multiple independent Claude Code subprocesses. This enables external
orchestrators (separate projects) to build sophisticated feature-development
workflows on top of BetCode.

- **Phase 2**: Basic subagent spawning (SpawnSubagent, WatchSubagent, ListSubagents)
- **Phase 4**: Full orchestration API (OrchestrationPlan, DAG scheduler, context sharing)

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
| Rust crates | 5 (proto, core, daemon, relay, cli) |
| Estimated Rust LOC | ~10,000-15,000 |
| NDJSON + subprocess + bridge | ~1,800 LOC |
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
