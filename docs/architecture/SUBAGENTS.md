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

## Worktree Concurrency Model

BetCode supports multiple subagents operating on overlapping or shared worktrees.
The concurrency strategy is **optimistic with fail-fast conflict detection**.

### Concurrency Strategies

| Strategy | Behavior | Use Case |
|----------|----------|----------|
| **Optimistic (default)** | Multiple subagents work freely; conflicts surface at git operations | Fast parallel execution, tolerant orchestrators |
| **Advisory Lock** | Subagent acquires lock before starting; other subagents wait or fail | Sequential access, critical sections |
| **Isolated Worktrees** | Each subagent gets its own worktree/branch | Maximum parallelism, merge at end |

### Optimistic Concurrency (Default)

Multiple subagents can operate on the same worktree simultaneously. File system
conflicts are handled as follows:

| Scenario | Behavior |
|----------|----------|
| Two subagents edit same file | Last write wins at file system level |
| Subagent A commits while B is editing | B's next commit may conflict |
| Git merge conflict | Bash tool returns non-zero exit; Claude sees error |
| File locked by OS | Write/Edit tool returns error; Claude retries or reports |

**Orchestrator responsibility**: When using optimistic concurrency, the orchestrator
should:
1. Design tasks to minimize file overlap (one subagent per module/directory).
2. Handle `ToolCallResult.is_error = true` from git operations.
3. Spawn a conflict-resolution subagent if needed.

### Advisory Worktree Lock

For orchestrators requiring exclusive access, the daemon provides advisory locks:

```protobuf
message SpawnSubagentRequest {
  // ... existing fields ...
  WorktreeLockMode worktree_lock = 10;
}

enum WorktreeLockMode {
  WORKTREE_LOCK_NONE = 0;      // Default: no lock
  WORKTREE_LOCK_SHARED = 1;    // Multiple readers, block writers
  WORKTREE_LOCK_EXCLUSIVE = 2; // Single writer, block all others
}
```

Lock behavior:

| Lock Mode | Same worktree, NONE | Same worktree, SHARED | Same worktree, EXCLUSIVE |
|-----------|---------------------|----------------------|-------------------------|
| NONE | Allow | Allow | Block (wait or fail) |
| SHARED | Allow | Allow | Block |
| EXCLUSIVE | Block | Block | Block |

When blocked:
- If `SpawnSubagentRequest.wait_for_lock = true`: queue request, spawn when lock released.
- If `wait_for_lock = false` (default): return `RESOURCE_EXHAUSTED` immediately.

Lock release: automatic on subagent completion (success, failure, or cancel).

### Advisory Lock Implementation

```sql
CREATE TABLE worktree_locks (
    worktree_id TEXT PRIMARY KEY REFERENCES worktrees(id),
    lock_mode TEXT NOT NULL CHECK (lock_mode IN ('shared', 'exclusive')),
    holder_session_ids TEXT NOT NULL,  -- JSON array
    acquired_at INTEGER NOT NULL
);
```

Lock acquisition is transactional:
```sql
-- Exclusive lock attempt (fails if any lock exists)
INSERT INTO worktree_locks (worktree_id, lock_mode, holder_session_ids, acquired_at)
SELECT ?, 'exclusive', ?, ?
WHERE NOT EXISTS (SELECT 1 FROM worktree_locks WHERE worktree_id = ?);

-- Shared lock attempt (fails if exclusive lock exists)
INSERT OR REPLACE INTO worktree_locks (...)
SELECT ... WHERE NOT EXISTS (... WHERE lock_mode = 'exclusive');
```

### Recommended Patterns

**Pattern 1: Isolated branches (maximum safety)**
```
Orchestrator creates worktree per subagent:
  worktree/feature-backend  -> subagent-backend
  worktree/feature-frontend -> subagent-frontend
  worktree/feature-tests    -> subagent-tests

Subagents work independently, orchestrator merges branches at end.
```

