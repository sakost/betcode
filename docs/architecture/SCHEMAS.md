# Database Schemas

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Parent**: [OVERVIEW.md](./OVERVIEW.md)

All three BetCode components use SQLite for persistence. The daemon and
relay use sqlx with compile-time checked queries and embedded migrations.
The Flutter client uses drift (formerly moor) with generated Dart code.

All timestamps are stored as Unix epoch seconds (INTEGER). All UUIDs and
identifiers are stored as TEXT (UUIDv7 formatted strings, sortable by
creation time). Boolean values use INTEGER (0/1). JSON payloads are stored
as TEXT with CHECK constraints where practical.

---

## Table of Contents

- [Daemon Database](#daemon-database)
- [Relay Database](#relay-database)
- [Client Database (Flutter/drift)](#client-database-flutterdrift)
- [Design Decisions](#design-decisions)
- [Migration Strategy](#migration-strategy)
- [Entity Relationship Overview](#entity-relationship-overview)
- [Query Patterns](#query-patterns)

---

## Daemon Database

The daemon database lives in the BetCode config directory (see
[DAEMON.md](./DAEMON.md) for platform-specific paths). Default:
`$XDG_CONFIG_HOME/betcode/daemon.db` (Linux),
`~/Library/Application Support/betcode/daemon.db` (macOS),
`%USERPROFILE%\.betcode\daemon.db` (Windows). It stores session
history, worktree state, permission grants, and connected client tracking.
The daemon is the only writer; reads happen from the gRPC service layer.

SQLite is configured with WAL mode, foreign keys enabled, and a busy
timeout of 5000ms.

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
```

### sessions

Tracks Claude subprocess sessions. Each session corresponds to one
`claude --json` subprocess managed by the daemon. The `id` comes from
Claude's `system.init` message. The `claude_session_id` is Claude's
internal identifier used for `--resume`.

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    claude_session_id TEXT,
    worktree_id TEXT REFERENCES worktrees(id),
    status TEXT NOT NULL DEFAULT 'idle'
        CHECK (status IN ('idle', 'active', 'completed', 'error')),
    model TEXT NOT NULL,
    working_directory TEXT NOT NULL,
    input_lock_client TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    total_input_tokens INTEGER DEFAULT 0,
    total_output_tokens INTEGER DEFAULT 0,
    total_cost_usd REAL DEFAULT 0.0,
    last_message_preview TEXT
);

CREATE INDEX idx_sessions_worktree ON sessions(worktree_id);
CREATE INDEX idx_sessions_status ON sessions(status)
    WHERE status = 'active';
CREATE INDEX idx_sessions_updated ON sessions(updated_at DESC);
```

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | From Claude's `system.init` message |
| claude_session_id | TEXT | Claude's internal session ID for `--resume` |
| worktree_id | TEXT FK | References worktrees(id), nullable |
| status | TEXT | idle (no subprocess), active (subprocess running), completed, error |
| model | TEXT | Model identifier (e.g. claude-sonnet-4-20250514) |
| working_directory | TEXT | Absolute path to session working directory |
| input_lock_client | TEXT | client_id currently holding the input lock |
| created_at | INTEGER | Unix epoch seconds |
| updated_at | INTEGER | Unix epoch seconds, updated on every state change |
| total_input_tokens | INTEGER | Cumulative input token count |
| total_output_tokens | INTEGER | Cumulative output token count |
| total_cost_usd | REAL | Cumulative estimated cost in USD |
| last_message_preview | TEXT | Truncated last assistant message (max 200 chars), nullable |

### messages

Stores every NDJSON line emitted by Claude's stdout. This is the raw
stream-json output, stored verbatim for faithful replay. Clients
reconnecting mid-session request messages starting from a sequence number
and receive the exact output Claude produced.

```sql
CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    message_type TEXT NOT NULL
        CHECK (message_type IN (
            'system', 'assistant', 'user', 'result',
            'stream_event', 'control_request', 'control_response'
        )),
    payload TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_messages_session_seq
    ON messages(session_id, sequence);
```

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-incrementing row ID |
| session_id | TEXT FK | References sessions(id), cascading delete |
| sequence | INTEGER | Monotonically increasing per session, gapless |
| message_type | TEXT | Top-level NDJSON type, extracted from JSON 'type' field |
| payload | TEXT | Raw JSON line from Claude's stdout |
| created_at | INTEGER | Unix epoch seconds |

The unique index on (session_id, sequence) enforces ordering invariants
and serves as the primary query path for replay.

### worktrees

Tracks git worktrees managed by the daemon. Each worktree is an isolated
working directory with its own branch, created via `git worktree add`.
Sessions are optionally bound to a worktree.

```sql
CREATE TABLE worktrees (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    branch TEXT NOT NULL,
    repo_path TEXT NOT NULL,
    setup_script TEXT,
    created_at INTEGER NOT NULL,
    last_active INTEGER NOT NULL
);

CREATE INDEX idx_worktrees_repo ON worktrees(repo_path);
```

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | UUIDv7 |
| name | TEXT | Human-readable worktree name |
| path | TEXT UNIQUE | Absolute filesystem path to worktree root |
| branch | TEXT | Git branch name checked out in this worktree |
| repo_path | TEXT | Absolute path to the parent git repository |
| setup_script | TEXT | Optional shell command run after worktree creation |
| created_at | INTEGER | Unix epoch seconds |
| last_active | INTEGER | Unix epoch seconds, updated on session activity |

### permission_grants

Runtime permission decisions made by the user during a session. These
persist for the session lifetime and are deleted when the session ends
(CASCADE). They sit at priority level 6 in the permission hierarchy
(see [DAEMON.md](./DAEMON.md) for permission hierarchy).

```sql
CREATE TABLE permission_grants (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    pattern TEXT,
    action TEXT NOT NULL
        CHECK (action IN ('allow', 'deny')),
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_grants_session_tool
    ON permission_grants(session_id, tool_name);
```

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-incrementing row ID |
| session_id | TEXT FK | References sessions(id), cascading delete |
| tool_name | TEXT | Tool identifier (e.g. Bash, Edit, mcp__github__*) |
| pattern | TEXT | Optional glob pattern (e.g. "git *", "src/**/*.rs") |
| action | TEXT | allow or deny |
| created_at | INTEGER | Unix epoch seconds |

### connected_clients

Tracks clients currently connected to the daemon over gRPC. Used for
multiplexing: multiple clients can observe the same session but only one
holds the input lock at a time. Rows are cleaned up on disconnect or
heartbeat timeout (30 seconds).

```sql
CREATE TABLE connected_clients (
    client_id TEXT PRIMARY KEY,
    session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
    client_type TEXT NOT NULL
        CHECK (client_type IN ('cli', 'flutter', 'headless')),
    has_input_lock INTEGER NOT NULL DEFAULT 0
        CHECK (has_input_lock IN (0, 1)),
    connected_at INTEGER NOT NULL,
    last_heartbeat INTEGER NOT NULL
);

CREATE INDEX idx_clients_session ON connected_clients(session_id);
CREATE INDEX idx_clients_heartbeat ON connected_clients(last_heartbeat);
```

| Column | Type | Description |
|--------|------|-------------|
| client_id | TEXT PK | Client-generated UUID, unique per connection |
| session_id | TEXT FK | References sessions(id), nullable (not yet attached) |
| client_type | TEXT | cli, flutter, or headless |
| has_input_lock | INTEGER | 0 or 1; at most one per session holds 1 |
| connected_at | INTEGER | Unix epoch seconds |
| last_heartbeat | INTEGER | Unix epoch seconds, updated by keepalive pings |

**Invariant**: For any given session_id, at most one row has
`has_input_lock = 1`. Enforced at the application layer via
compare-and-swap logic within a transaction.

### audit_log

Persistent security audit trail for forensic analysis and compliance. This
table stores all security-relevant events with mandatory retention periods.
See [SECURITY.md](./SECURITY.md#audit-log-schema) for detailed schema
documentation and retention policies.

**Critical**: This table is required for daemon startup. The daemon refuses
to start if the audit_log table is missing or corrupted.

```sql
CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    event_type TEXT NOT NULL
        CHECK (event_type IN (
            'auto_approve_tool_call',
            'permission_grant',
            'permission_deny',
            'permission_revoke',
            'subagent_spawn',
            'subagent_complete',
            'subagent_fail',
            'subagent_cancel',
            'subagent_timeout',
            'session_create',
            'session_resume',
            'session_complete',
            'session_error',
            'rate_limit_exceeded',
            'tool_validation_fail',
            'auth_success',
            'auth_failure'
        )),
    severity TEXT NOT NULL DEFAULT 'info'
        CHECK (severity IN ('debug', 'info', 'warn', 'error', 'critical')),
    session_id TEXT,
    subagent_id TEXT,
    parent_session_id TEXT,
    actor_type TEXT NOT NULL
        CHECK (actor_type IN ('user', 'daemon', 'subagent', 'system')),
    actor_id TEXT NOT NULL,
    tool_name TEXT,
    tool_input_hash TEXT,
    tool_input_preview TEXT,
    outcome TEXT NOT NULL
        CHECK (outcome IN ('success', 'failure', 'denied', 'timeout')),
    error_code TEXT,
    error_message TEXT,
    context TEXT,
    client_ip TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX idx_audit_session ON audit_log(session_id, created_at DESC)
    WHERE session_id IS NOT NULL;
CREATE INDEX idx_audit_subagent ON audit_log(subagent_id, created_at DESC)
    WHERE subagent_id IS NOT NULL;
CREATE INDEX idx_audit_type ON audit_log(event_type, created_at DESC);
CREATE INDEX idx_audit_expiry ON audit_log(expires_at)
    WHERE expires_at > 0;
CREATE INDEX idx_audit_actor ON audit_log(actor_type, actor_id, created_at DESC);
CREATE INDEX idx_audit_tool ON audit_log(tool_name, created_at DESC)
    WHERE tool_name IS NOT NULL;
```

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-incrementing row ID |
| event_id | TEXT UNIQUE | UUIDv7 for global correlation across components |
| event_type | TEXT | Security event category (see CHECK constraint) |
| severity | TEXT | Log level: debug, info, warn, error, critical |
| session_id | TEXT | References sessions(id), nullable for non-session events |
| subagent_id | TEXT | References subagents(id), nullable for non-subagent events |
| parent_session_id | TEXT | Parent session for subagent correlation |
| actor_type | TEXT | Entity initiating the event |
| actor_id | TEXT | Identifier of the actor (user_id, subagent_id, or 'system') |
| tool_name | TEXT | Tool identifier for tool-related events, nullable |
| tool_input_hash | TEXT | SHA-256 hash of tool input (integrity, not raw data) |
| tool_input_preview | TEXT | First 256 chars of input, sanitized |
| outcome | TEXT | Result of the operation |
| error_code | TEXT | Structured error identifier, nullable |
| error_message | TEXT | Human-readable error, max 1KB, nullable |
| context | TEXT | JSON object with event-specific metadata |
| client_ip | TEXT | IP address for remote operations, nullable |
| created_at | INTEGER | Unix epoch seconds |
| expires_at | INTEGER | Retention expiry timestamp for automatic cleanup |

**Retention enforcement**: A background task runs hourly to delete rows where
`expires_at < now()`. Minimum retention (90 days) is enforced at insert time;
the daemon calculates `expires_at = created_at + retention_seconds` based on
event_type. Manual deletion before expiry requires administrative override.

### subagent_rate_limits

Tracks rate limit state for auto-approve subagents. Token bucket counters
are stored per subagent and reset periodically.

```sql
CREATE TABLE subagent_rate_limits (
    subagent_id TEXT PRIMARY KEY REFERENCES subagents(id) ON DELETE CASCADE,
    tool_calls_bucket INTEGER NOT NULL DEFAULT 60,
    tool_calls_last_refill INTEGER NOT NULL,
    bash_bucket INTEGER NOT NULL DEFAULT 20,
    bash_last_refill INTEGER NOT NULL,
    write_bucket INTEGER NOT NULL DEFAULT 30,
    write_last_refill INTEGER NOT NULL,
    session_tool_calls INTEGER NOT NULL DEFAULT 0,
    session_limit INTEGER NOT NULL DEFAULT 1000
);
```

| Column | Type | Description |
|--------|------|-------------|
| subagent_id | TEXT PK/FK | References subagents(id), cascading delete |
| tool_calls_bucket | INTEGER | Current tokens for general tool calls |
| tool_calls_last_refill | INTEGER | Unix epoch of last bucket refill |
| bash_bucket | INTEGER | Current tokens for Bash commands |
| bash_last_refill | INTEGER | Unix epoch of last Bash bucket refill |
| write_bucket | INTEGER | Current tokens for Write/Edit operations |
| write_last_refill | INTEGER | Unix epoch of last write bucket refill |
| session_tool_calls | INTEGER | Total tool calls in session (monotonic) |
| session_limit | INTEGER | Maximum tool calls allowed in session |

**Token bucket algorithm**: On each tool call, the daemon refills buckets
based on time elapsed since last refill (1 token per second up to max),
then decrements the appropriate bucket. If bucket is empty, the call is
denied or queued based on configuration.

---

### todos

Stores task items created by the agent via the TodoWrite tool. These are
displayed in client UIs to show agent progress. Rows cascade-delete with
the session.

```sql
CREATE TABLE todos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    subject TEXT NOT NULL,
    description TEXT,
    active_form TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'in_progress', 'completed')),
    sequence INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX idx_todos_session ON todos(session_id, sequence);
```

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-incrementing row ID |
| session_id | TEXT FK | References sessions(id), cascading delete |
| subject | TEXT | Task title (imperative form, e.g. "Fix auth bug") |
| description | TEXT | Detailed task description, nullable |
| active_form | TEXT | Present continuous form shown during execution |
| status | TEXT | pending, in_progress, completed |
| sequence | INTEGER | Display order within the session |
| updated_at | INTEGER | Unix epoch seconds, updated on status change |

---

## Relay Database

The relay database lives at the relay server's configured data directory,
typically `/var/lib/betcode-relay/relay.db`. It stores user accounts,
authentication tokens, machine registrations, message buffers for offline
delivery, and mTLS certificate records.

The relay is the sole writer. Reads happen from request handlers. The
relay enforces row-level ownership: users can only access their own
machines, tokens, and certificates.

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
```

### users

User accounts for relay authentication. Passwords are hashed with argon2id
(PHC string format). The relay does not store plaintext passwords under any
circumstances.

```sql
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_login INTEGER
);

CREATE UNIQUE INDEX idx_users_email ON users(email);
```

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | UUIDv7 |
| email | TEXT UNIQUE | Login identifier, case-normalized |
| password_hash | TEXT | argon2id PHC string |
| created_at | INTEGER | Unix epoch seconds |
| last_login | INTEGER | Unix epoch seconds, nullable |

### tokens

JWT tracking table. The relay issues short-lived JWTs (15 minutes) with
longer-lived refresh tokens (7 days). Token hashes are stored for
revocation checks. The relay validates JWTs cryptographically first, then
checks this table for revocation.

```sql
CREATE TABLE tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0
        CHECK (revoked IN (0, 1))
);

CREATE INDEX idx_tokens_user ON tokens(user_id);
CREATE INDEX idx_tokens_expiry ON tokens(expires_at)
    WHERE revoked = 0;
```

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | UUIDv7 (also the JWT `jti` claim) |
| user_id | TEXT FK | References users(id), cascading delete |
| token_hash | TEXT UNIQUE | SHA-256 hash of the raw JWT string |
| expires_at | INTEGER | Unix epoch seconds |
| created_at | INTEGER | Unix epoch seconds |
| revoked | INTEGER | 0 = active, 1 = revoked |

A background task periodically deletes rows where `expires_at < now()`
to prevent unbounded table growth.

### machines

Registered development machines. A machine represents a daemon instance
that can accept remote sessions. Machines authenticate to the relay via
mTLS (see [certificates](#certificates)).

```sql
CREATE TABLE machines (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    hostname TEXT,
    owner_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    capabilities TEXT,
    last_seen INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'offline'
        CHECK (status IN ('online', 'offline', 'connecting')),
    created_at INTEGER NOT NULL
);

CREATE INDEX idx_machines_owner ON machines(owner_id);
CREATE INDEX idx_machines_status ON machines(owner_id, status)
    WHERE status = 'online';
```

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | UUIDv7 |
| name | TEXT | User-assigned machine name (e.g. "workstation", "laptop") |
| hostname | TEXT | OS hostname, nullable (informational) |
| owner_id | TEXT FK | References users(id) |
| capabilities | TEXT | JSON array of capability strings |
| last_seen | INTEGER | Unix epoch seconds, updated on heartbeat |
| status | TEXT | online, offline, connecting |
| created_at | INTEGER | Unix epoch seconds |

The `capabilities` field is a JSON array describing what the machine
supports, for example `["gpu", "docker", "large-context"]`. This is
informational and used by clients to display machine status.

### message_buffer

Buffered requests for machines that are currently offline. When a client
sends a request targeting an offline machine, the relay stores it here
with a configurable TTL (default 7 days). When the machine reconnects,
buffered messages are delivered in priority order (then FIFO within
priority) and marked as delivered.

```sql
CREATE TABLE message_buffer (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    machine_id TEXT NOT NULL REFERENCES machines(id) ON DELETE CASCADE,
    request_id TEXT NOT NULL,
    payload BLOB NOT NULL,
    message_type TEXT NOT NULL DEFAULT 'user_message',
    priority INTEGER NOT NULL DEFAULT 2
        CHECK (priority BETWEEN 0 AND 4),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    delivered INTEGER NOT NULL DEFAULT 0
        CHECK (delivered IN (0, 1))
);

CREATE INDEX idx_buffer_machine
    ON message_buffer(machine_id, delivered, priority, id);
CREATE INDEX idx_buffer_expiry ON message_buffer(expires_at)
    WHERE delivered = 0;
```

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-incrementing row ID |
| machine_id | TEXT FK | References machines(id) |
| request_id | TEXT | Correlation ID for request/response matching |
| payload | BLOB | Serialized gRPC request (protobuf bytes) |
| message_type | TEXT | Type hint: permission_response, cancel, user_message, etc. |
| priority | INTEGER | 0=highest (permission responses), 4=lowest (heartbeats) |
| created_at | INTEGER | Unix epoch seconds |
| expires_at | INTEGER | Unix epoch seconds (default: created_at + 604800 = 7 days) |
| delivered | INTEGER | 0 = pending, 1 = delivered |

**Priority Values:**

| Priority | Message Type | Description |
|----------|--------------|-------------|
| 0 | permission_response | Unblocks waiting agent operations |
| 1 | cancel_request | Time-sensitive user intent |
| 2 | user_message | Primary user interaction (default) |
| 3 | session_control | Session management operations |
| 4 | heartbeat | Background sync, lowest priority |

**Delivery Query (on daemon reconnect):**

```sql
SELECT id, request_id, payload FROM message_buffer
WHERE machine_id = ? AND delivered = 0
ORDER BY priority ASC, id ASC;  -- Priority first, then FIFO
```

A background task periodically deletes rows where
`expires_at < now() OR delivered = 1` to reclaim space.

### certificates

mTLS client certificates issued to machines. Each machine receives a
certificate signed by the relay's internal CA during registration. The
relay validates the certificate fingerprint on every tunnel connection.

```sql
CREATE TABLE certificates (
    id TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL REFERENCES machines(id) ON DELETE CASCADE,
    fingerprint TEXT NOT NULL UNIQUE,
    pem TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0
        CHECK (revoked IN (0, 1))
);

CREATE INDEX idx_certs_machine ON certificates(machine_id);
CREATE INDEX idx_certs_fingerprint ON certificates(fingerprint)
    WHERE revoked = 0;
```

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | UUIDv7 |
| machine_id | TEXT FK | References machines(id) |
| fingerprint | TEXT UNIQUE | SHA-256 fingerprint of the certificate DER |
| pem | TEXT | Full PEM-encoded certificate |
| expires_at | INTEGER | Unix epoch seconds |
| created_at | INTEGER | Unix epoch seconds |
| revoked | INTEGER | 0 = active, 1 = revoked |

---

## Client Database (Flutter/drift)

The client database lives on the mobile/desktop device, managed by the
drift package (Dart). It provides offline queueing, session caching for
disconnected viewing, and local preferences.

This schema is defined in Dart (drift table classes) but the logical
structure is documented here in SQL for reference. drift generates the
actual SQLite DDL.

### sync_queue

Offline command queue. When the device has no connectivity to the relay or
target machine, user actions are serialized and queued here. On reconnect,
the sync engine drains the queue in priority order (then FIFO within priority),
applying exponential backoff on failures. Default TTL is 7 days.

```sql
CREATE TABLE sync_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    machine_id TEXT NOT NULL,
    session_id TEXT,
    request_type TEXT NOT NULL,
    payload BLOB NOT NULL,
    priority INTEGER NOT NULL DEFAULT 3
        CHECK (priority BETWEEN 0 AND 5),
    sequence INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'sending', 'sent', 'blocked', 'failed')),
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);

CREATE INDEX idx_sync_status ON sync_queue(status, priority, sequence)
    WHERE status IN ('pending', 'blocked', 'failed');
CREATE INDEX idx_sync_expiry ON sync_queue(expires_at)
    WHERE status NOT IN ('sent', 'failed');
```

| Column | Type | Description |
|--------|------|-------------|
| id | INTEGER PK | Auto-incrementing row ID |
| machine_id | TEXT | Target machine identifier |
| session_id | TEXT | Target session, nullable (for session creation) |
| request_type | TEXT | gRPC method name (e.g. "SendMessage") |
| payload | BLOB | Serialized protobuf request bytes |
| priority | INTEGER | 0=highest (permission responses), 5=lowest (status) |
| sequence | INTEGER | Monotonic ordering for FIFO within priority |
| status | TEXT | pending, sending, sent, blocked, failed |
| retry_count | INTEGER | Number of delivery attempts |
| last_error | TEXT | Most recent error message, nullable |
| created_at | INTEGER | Unix epoch seconds |
| expires_at | INTEGER | Unix epoch seconds (default: created_at + 604800 = 7 days) |

**Priority Values (Client):**

| Priority | Request Type | Description |
|----------|--------------|-------------|
| 0 | permission_response | Unblocks agent, highest priority |
| 1 | question_response | Unblocks agent questions |
| 2 | cancel_request | Time-sensitive user intent |
| 3 | user_message | Primary interaction (default) |
| 4 | session_control | Management operations |
| 5 | heartbeat | Background sync, lowest priority |

**Drain Query:**

```sql
SELECT id, machine_id, session_id, request_type, payload
FROM sync_queue
WHERE status IN ('pending', 'blocked')
  AND expires_at > :now
ORDER BY priority ASC, sequence ASC;
```

**TTL Cleanup (run on app launch and periodically):**

```sql
DELETE FROM sync_queue WHERE expires_at < :now;
```

### cached_sessions

Local cache of session state for offline viewing. The client snapshots
session data (messages, status, token usage) on each sync so users can
review conversation history without connectivity.

```sql
CREATE TABLE cached_sessions (
    id TEXT PRIMARY KEY,
    machine_id TEXT NOT NULL,
    data TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX idx_cached_machine ON cached_sessions(machine_id);
```

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | Session ID (matches daemon session ID) |
| machine_id | TEXT | Machine the session belongs to |
| data | TEXT | JSON snapshot of session state |
| updated_at | INTEGER | Unix epoch seconds of last sync |

### machines (client)

Local bookmarks of known machines. This is the client-side view of
machines the user has connected to. It stores the relay URL and display
preferences. This table is independent of the relay's machines table.

```sql
CREATE TABLE machines (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    relay_url TEXT NOT NULL,
    last_connected INTEGER,
    is_favorite INTEGER NOT NULL DEFAULT 0
        CHECK (is_favorite IN (0, 1))
);
```

| Column | Type | Description |
|--------|------|-------------|
| id | TEXT PK | Machine ID (matches relay machine ID) |
| name | TEXT | Display name |
| relay_url | TEXT | URL of the relay this machine connects through |
| last_connected | INTEGER | Unix epoch seconds, nullable |
| is_favorite | INTEGER | 0 or 1, controls sort order in UI |

### settings

Key-value store for local application preferences: theme, default machine,
notification settings, and similar client-only configuration.

```sql
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

| Column | Type | Description |
|--------|------|-------------|
| key | TEXT PK | Setting identifier (e.g. "theme", "default_machine_id") |
| value | TEXT | JSON-encoded setting value |

---

## Design Decisions

### Why SQLite

SQLite is chosen for all three components because:

1. **Zero configuration**: No separate database server to deploy or manage.
2. **Single-file storage**: Easy backup (copy the file), easy migration.
3. **WAL mode**: Concurrent reads do not block the single writer, which
   matches all three components' access patterns.
4. **Compile-time query checking**: sqlx verifies every query against the
   schema at compile time, eliminating a class of runtime SQL errors.
5. **Embedded in binary**: No external dependencies, no version mismatches.

### Why UUIDv7 over ULID or auto-increment

UUIDv7 provides monotonic sortability (timestamp-based) with global
uniqueness. This matters for:

- **Session IDs**: Must be globally unique across machines for relay routing.
- **Machine IDs**: Must be unique across the relay's user base.
- **Sortability**: Natural chronological ordering without a separate
  timestamp index.

Auto-increment IDs are used only for tables where global uniqueness is not
required (messages, permission_grants, todos, sync_queue, message_buffer).

### Why INTEGER timestamps over TEXT (ISO 8601)

Integer Unix timestamps are:

- 8 bytes vs ~25 bytes for ISO 8601 strings, saving space at scale.
- Directly comparable with `<`, `>`, `BETWEEN` without parsing.
- Timezone-unambiguous (always UTC).
- Compatible with Rust's `std::time::SystemTime` and Dart's
  `DateTime.fromMillisecondsSinceEpoch` without string formatting.

### Why TEXT for JSON columns

SQLite's JSON1 extension is available but not guaranteed on all platforms.
Storing JSON as TEXT with application-layer parsing via serde (Rust) or
dart:convert (Dart) provides consistent behavior. The CHECK constraints
on enum columns catch invalid values at the database layer.

### Design Notes

- **No summaries table**: Context compaction is handled by Claude Code
  internally. The daemon stores raw NDJSON for replay; Claude resumes
  and re-summarizes on demand via `--resume`.
- **No hooks table**: Hook configuration is read from `.claude/settings.json`
  at session start. Config files are the source of truth.
- **No mcp_servers table**: MCP server config lives in `.mcp.json` and
  `$config_dir/mcp.json`. Process state is runtime-only.
- **connected_clients table**: Enables multiplexing — multiple clients
  (CLI, Flutter, headless) observe the same session with input lock.
- **messages as stream capture**: Stores raw NDJSON lines from Claude's
  stdout (`message_type` + `payload`), not parsed conversation turns.
  Preserves full fidelity for replay and reconnection.
- **Payload size limits**: The `messages.payload` column stores raw NDJSON lines
  which may contain large tool results. The daemon enforces a configurable max
  payload size (default 10 MB, `daemon.max_payload_bytes`) at the application
  layer before insertion. Messages exceeding this limit are truncated with a
  `[truncated: original_size=X bytes]` marker. SQLite handles TEXT up to 1 GB
  but this limit is not reached in practice.
- **Input lock denormalization**: The input lock is tracked in two places:
  `sessions.input_lock_client` (authoritative, used by the permission
  bridge) and `connected_clients.has_input_lock` (derived, used for
  client-facing queries). The daemon updates both atomically within a
  single transaction. On startup reconciliation, both are cleared.

---

## Migration Strategy

### Daemon and Relay (sqlx)

Both Rust components use
[sqlx embedded migrations](https://docs.rs/sqlx/latest/sqlx/migrate/index.html).
Migration files are compiled into the binary, eliminating the need to ship
SQL files alongside the executable.

**Directory structure**:

```
crates/betcode-daemon/migrations/
    20260201000000_initial.sql
    20260201000001_add_connected_clients.sql
    ...

crates/betcode-relay/migrations/
    20260201000000_initial.sql
    ...
```

**Naming convention**: `YYYYMMDDHHMMSS_description.sql`. Timestamps are
UTC. Each migration is a single SQL file containing both schema changes
and any necessary data transformations.

**Execution at startup**:

```rust
use sqlx::sqlite::SqlitePoolOptions;

let pool = SqlitePoolOptions::new()
    .max_connections(5)
    .connect("sqlite:daemon.db?mode=rwc")
    .await?;

// Run all pending migrations. Already-applied migrations are
// skipped based on the _sqlx_migrations table.
sqlx::migrate!("./migrations")
    .run(&pool)
    .await?;
```

sqlx creates a `_sqlx_migrations` table automatically to track which
migrations have been applied. Each migration runs inside a transaction
and is atomic: it either fully applies or fully rolls back.

**Rules for writing migrations**:

1. Migrations are append-only. Never edit or delete a migration that has
   been released.
2. Each migration must be idempotent in intent: if the daemon crashes
   mid-migration, restarting recovers cleanly (guaranteed by sqlx's
   transactional execution).
3. Destructive changes (DROP TABLE, DROP COLUMN) require a two-phase
   approach: deprecate in version N, remove in version N+1.
4. Data migrations (backfilling columns) go in the same file as the
   schema change when the transformation is simple. Complex data
   migrations get their own file.

**Compile-time verification**:

sqlx checks all queries against the live database at compile time via
`sqlx::query!` and `sqlx::query_as!`. After adding a migration, run:

```bash
cargo sqlx database create
cargo sqlx migrate run
cargo sqlx prepare  # generates .sqlx/ for CI without a live database
```

The generated `.sqlx/` directory is committed to version control so CI
can build without a live database.

### Client (drift)

The Flutter client uses drift's versioned schema migrations:

```dart
@DriftDatabase(tables: [SyncQueue, CachedSessions, Machines, Settings])
class AppDatabase extends _$AppDatabase {
  @override
  int get schemaVersion => 2;

  @override
  MigrationStrategy get migration => MigrationStrategy(
    onCreate: (m) => m.createAll(),
    onUpgrade: (m, from, to) async {
      if (from < 2) {
        await m.addColumn(syncQueue, syncQueue.lastError);
      }
    },
  );
}
```

drift generates a schema snapshot per version. The `drift_dev` CLI can
produce migration test helpers to verify that upgrades from any previous
version to the current version produce the correct schema.

---

## Entity Relationship Overview

```
DAEMON DATABASE
===============
worktrees 1──<0..* sessions
sessions  1──<0..* messages
sessions  1──<0..* permission_grants
sessions  1──<0..* todos
sessions  1──<0..* connected_clients
sessions  1──<0..* subagents (as parent)
sessions  1──<1    subagents (as own session)
sessions  1──<0..* orchestrations
subagents 1──<0..1 subagent_rate_limits
subagents 1──<0..* audit_log (via subagent_id)
sessions  1──<0..* audit_log (via session_id)
orchestrations 1──<1..* orchestration_steps
orchestration_steps 0..1──<1 subagents


RELAY DATABASE
==============
users     1──<0..* tokens
users     1──<0..* machines
machines  1──<0..* message_buffer
machines  1──<0..* certificates


CLIENT DATABASE
===============
(no foreign keys; machine_id/session_id are
 soft references to relay/daemon entities)
```

---

## Query Patterns

Key queries and the indexes that serve them.

**Daemon -- replay messages for a reconnecting client**:

```sql
SELECT payload FROM messages
WHERE session_id = ? AND sequence > ?
ORDER BY sequence ASC;
-- served by: idx_messages_session_seq (covering)
```

**Daemon -- check permission grant for a tool call**:

```sql
SELECT action FROM permission_grants
WHERE session_id = ? AND tool_name = ?
ORDER BY created_at DESC LIMIT 1;
-- served by: idx_grants_session_tool
```

**Daemon -- find stale clients for cleanup**:

```sql
SELECT client_id FROM connected_clients
WHERE last_heartbeat < ?;
-- served by: idx_clients_heartbeat
```

**Relay -- deliver buffered messages on machine reconnect**:

```sql
SELECT id, request_id, payload FROM message_buffer
WHERE machine_id = ? AND delivered = 0
ORDER BY id ASC;
-- served by: idx_buffer_machine
```

**Relay -- validate token (after JWT signature check)**:

```sql
SELECT revoked FROM tokens
WHERE id = ? AND expires_at > ?;
-- served by: PRIMARY KEY + idx_tokens_expiry
```

**Client -- drain sync queue**:

```sql
SELECT id, machine_id, session_id, request_type, payload
FROM sync_queue
WHERE status IN ('pending', 'failed')
ORDER BY sequence ASC;
-- served by: idx_sync_status
```

**Daemon -- forensic: all auto-approved actions by subagent**:

```sql
SELECT event_id, tool_name, tool_input_preview, outcome, created_at
FROM audit_log
WHERE subagent_id = ? AND event_type = 'auto_approve_tool_call'
ORDER BY created_at ASC;
-- served by: idx_audit_subagent
```

**Daemon -- security review: denied permissions in time range**:

```sql
SELECT * FROM audit_log
WHERE event_type IN ('permission_deny', 'tool_validation_fail')
  AND created_at BETWEEN ? AND ?
ORDER BY created_at DESC;
-- served by: idx_audit_type
```

**Daemon -- incident correlation: all events for parent session**:

```sql
SELECT * FROM audit_log
WHERE session_id = ? OR parent_session_id = ?
ORDER BY created_at ASC;
-- served by: idx_audit_session (partial), requires OR optimization
```

**Daemon -- retention cleanup (hourly background task)**:

```sql
DELETE FROM audit_log
WHERE expires_at < ?;
-- served by: idx_audit_expiry
```

**Daemon -- check rate limit state for subagent**:

```sql
SELECT tool_calls_bucket, tool_calls_last_refill,
       bash_bucket, bash_last_refill,
       session_tool_calls, session_limit
FROM subagent_rate_limits
WHERE subagent_id = ?;
-- served by: PRIMARY KEY
```

**Daemon -- decrement rate limit bucket (within transaction)**:

```sql
UPDATE subagent_rate_limits
SET tool_calls_bucket = tool_calls_bucket - 1,
    session_tool_calls = session_tool_calls + 1
WHERE subagent_id = ?
  AND tool_calls_bucket > 0
  AND session_tool_calls < session_limit;
-- Returns rows_affected=0 if rate limited
```
