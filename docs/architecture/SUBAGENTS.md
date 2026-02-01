# Subagent Orchestration Architecture

**Version**: 0.2.0
**Last Updated**: 2026-02-01
**Status**: Design Phase

## Overview

BetCode supports subagent execution at two distinct levels. **Level 1** is Claude Code's built-in `Task` tool, which spawns lightweight agent loops inside a single subprocess. **Level 2** is BetCode's daemon-orchestrated subagent system, which spawns multiple independent Claude Code subprocesses that work on separate tasks simultaneously, potentially in different worktrees.

Level 1 requires nothing from BetCode -- the daemon streams NDJSON through to clients. Level 2 is BetCode's key architectural addition: a subprocess pool, DAG scheduler, and gRPC API that external orchestrators build on top of.

```
                              External Orchestrator (separate project, gRPC)
                                        |
                                        v  SubagentService
+-----------------------------------------------------------------------+
|  betcode-daemon                                                       |
|  Subprocess Pool | DAG Scheduler | Context Manager                    |
|       |          |          |          |                               |
|  [claude #1] [claude #2] [claude #3] [claude #4]                     |
|  worktree:A  worktree:B  worktree:C  worktree:A                     |
+-----------------------------------------------------------------------+
```

---

## Level 1: Claude-Internal Subagents

Claude Code's `Task` tool spawns subagents within a single session. These
run inside the same `claude` process as lightweight agent loops sharing
the parent's context window and working directory.

### How They Appear in stream-json

The daemon sees subagent activity through these NDJSON markers:

1. **Subagent spawn**: `assistant` message with `tool_use` block where
   `name: "Task"`. The `input` contains `prompt`, `subagent_type`,
   `model`, `max_turns`, `run_in_background`.

2. **Subagent messages**: All `stream_event`, `assistant`, and `user`
   messages from within a subagent carry `parent_tool_use_id` set to
   the `Task` tool_use block's `id`. Top-level messages have
   `parent_tool_use_id: null`.

3. **Subagent permissions**: `control_request` messages from subagents
   also carry `parent_tool_use_id`. The daemon bridges these to clients
   the same way, but clients can display which subagent is requesting.

4. **Subagent completion**: A `tool_result` for the `Task` tool_use
   block, containing the subagent's final output text.

### What the Daemon Does

- **Tracks parent_tool_use_id**: The multiplexer tags gRPC `AgentEvent`
  messages with the originating subagent ID so clients can render
  subagent activity in nested/collapsible UI sections.
- **Permission bridge**: Subagent permissions flow through the same
  bridge. The `PermissionRequest` gRPC message includes a
  `parent_tool_use_id` field so the client shows context like
  "Subagent (Explore) wants to read src/main.rs".
- **Usage tracking**: Token usage from subagents is included in the
  parent session's aggregated `result` message. The daemon does not
  need to track subagent usage separately.
- **Background subagents**: When `run_in_background: true`, Claude Code
  runs the subagent asynchronously. Messages continue to stream with
  the `parent_tool_use_id` set. The daemon handles these identically.

### gRPC AgentEvent Addition

```protobuf
message AgentEvent {
  uint64 sequence = 1;
  google.protobuf.Timestamp timestamp = 2;
  string parent_tool_use_id = 3;  // non-empty = from internal subagent

  oneof event { ... }
}
```

Clients use `parent_tool_use_id` to:
- Nest subagent output under the parent Task call in conversation view
- Show subagent type/model in a badge
- Collapse/expand subagent activity
- Route permission prompts with context

## Level 2: Daemon-Orchestrated Subagents

The daemon spawns independent `claude` subprocesses, each with its own session, process, working directory, model, tool restrictions, and permission context. Unlike Level 1, Level 2 subagents run in parallel across separate processes with isolated context windows.

**Use cases**: parallel feature development across worktrees, large refactors (one agent per module), multi-concern tasks (docs + implementation + tests), cost optimization (Haiku for simple subtasks, Opus for complex ones).