**Pattern 2: Exclusive lock for critical sections**
```
Subagent-1: SHARED lock, reads config files
Subagent-2: SHARED lock, reads config files
Subagent-3: EXCLUSIVE lock, modifies config files (subagents 1,2 wait)
```

**Pattern 3: Optimistic with retry**
```
Subagent fails due to git conflict
  -> Orchestrator catches StepFailed
  -> Spawns conflict-resolution subagent with EXCLUSIVE lock
  -> Resolution subagent commits
  -> Orchestrator retries original subagent
```

---

## Permission Handling

Four modes, combinable per subagent:

| Mode | Behavior |
|------|----------|
| **Auto-approve** | `auto_approve_permissions = true`: daemon auto-allows all `control_request` |
| **Inherit parent** | Copy parent session's `permission_grants` to subagent |
| **Tool restriction** | `allowed_tools` limits available tools via `--allowedTools` flag |
| **Forward to client** | Route `PermissionRequest` to parent session's input lock holder |

### Permission Validation Rules

The daemon enforces security invariants on `SpawnSubagentRequest` and `OrchestrationStep`
before spawning any subprocess.

#### Auto-Approve Constraint

**Rule**: When `auto_approve_permissions = true`, the `allowed_tools` field MUST be
non-empty.

| `auto_approve_permissions` | `allowed_tools` | Result |
|---------------------------|-----------------|--------|
| `false` | Empty (all tools) | Valid: permissions forwarded to client |
| `false` | Non-empty | Valid: permissions forwarded for listed tools |
| `true` | Empty | **INVALID**: returns `INVALID_ARGUMENT` |
| `true` | Non-empty | Valid: listed tools auto-approved |

**Rationale**: An empty allowlist means "all tools available". Combined with
`auto_approve_permissions = true`, this would auto-approve every tool invocation
including destructive operations (`Bash(rm -rf /)`, `Write` to arbitrary paths).
Requiring an explicit allowlist forces the orchestrator to consciously enumerate
which tools are safe for unattended execution.

#### Validation Error Response

```protobuf
// Returned when auto_approve_permissions = true with empty allowed_tools
ErrorEvent {
  code: "INVALID_PERMISSION_CONFIG",
  message: "auto_approve_permissions requires non-empty allowed_tools list",
  is_fatal: true
}
```

#### Recommended Safe Tool Sets

For common auto-approve scenarios, these tool sets balance utility with safety:

| Use Case | Recommended `allowed_tools` |
|----------|----------------------------|
| Read-only analysis | `["Read", "Glob", "Grep", "TodoWrite"]` |
| Code generation (no execution) | `["Read", "Glob", "Grep", "Write", "Edit"]` |
| Test execution | `["Read", "Glob", "Grep", "Bash"]` with pattern rules |
| Documentation | `["Read", "Glob", "Grep", "Write", "WebFetch"]` |

**Note**: Even with `allowed_tools` restrictions, the `Bash` tool can execute arbitrary
commands. Use the daemon's permission engine pattern rules (e.g., `"Bash(cargo test *)"`)
to constrain Bash invocations when auto-approving. Pattern rules are evaluated before
auto-approve.

#### Audit Logging

All auto-approved permissions are logged with:
- Subagent ID and parent session ID
- Tool name and input (truncated to 1KB)
- `auto_approve: true` flag
- Timestamp

This enables post-incident review of what a subagent auto-approved.

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

Subagents inherit the daemon's security model ([SECURITY.md](./SECURITY.md)).
This section details subagent-specific security controls with emphasis on
auto-approve hardening.

### Baseline Security (All Subagents)

| Control | Description |
|---------|-------------|
| Worktree enforcement | Each subagent is confined to its `working_directory` |
| Permission scoping | Permission grants are per-subagent session, no cross-session leakage |
| Resource limits | Max concurrent, max turns, timeout prevent exhaustion |
| API key sharing | All subagents share daemon's `ANTHROPIC_API_KEY` (no isolation) |

### Auto-Approve Security Model