---

## SubagentService gRPC API

New service in `proto/betcode/v1/subagent.proto`. Existing services unchanged.

```protobuf
service SubagentService {
  rpc SpawnSubagent(SpawnSubagentRequest) returns (SpawnSubagentResponse);
  rpc WatchSubagent(WatchSubagentRequest) returns (stream AgentEvent);
  rpc SendToSubagent(SubagentInput) returns (SubagentInputResponse);
  rpc CancelSubagent(CancelSubagentRequest) returns (CancelSubagentResponse);
  rpc ListSubagents(ListSubagentsRequest) returns (ListSubagentsResponse);
  rpc CreateOrchestration(OrchestrationPlan) returns (OrchestrationResponse);
  rpc WatchOrchestration(WatchOrchestrationRequest) returns (stream OrchestrationEvent);
}

message SpawnSubagentRequest {
  string parent_session_id = 1;
  string prompt = 2;
  string model = 3;                   // Override (e.g. haiku for fast tasks)
  string working_directory = 4;       // Can differ from parent
  repeated string allowed_tools = 5;  // Empty = all tools
  int32 max_turns = 6;               // 0 = unlimited
  map<string, string> env = 7;
  string name = 8;                    // Human-readable label
  bool auto_approve_permissions = 9;
}

message SpawnSubagentResponse {
  string subagent_id = 1;
  string session_id = 2;             // Claude session ID for the subprocess
}
```

### Orchestration Messages

```protobuf
message OrchestrationPlan {
  string parent_session_id = 1;
  repeated OrchestrationStep steps = 2;
  OrchestrationStrategy strategy = 3;  // PARALLEL, SEQUENTIAL, DAG
}

message OrchestrationStep {
  string id = 1;
  string name = 2;
  string prompt = 3;
  string model = 4;
  string working_directory = 5;
  repeated string allowed_tools = 6;
  repeated string depends_on = 7;      // Step IDs (DAG edges)
  int32 max_turns = 8;
  bool auto_approve_permissions = 9;
}

message OrchestrationEvent {
  string orchestration_id = 1;
  oneof event {
    StepStarted step_started = 2;
    StepCompleted step_completed = 3;
    StepFailed step_failed = 4;
    OrchestrationCompleted completed = 5;
    OrchestrationFailed failed = 6;
    SubagentAgentEvent agent_event = 7; // Proxied from step's claude
  }
}
```

See `proto/betcode/v1/subagent.proto` for complete message definitions including `SubagentInfo`, `SubagentStatus`, `StepStarted`, `StepCompleted`, `StepFailed`, `OrchestrationCompleted`, `OrchestrationFailed`, and supporting types.

---

## Daemon Implementation

### New Module Structure

```
betcode-daemon/src/
  subprocess/
    process.rs               # Single claude subprocess (existing)
    pool.rs                  # Pool of claude subprocesses (new)
  orchestration/
    mod.rs                   # SubagentService gRPC impl
    manager.rs               # Subagent lifecycle (spawn, monitor, cancel)
    scheduler.rs             # DAG scheduler (topological sort, parallel dispatch)
    context.rs               # Shared context between subagents
```

### Subprocess Pool

Each subagent gets an independent `claude` process using the same spawn command as regular sessions (see [DAEMON.md](./DAEMON.md)) with `--max-turns` and optional `--allowedTools`.

| Parameter | Default | Range | Config path |
|-----------|---------|-------|-------------|
| Max concurrent subagents | 5 | 1-20 | `subagents.max_concurrent` |
| Max turns per subagent | 50 | 1-200 | Per-request |
| Subagent timeout | 30 min | 1-120 min | `subagents.timeout_minutes` |
| Max subagents per parent | 20 | 1-100 | `subagents.max_per_session` |

When the pool is full, requests queue. Pool-at-capacity returns `RESOURCE_EXHAUSTED` if the queue is also full.

### DAG Scheduler

For `strategy = DAG`: validate graph (reject cycles), compute in-degrees, spawn zero-dependency steps in parallel, decrement downstream in-degrees on completion, dispatch newly unblocked steps. Failed steps cascade failure to all downstream dependents.

```
Example:  [analyze] --> [backend]  --> [integration-tests]
          [analyze] --> [frontend] --> [integration-tests]
          [analyze] --> [docs]

t0: spawn [analyze]
t1: complete -> spawn [backend], [frontend], [docs] in parallel
t2: [backend] + [frontend] complete -> spawn [integration-tests]
t3: [docs] completes, [integration-tests] completes -> done
```

`PARALLEL` spawns all steps immediately. `SEQUENTIAL` creates an implicit chain.

### Context Sharing

Three mechanisms for information flow between subagents:

1. **File-based**: subagents share or overlap worktrees; git handles merging
2. **Summary injection**: completed step's `result_summary` prepended to downstream prompts
3. **Artifact-based**: steps produce files at known paths for downstream consumption

---

## Permission Handling

Four modes, combinable per subagent:

| Mode | Behavior |
|------|----------|
| **Auto-approve** | `auto_approve_permissions = true`: daemon auto-allows all `control_request` |
| **Inherit parent** | Copy parent session's `permission_grants` to subagent |
| **Tool restriction** | `allowed_tools` limits available tools via `--allowedTools` flag |
| **Forward to client** | Route `PermissionRequest` to parent session's input lock holder |

---

## Session Hierarchy

```
Parent Session (interactive)
  |-- Subagent A (backend)      [worktree: feature/auth-backend]
  |-- Subagent B (frontend)     [worktree: feature/auth-frontend]
  |-- Subagent C (tests)        [worktree: feature/auth-tests]
  +-- Subagent D (docs)         [same worktree as parent]
```

Child sessions are full sessions in SQLite with their own message history, permission grants, and token tracking. Parent termination does NOT auto-kill running children -- they complete or timeout independently.

---

## Database Schema Additions

Three new tables in the daemon database. See [SCHEMAS.md](./SCHEMAS.md) for existing tables.

```sql
CREATE TABLE subagents (
    id TEXT PRIMARY KEY,
    parent_session_id TEXT NOT NULL REFERENCES sessions(id),
    session_id TEXT NOT NULL REFERENCES sessions(id),
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    prompt TEXT NOT NULL,
    model TEXT,
    working_directory TEXT NOT NULL,
    auto_approve_permissions INTEGER NOT NULL DEFAULT 0,
    max_turns INTEGER NOT NULL DEFAULT 50,
    result_summary TEXT,
    created_at INTEGER NOT NULL,
    completed_at INTEGER
);
CREATE INDEX idx_subagents_parent ON subagents(parent_session_id);
CREATE INDEX idx_subagents_status ON subagents(status) WHERE status IN ('pending', 'running');

CREATE TABLE orchestrations (
    id TEXT PRIMARY KEY,
    parent_session_id TEXT NOT NULL REFERENCES sessions(id),
    strategy TEXT NOT NULL CHECK (strategy IN ('parallel', 'sequential', 'dag')),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed')),
    total_steps INTEGER NOT NULL,
    completed_steps INTEGER NOT NULL DEFAULT 0,
    failed_steps INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    completed_at INTEGER
);
CREATE INDEX idx_orchestrations_parent ON orchestrations(parent_session_id);

CREATE TABLE orchestration_steps (
    id TEXT PRIMARY KEY,
    orchestration_id TEXT NOT NULL REFERENCES orchestrations(id) ON DELETE CASCADE,
    subagent_id TEXT REFERENCES subagents(id),
    name TEXT NOT NULL,
    prompt TEXT NOT NULL,
    model TEXT,
    working_directory TEXT,
    allowed_tools TEXT,                -- JSON array
    depends_on TEXT,                   -- JSON array of step IDs
    auto_approve_permissions INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'blocked')),
    sequence INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    completed_at INTEGER
);
CREATE INDEX idx_orch_steps ON orchestration_steps(orchestration_id, sequence);
```