**WARNING**: Auto-approve bypasses user confirmation for tool calls. This is
inherently dangerous and requires strict guardrails. See
[SECURITY.md](./SECURITY.md#auto-approve-subagent-security-hardening) for
complete security controls.

#### Mandatory Constraints

When `auto_approve_permissions = true`:

1. **Non-empty allowlist required**: `allowed_tools` MUST be specified
2. **Time-boxed execution**: Maximum 4 hours (default 1 hour)
3. **Rate-limited operations**: Per-minute and per-session limits
4. **Full audit trail**: Every auto-approved call logged with 90-day retention
5. **Runtime tool validation**: Tools verified before each auto-approve

#### SpawnSubagentRequest Security Fields

```protobuf
message SpawnSubagentRequest {
  string parent_session_id = 1;
  string prompt = 2;
  string model = 3;
  string working_directory = 4;
  repeated string allowed_tools = 5;       // REQUIRED if auto_approve=true
  int32 max_turns = 6;
  map<string, string> env = 7;
  string name = 8;
  bool auto_approve_permissions = 9;
  WorktreeLockMode worktree_lock = 10;

  // Security additions (Phase 2)
  AutoApproveConfig auto_approve_config = 11;
}

message AutoApproveConfig {
  int32 max_duration_seconds = 1;          // Default: 3600, Max: 14400
  int32 tool_calls_per_minute = 2;         // Default: 60, Max: 300
  int32 tool_calls_per_session = 3;        // Default: 1000, Max: 10000
  int32 bash_commands_per_minute = 4;      // Default: 20, Max: 60
  int32 write_operations_per_minute = 5;   // Default: 30, Max: 100
  bool queue_on_rate_limit = 6;            // Default: false (deny)
}
```

#### Spawn-Time Validation

The daemon performs these checks before spawning an auto-approve subagent:

```
1. Verify allowed_tools is non-empty
   -> INVALID_ARGUMENT: "auto_approve_permissions requires non-empty allowed_tools"

2. Verify all tools exist in registry
   -> INVALID_ARGUMENT: "unknown tool in allowed_tools: {tool_name}"

3. Verify max_duration_seconds is within bounds
   -> INVALID_ARGUMENT: "max_duration_seconds must be 60-14400"

4. Verify rate limits are within bounds
   -> INVALID_ARGUMENT: "tool_calls_per_minute must be 10-300"

5. Create audit entry: subagent_spawn with auto_approve=true
```

#### Runtime Tool Call Flow (Auto-Approve)

```
Claude requests tool call
    |
    v
[1] Is tool in allowed_tools?
    |-- No --> Forward to client for manual approval
    |-- Yes --> Continue
    v
[2] Does tool still exist in registry?
    |-- No --> Deny, audit: tool_validation_fail
    |-- Yes --> Continue
    v
[3] Is subagent within time limit?
    |-- No --> SIGTERM subagent, audit: subagent_timeout
    |-- Yes --> Continue
    v
[4] Is rate limit available?
    |-- No, queue=false --> Deny, audit: rate_limit_exceeded
    |-- No, queue=true  --> Queue with backoff
    |-- Yes --> Continue
    v
[5] Execute tool, audit: auto_approve_tool_call
    |
    v
[6] Return result to Claude
```

#### Mid-Execution Revocation

Auto-approve can be revoked without killing the subagent. The subagent
continues but subsequent tool calls require manual approval.

```protobuf
// Added to SubagentService
rpc RevokeAutoApprove(RevokeAutoApproveRequest) returns (RevokeAutoApproveResponse);

message RevokeAutoApproveRequest {
  string subagent_id = 1;
  string reason = 2;             // Required, min 10 chars
  bool terminate_if_pending = 3; // Kill if tool call in flight
}

message RevokeAutoApproveResponse {
  bool revoked = 1;
  int32 pending_tool_calls = 2;
  string subagent_status = 3;    // 'running', 'completed', 'failed'
}
```

**Use cases for revocation**:
- Suspicious activity detected in audit log
- User wants to review remaining operations manually
- External system signals concern (security scanner, etc.)
- Subagent approaching sensitive operation

#### CancelSubagent with Permission Cleanup

When canceling an auto-approve subagent, all associated permissions and
in-flight state must be cleaned up atomically.

```protobuf
message CancelSubagentRequest {
  string subagent_id = 1;
  string reason = 2;              // Required for audit
  bool force = 3;                 // SIGKILL vs SIGTERM
  bool cleanup_worktree = 4;      // Revert uncommitted changes
}

message CancelSubagentResponse {
  bool cancelled = 1;
  string final_status = 2;
  int32 tool_calls_executed = 3;  // For audit summary
  int32 tool_calls_auto_approved = 4;
}
```

**Cleanup sequence**:

```
1. Set subagent.auto_approve_permissions = false (prevent new auto-approves)
2. If force=false: SIGTERM, wait 10s grace period
3. If force=true or grace period exceeded: SIGKILL
4. Release worktree lock (if held)
5. Audit: subagent_cancel with final metrics
6. If cleanup_worktree=true: git checkout . && git clean -fd
7. Update subagents table: status='cancelled'
```

### Audit Log Integration

All subagent security events are logged to the `audit_log` table. See
[SECURITY.md](./SECURITY.md#audit-log-schema) for schema details.

**Subagent-specific audit events**:

| Event Type | Trigger | Severity |
|------------|---------|----------|
| `subagent_spawn` | SpawnSubagent called | info (warn if auto_approve) |
| `subagent_complete` | Clean exit | info |
| `subagent_fail` | Non-zero exit | warn |
| `subagent_cancel` | CancelSubagent called | warn |
| `subagent_timeout` | Time limit exceeded | error |
| `auto_approve_tool_call` | Tool auto-approved | warn |
| `tool_validation_fail` | Tool no longer valid | error |
| `rate_limit_exceeded` | Rate limit hit | warn |
| `permission_revoke` | RevokeAutoApprove called | warn |

**Audit context for auto_approve_tool_call**:

```json
{
  "tool_name": "Bash",
  "tool_input_preview": "cargo test --workspace",
  "tool_input_hash": "sha256:...",
  "allowed_tools": ["Read", "Bash", "Glob"],
  "rate_limit_state": {
    "tool_calls_remaining": 47,
    "bash_calls_remaining": 15,
    "session_calls_remaining": 892
  },
  "time_remaining_seconds": 2847,
  "execution_time_ms": 1234
}
```

### Security Recommendations for Orchestrators

Orchestrators using auto-approve subagents SHOULD:

1. **Minimize allowed_tools**: Only include tools necessary for the task
2. **Prefer read-only tools**: `["Read", "Glob", "Grep"]` when possible
3. **Constrain Bash patterns**: Use permission engine rules if Bash needed
4. **Set conservative time limits**: 30 minutes for most tasks, not 4 hours
5. **Monitor audit logs**: Alert on `rate_limit_exceeded`, `tool_validation_fail`
6. **Use isolated worktrees**: Prevent cross-subagent file conflicts
7. **Implement circuit breakers**: Revoke auto-approve on repeated failures

**Example: Minimal auto-approve for test runner**:

```protobuf
SpawnSubagentRequest {
  prompt: "Run the test suite and report failures"
  allowed_tools: ["Read", "Glob", "Grep", "Bash"]
  auto_approve_permissions: true
  auto_approve_config: {
    max_duration_seconds: 1800   // 30 minutes
    bash_commands_per_minute: 10 // Conservative
    tool_calls_per_session: 200  // Bounded
  }
}
```

With permission engine rule (in daemon config):

```toml
[[permissions.rules]]
tool = "Bash"
pattern = "cargo test *"
action = "allow"

[[permissions.rules]]
tool = "Bash"
pattern = "*"
action = "deny"  # Deny all other Bash commands
```

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