**Entity relationships**: `sessions 1--<* subagents` (parent), `sessions 1--<1 subagents` (own session), `sessions 1--<* orchestrations`, `orchestrations 1--<* orchestration_steps`, `subagents 1--<? orchestration_steps`.

---

## External Orchestrator Pattern

BetCode provides **infrastructure** (subprocess pool, worktree isolation, session persistence, permission delegation, DAG scheduling, event streaming, cost tracking). An external orchestrator provides **strategy** (task decomposition, quality gates, integration testing, failure recovery, domain workflows, branch management).

```
Feature Orchestrator                    BetCode Daemon
====================                    ==============
1. Analyze feature request        --->  SpawnSubagent (planner)
2. Receive plan                   <---  WatchSubagent
3. Create worktrees               --->  WorktreeService.CreateWorktree
4. Submit DAG plan                --->  CreateOrchestration
5. Monitor progress               <---  WatchOrchestration
6. Handle failures                --->  SpawnSubagent (retry)
7. Run integration tests          --->  SpawnSubagent (test runner)
8. Merge branches                       (git operations)
```

**Why external**: orchestration strategy is domain-specific and changes rapidly. Keeping it external means different teams use different strategies, the orchestrator itself can be an AI agent, BetCode stays small, orchestrator upgrades need no daemon redeployment, and multiple orchestrators coexist.

---

## Error Handling

| Scenario | Response |
|----------|----------|
| Exit 0 | Mark completed, store result_summary |
| Exit non-zero | Mark failed, store error |
| Exceeds max_turns | SIGTERM (Claude exits cleanly at turn limit) |
| Timeout | SIGTERM, wait 5s, SIGKILL, mark failed |
| Pool full + queue full | Reject with RESOURCE_EXHAUSTED |
| DAG cycle | Reject with INVALID_ARGUMENT |
| Dependency fails | Mark downstream as failed ("dependency failed") |
| Daemon shutdown | SIGTERM all subagents, persist state |

No automatic restart for subagents (unlike interactive sessions). The orchestrator decides whether to retry.

## Security Considerations

Subagents inherit the daemon's security model ([SECURITY.md](./SECURITY.md)). Additional constraints: worktree enforcement per subagent, permission grants scoped per subagent session, auto-approve logged for audit, resource limits prevent exhaustion, all subagents share the daemon's `ANTHROPIC_API_KEY` (no per-subagent isolation).

## Roadmap Integration

**Phase 2**: `SpawnSubagent`, `WatchSubagent`, `CancelSubagent`, `ListSubagents`, subprocess pool, `subagents` table, permission inheritance, auto-approve, CLI commands.

**Phase 4**: `CreateOrchestration`, `WatchOrchestration`, DAG scheduler, summary injection, `orchestrations` + `orchestration_steps` tables, UI integration, cost aggregation.

---

## Related Documents

| Document | Description |
|----------|-------------|
| [OVERVIEW.md](./OVERVIEW.md) | System overview, C4 diagrams, tech stack |
| [DAEMON.md](./DAEMON.md) | Daemon internals, subprocess management |
| [PROTOCOL.md](./PROTOCOL.md) | Communication protocols (NDJSON + gRPC) |
| [PROTOCOL_L2.md](./PROTOCOL_L2.md) | gRPC API definitions |
| [SCHEMAS.md](./SCHEMAS.md) | SQLite schemas for daemon, relay, client |
| [SECURITY.md](./SECURITY.md) | Auth, permissions, sandboxing |
| [ROADMAP.md](./ROADMAP.md) | Phased implementation plan |
| [TOPOLOGY.md](./TOPOLOGY.md) | Network topology, relay architecture |
| [CLIENTS.md](./CLIENTS.md) | CLI and Flutter client architecture |
