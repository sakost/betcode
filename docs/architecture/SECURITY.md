# Security Architecture

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Status**: Implemented

## Overview

BetCode wraps Claude Code as a subprocess. It does NOT handle LLM API
calls directly. Security concerns are in the transport/orchestration
layers: relay auth, daemon tunnels, local CLI access, tool permissions,
and secrets management.

---

## Authentication Layers

| Connection | Method | Details |
|------------|--------|---------|
| Claude Code -> Anthropic API | `ANTHROPIC_API_KEY` env var | Handled by Claude Code subprocess, not BetCode |
| Client -> Relay | TLS + JWT | BetCode relay account |
| Daemon -> Relay | mTLS | Machine certificate |
| CLI -> Local Daemon | OS socket permissions | Unix socket (Linux/macOS) / named pipe (Windows) |

### LLM API Auth (Claude Code Handles This)

BetCode passes LLM credentials to Claude Code via environment variables.
BetCode itself never calls the Anthropic API.

- `ANTHROPIC_API_KEY` -- direct API key (primary method)
- `ANTHROPIC_BASE_URL` -- custom endpoint (proxies, Bedrock, Vertex)
- `apiKeyHelper` -- shell script returning API key, read from
  `$config_dir/settings.json` (imported from Claude Code on first run)

BetCode does NOT touch Claude Code's OAuth tokens (`~/.claude.json`).

### Relay JWT Flow

1. User registers on BetCode relay (separate from Anthropic account)
2. Relay issues JWT with claims: `{ sub: user_id, email, iss: "betcode-relay",
   aud: "betcode", exp, iat, jti, machine_ids: ["m1", "m2"] }`.
   The `machine_ids` claim restricts which machines this token can access.
   Omitted = all machines owned by the user.
3. Client stores JWT in secure storage (Keychain/Keystore on Flutter)
4. JWT in gRPC `authorization` metadata header on every call
5. Relay validates signature, expiry, issuer, and revocation status
6. Token refresh via dedicated `RefreshToken` RPC before expiry

**Validation rules**: signature verification, `exp` check, `iss` match,
revocation check against `tokens` table (`revoked = 0`).

### mTLS Flow (Daemon -> Relay)

1. User registers machine via relay (JWT-authenticated)
2. Relay generates/accepts client certificate for the machine
3. Fingerprint stored in relay's `certificates` table
4. Daemon uses client cert when dialing relay tunnel endpoint
5. Relay validates cert chain + fingerprint against registered certs
6. Revocation: set `revoked = 1` in certificates table

### Certificate Rotation

Certificates have a finite lifetime (default: 1 year). The daemon monitors
certificate expiry and initiates renewal:

1. **Auto-renewal**: 30 days before expiry, the daemon generates a new
   keypair and sends a CSR to the relay (over the existing mTLS tunnel).
2. **Relay signs**: The relay validates the request (JWT-authenticated
   user must own the machine), issues a new certificate, stores the
   fingerprint in `certificates`.
3. **Hot swap**: The daemon loads the new certificate without restarting.
   The old certificate remains valid until its original expiry.
4. **Revocation on compromise**: User revokes via relay API. The relay
   sets `revoked = 1` and the daemon's next tunnel reconnection fails,
   prompting re-registration.

Manual rotation: `betcode daemon rotate-cert` forces immediate renewal.

**Daemon cert storage**: `$config_dir/certs/` (see platform-specific
config paths in [DAEMON.md](./DAEMON.md)) with 600 permissions on
Linux/macOS, restricted ACLs (owner-only) on Windows.

### Local CLI Authentication

No network auth needed. The daemon listens on a Unix socket or named
pipe with OS-level access control. Only the user running the daemon
can connect.

| Platform | Mechanism | Access Control |
|----------|-----------|----------------|
| Linux/macOS | Unix domain socket | Socket file permissions (owner-only) |
| Windows | Named pipe | DACL restricted to creating user's SID |

---

## Authorization

### Machine Access Control

- Users own machines (registered via relay)
- JWT `user_id` checked against machine `owner_id` on every request
- No cross-user machine access without explicit sharing grants

### Tool Permissions

Two enforcement layers:
1. **Daemon Permission Engine** -- pre-filters tool requests before
   Claude Code sees them (auto-allow, auto-deny, pattern matching)
2. **Claude Code's own system** -- handles remaining allow/deny decisions

**Permission hierarchy** (highest to lowest priority):
CLI flags > env vars > project local settings > project settings >
user settings > session grants > default ASK.

**Permission categories**:

| Category | Tools | Default |
|----------|-------|---------|
| ReadOnly | Read, Glob, Grep, TodoWrite | Auto-allowed |
| FileWrite | Write, Edit, NotebookEdit | Requires permission |
| BashExec | Bash, Skill | Per-command permission |
| Network | WebFetch, WebSearch | Requires permission |
| Mcp | MCP server tools | Inherits server permissions |

Session grants stored in daemon DB (`permission_grants` table), scoped
per session. Not forwarded to Claude Code -- daemon handles responses.

### Session Isolation

- Each session has own permission grants (no cross-session leakage)
- Worktree scoping: working directory locked per session
- Tools cannot access paths outside the session's directory tree

---

## Sandboxing

Claude Code handles its own tool sandboxing. BetCode adds three layers:

**Worktree directory enforcement**: Daemon sets `--cwd` on Claude Code
subprocess. Path traversal rejected after canonicalization. Symlinks
followed only within allowed directory tree.

**Pre-approved tool filtering**: Daemon evaluates requests before Claude
Code. Enables org-level policies: auto-deny `Bash(rm -rf /)`,
pattern-allow `Bash(git *)`, auto-allow all ReadOnly tools.

**Network restrictions (relay)**: TLS required on all connections.
No unauthenticated endpoints except registration/login (rate-limited).

## Rate Limiting

Rate limits protect the relay and daemon from abuse. Enforcement points:

| Endpoint | Limit | Scope | Action on Exceed |
|----------|-------|-------|------------------|
| Registration/login | 10/min | per IP | 429 + backoff header |
| Token refresh | 30/min | per user | 429 |
| Converse (new session) | 20/hour | per user | RESOURCE_EXHAUSTED |
| SpawnSubagent | 50/hour | per parent session | RESOURCE_EXHAUSTED |
| Tunnel registration | 5/min | per IP | Connection refused |

Daemon-local endpoints (Unix socket / named pipe) are not rate-limited
since they are protected by OS-level access control.

Implementation: token bucket per scope, stored in-memory (relay) or
per-session state (daemon). No shared state needed — each component
rate-limits independently.

## Audit Logging

Security-relevant events are logged to structured output (JSON lines to
stderr or a configured log sink) AND persisted to the daemon database
for forensic analysis. These events support incident investigation and
compliance review.

**Logged events**:
- Authentication: login success/failure, token refresh, token revocation
- Authorization: permission grant/deny (including auto-approve for subagents)
- Session lifecycle: create, resume, complete, error, input lock transfer
- Subagent lifecycle: spawn, complete, fail, cancel (with auto_approve flag)
- Machine lifecycle: register, deregister, certificate issue/revoke
- Rate limit: threshold exceeded events
- **Auto-approved tool calls**: Every tool invocation auto-approved by a subagent

**Not logged** (to avoid secret exposure): API key values, JWT tokens,
request/response payloads, file contents from tool execution.

Each log entry includes: timestamp, event type, actor (user_id or
machine_id), session_id (if applicable), outcome (success/failure),
and a human-readable description.

### Audit Log Persistence

Auto-approved operations require persistent audit trails for post-incident
forensics. The daemon stores audit logs in the `audit_log` table with
mandatory retention periods.

**Retention Policy**:

| Log Category | Minimum Retention | Maximum Retention | Rationale |
|--------------|-------------------|-------------------|-----------|
| Auto-approved tool calls | 90 days | 365 days | Forensic investigation window |
| Permission decisions | 90 days | 365 days | Compliance audit trail |
| Subagent lifecycle | 90 days | 365 days | Incident correlation |
| Authentication events | 90 days | 365 days | Security review |
| Rate limit events | 30 days | 90 days | Operational metrics |

**Retention is NOT optional**. The daemon refuses to start if the audit_log
table is missing or corrupted. Audit log deletion before the minimum
retention period requires explicit administrative override with a separate
audit trail entry recording the deletion request.

### Audit Log Schema

```sql
CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,           -- UUIDv7 for global correlation
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
    session_id TEXT,                          -- nullable for non-session events
    subagent_id TEXT,                         -- nullable for non-subagent events
    parent_session_id TEXT,                   -- for subagent correlation
    actor_type TEXT NOT NULL
        CHECK (actor_type IN ('user', 'daemon', 'subagent', 'system')),
    actor_id TEXT NOT NULL,                   -- user_id, subagent_id, or 'system'
    tool_name TEXT,                           -- for tool-related events
    tool_input_hash TEXT,                     -- SHA-256 of input (not raw input)
    tool_input_preview TEXT,                  -- First 256 chars, sanitized
    outcome TEXT NOT NULL
        CHECK (outcome IN ('success', 'failure', 'denied', 'timeout')),
    error_code TEXT,                          -- structured error identifier
    error_message TEXT,                       -- human-readable, max 1KB
    context TEXT,                             -- JSON object with event-specific data
    client_ip TEXT,                           -- for remote operations
    created_at INTEGER NOT NULL,              -- Unix epoch seconds
    expires_at INTEGER NOT NULL               -- Retention expiry (created_at + retention_days)
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

**Column Details**:

| Column | Purpose | Security Consideration |
|--------|---------|------------------------|
| event_id | Global correlation across components | UUIDv7 for ordering |
| tool_input_hash | Integrity verification | Never store raw sensitive input |
| tool_input_preview | Quick forensic review | Sanitized, truncated, no secrets |
| context | Event-specific metadata | JSON schema per event_type |
| expires_at | Retention enforcement | Automatic cleanup after period |

### Auto-Approve Audit Entry Example

```json
{
  "event_id": "019474a2-3b4c-7def-8901-234567890abc",
  "event_type": "auto_approve_tool_call",
  "severity": "warn",
  "session_id": "session-xyz",
  "subagent_id": "subagent-123",
  "parent_session_id": "parent-456",
  "actor_type": "subagent",
  "actor_id": "subagent-123",
  "tool_name": "Bash",
  "tool_input_hash": "sha256:a1b2c3...",
  "tool_input_preview": "cargo test --workspace",
  "outcome": "success",
  "context": {
    "allowed_tools": ["Read", "Bash", "Glob"],
    "auto_approve_config": {
      "max_duration_seconds": 3600,
      "rate_limit_remaining": 47
    },
    "execution_time_ms": 1234
  },
  "created_at": 1738540800,
  "expires_at": 1746316800
}
```

### Audit Log Query Patterns

**Forensic investigation -- all auto-approved actions by a subagent**:

```sql
SELECT * FROM audit_log
WHERE subagent_id = ? AND event_type = 'auto_approve_tool_call'
ORDER BY created_at ASC;
```

**Security review -- all denied permissions in time range**:

```sql
SELECT * FROM audit_log
WHERE event_type = 'permission_deny'
  AND created_at BETWEEN ? AND ?
ORDER BY created_at DESC;
```

**Incident correlation -- all events related to a parent session**:

```sql
SELECT * FROM audit_log
WHERE session_id = ? OR parent_session_id = ?
ORDER BY created_at ASC;
```

---

## Auto-Approve Subagent Security Hardening

Auto-approve is a **high-risk feature** that bypasses user confirmation for
tool calls. This section defines the mandatory security controls that make
auto-approve safe for production use.

**Security Principle**: Auto-approve should be treated as granting temporary,
scoped, audited sudo access to an autonomous agent. Every guardrail exists
because the alternative is unattended execution of arbitrary operations.

### Time-Boxing (Mandatory)

All auto-approve subagents have a maximum session duration. This is
**non-negotiable** -- there is no "unlimited" option for auto-approve.

| Parameter | Default | Range | Config Path |
|-----------|---------|-------|-------------|
| `max_auto_approve_duration_seconds` | 3600 (1 hour) | 60-14400 (1 min - 4 hours) | `subagents.max_auto_approve_duration` |

**Enforcement**:
1. Timer starts at subagent spawn, not first tool call
2. At expiration: daemon sends `SIGTERM`, logs `subagent_timeout` audit event
3. Grace period: 10 seconds for cleanup before `SIGKILL`
4. No extension mechanism -- spawn a new subagent if more time needed

**Rationale**: Unbounded auto-approve sessions are a security incident waiting
to happen. A compromised or malfunctioning subagent with unlimited time can
cause unbounded damage. One hour is sufficient for most automated tasks; longer
tasks should checkpoint and spawn fresh subagents.

### Tool Validation (Spawn-Time AND Runtime)

The `allowed_tools` list is validated at two points:

**Spawn-Time Validation**:
- All tool names in `allowed_tools` must exist in the daemon's tool registry
- Unknown tools reject the spawn request with `INVALID_ARGUMENT`
- Tool registry is loaded from Claude Code's capabilities on daemon start

**Runtime Validation** (per tool call):
- Before auto-approving, daemon verifies the tool still exists
- If tool was removed/deprecated since spawn: deny with `tool_validation_fail` audit event
- Subagent receives error response, can continue with other tools

```protobuf
// Error returned when a previously-valid tool is no longer available
message ToolValidationError {
  string tool_name = 1;
  string reason = 2;  // "deprecated", "removed", "disabled"
  int64 deprecated_at = 3;  // Unix timestamp, 0 if not deprecated
}
```

**Tool Registry Updates**:
- Daemon reloads tool registry on `SIGHUP` or config reload
- Active subagents are NOT terminated on registry update
- New tool calls are validated against the updated registry
- Audit log records registry update events

### Mid-Execution Permission Revocation

Auto-approve permissions can be revoked without terminating the subagent.
This enables graceful degradation: the subagent continues running but
must request user confirmation for subsequent tool calls.

**Revocation Mechanism**:

```protobuf
message RevokeAutoApproveRequest {
  string subagent_id = 1;
  string reason = 2;            // Required: audit trail
  bool terminate_if_pending = 3; // Kill subagent if tool call in flight
}

message RevokeAutoApproveResponse {
  bool revoked = 1;
  int32 pending_tool_calls = 2; // Number of in-flight calls at revocation
}
```

**Revocation Behavior**:

| Scenario | Behavior |
|----------|----------|
| No tool call in progress | Revoke immediately, subagent continues with manual approval |
| Tool call in flight, `terminate_if_pending=false` | Let call complete, revoke after |
| Tool call in flight, `terminate_if_pending=true` | Send SIGTERM, revoke, terminate |
| Subagent already completed | No-op, return `revoked=false` |

**Audit Trail**: Revocation creates a `permission_revoke` audit entry with:
- Revoking actor (user_id or system)
- Reason text (required, min 10 characters)
- Subagent state at revocation
- Count of tool calls executed before revocation

### Rate Limiting for Auto-Approved Operations

Individual auto-approved tool calls are rate-limited to prevent runaway
execution. This is separate from the `SpawnSubagent` rate limit.

| Limit Type | Default | Range | Scope |
|------------|---------|-------|-------|
| Tool calls per minute | 60 | 10-300 | Per subagent |
| Tool calls per session | 1000 | 100-10000 | Per subagent lifetime |
| Bash commands per minute | 20 | 5-60 | Per subagent |
| Write/Edit operations per minute | 30 | 10-100 | Per subagent |

**Enforcement**:
- Token bucket algorithm per limit type
- On limit exceeded: tool call queued (if queue enabled) or denied
- Audit log records `rate_limit_exceeded` with tool name and limit type
- Subagent notified via error response (can retry after backoff)

**Configuration**:

```toml
[subagents.rate_limits]
tool_calls_per_minute = 60
tool_calls_per_session = 1000
bash_per_minute = 20
write_per_minute = 30
queue_on_limit = false  # false = deny, true = queue with backoff
```

### Defense-in-Depth Layers

Auto-approve security uses multiple independent layers. Compromise of one
layer does not grant unrestricted access.

```
Layer 1: Spawn Validation
    - allowed_tools must be non-empty
    - All tools must exist in registry
    - Time limit must be within bounds

Layer 2: Runtime Validation
    - Tool existence check before each call
    - Rate limit check before each call
    - Session timeout check before each call

Layer 3: Execution Constraints
    - Worktree boundary enforcement
    - Permission engine pattern rules still apply
    - Bash blocklist patterns still evaluated

Layer 4: Audit Trail
    - Every auto-approved call logged
    - Hash of input for integrity
    - 90-day minimum retention

Layer 5: Revocation
    - Mid-execution permission revoke
    - Graceful degradation to manual approval
    - Forced termination option
```

### Auto-Approve Checklist (Required Before Enabling)

Before setting `auto_approve_permissions = true` on any subagent:

- [ ] `allowed_tools` is explicitly specified (empty list = INVALID)
- [ ] Each tool in `allowed_tools` is reviewed for safety in unattended mode
- [ ] `max_turns` is set to a reasonable limit (not 0/unlimited)
- [ ] Time limit is appropriate for the task (default 1 hour)
- [ ] Worktree is scoped appropriately (not root directory)
- [ ] Bash patterns (if Bash allowed) are constrained via permission engine
- [ ] Monitoring is configured to alert on `rate_limit_exceeded` events
- [ ] Incident response procedure exists for runaway subagent scenarios

---

## Input Validation

All gRPC request fields are validated before processing. The daemon and
relay reject malformed requests with `INVALID_ARGUMENT` status.

**Validation rules**:
- String fields: UTF-8 validity, length limits (session_id: 128, prompt: 1MB,
  tool names: 256 chars)
- Path fields: canonicalized, checked against session worktree boundary
- Enum fields: must match defined values (reject unknown variants)
- Repeated fields: bounded cardinality (e.g. allowed_tools: max 100,
  orchestration steps: max 50)
- Map fields: key/value length limits, bounded entry count

**Environment variable passthrough**: The daemon passes a fixed set of
env vars to Claude subprocesses (`ANTHROPIC_API_KEY`, `ANTHROPIC_BASE_URL`,
`PATH`, platform essentials). Arbitrary env injection via `SpawnSubagentRequest.env`
is restricted to keys matching an allowlist pattern. Keys starting with
`ANTHROPIC_`, `BETCODE_`, or `CLAUDE_` are prohibited from override.

---

## Connection Resilience

### State Machine

```
DISCONNECTED -> CONNECTING -> AUTHENTICATING -> CONNECTED
                                                    |
                                              error/timeout
                                                    |
                                                    v
                                              RECONNECTING
                                       (exp backoff + jitter)
                                                    |
                                                    v
                                               CONNECTED
```

### Keepalive

| Layer | Interval | Timeout | Purpose |
|-------|----------|---------|---------|
| HTTP/2 PING | 30s | 20s | Detect dead TCP connections |
| TCP keepalive | 60s | - | OS-level liveness |
| App heartbeat | 15s | 10s | Detect stale gRPC streams |
| Tunnel heartbeat | 20s | 15s | Daemon-relay liveness |

### Stream Reconnection

- `AgentEvent` messages carry sequence numbers
- Client tracks last received sequence per session
- On disconnect: exponential backoff, re-establish stream with
  `last_sequence` parameter
- Server replays events from SQLite; client deduplicates
- Guarantees at-least-once delivery (clients handle idempotently)

### Offline Queue (Client)

Messages queued in `sync_queue` SQLite table while disconnected.
Replayed in FIFO order on reconnect (2-5s stability delay first).
Failed replays use exponential backoff and remain queued.

### Relay Message Buffer

Requests for offline daemons buffered in `message_buffer` table with
24h default TTL. Delivered in order when daemon reconnects. Expired
messages purged on periodic cleanup.

---

## Secrets Management

### Principles

- Never persist API keys in databases (env vars at runtime only)
- Never auto-commit: `.env`, `*.key`, `*.pem`, `credentials.*`
- argon2id for password hashing (memory: 64 MiB, iterations: 3)
- Minimize secret lifetime (JWT expiry, cert expiry, session scoping)

### Storage by Component

| Component | Secret | Storage | Protection |
|-----------|--------|---------|------------|
| Client (Flutter) | JWT | flutter_secure_storage | Keychain / Keystore |
| Client (CLI) | JWT | OS credential store | Platform secure storage |
| Daemon | mTLS key | Config dir `certs/` subdirectory | File permissions 600 / ACL |
| Daemon | API key | Environment variable | Not persisted to disk |
| Relay | Passwords | `users.password_hash` | argon2id hash |
| Relay | JWT signing key | Env var or HSM | Not in database |

### API Key Flow

```
User sets ANTHROPIC_API_KEY (or configures apiKeyHelper)
  -> Daemon reads at session start
  -> Passed to Claude Code subprocess via environment
  -> Exists only in process memory, never written to disk
```

### Payload Sensitivity

The `messages` table stores raw NDJSON from Claude's stdout, which may
include file contents (via Read tool results), command output (via Bash),
and other potentially sensitive data. BetCode does NOT redact stored
payloads because faithful replay requires the original content.

**Mitigations**:
- The daemon database is local to the machine with owner-only file
  permissions (600 on Unix, ACL on Windows).
- The relay never sees or stores NDJSON payloads — it only routes
  opaque gRPC frames.
- Database encryption at rest is recommended for sensitive environments
  (SQLite Encryption Extension or OS-level disk encryption).
- Session deletion (`betcode session clear`) removes all messages
  via CASCADE.

---

## Windows-Specific Security

- **Named pipes**: `\\.\pipe\betcode-daemon-{user_id}` with DACL
  restricting to creator's SID. `PIPE_REJECT_REMOTE_CLIENTS` flag set.
- **Certificate storage**: `%USERPROFILE%\.betcode\certs\` (default config dir) with
  owner-only ACL, inherited permissions removed. NOT stored in Windows
  Certificate Store (portability and explicit lifecycle control).
- **File permissions**: `SetNamedSecurityInfo` with explicit DACL,
  inherited ACEs removed, owner-only access by current user SID.

---

## Threat Model

### Trust Boundaries

| Boundary | Mechanism | Primary Threats | Mitigations |
|----------|-----------|-----------------|-------------|
| Client <-> Relay | TLS + JWT | Token theft, replay, credential stuffing | TLS, JWT expiry, rate limiting, revocation |
| Relay <-> Daemon | mTLS | Machine impersonation, cert theft | Mutual TLS, fingerprint validation, revocation |
| CLI <-> Daemon | Local IPC | Privilege escalation, IPC hijacking | OS socket/pipe permissions |
| Daemon <-> Claude Code | Subprocess | Tool abuse, path traversal, injection | Worktree enforcement, permission pre-filter |

### Attack Vectors

| Vector | Severity | Mitigation |
|--------|----------|------------|
| JWT token theft | High | Short expiry, TLS-only, secure storage |
| mTLS cert theft | High | File permissions, cert revocation |
| Relay compromise | Critical | mTLS prevents daemon impersonation, JWT prevents user impersonation |
| Path traversal | High | Canonicalization, worktree boundary check |
| Command injection (Bash) | High | Permission engine patterns, blocklist |
| Subagent auto-approve abuse | High | Audit logging, per-session scope, tool allowlist required with auto-approve |
| DoS on relay | Medium | Rate limiting, per-user connection limits |
| Offline message tampering | Medium | Protobuf serialization, TLS in transit |
| Local IPC hijacking | Low | OS-enforced socket/pipe permissions |

---

## Security Checklist

**Code review**:
- No hardcoded secrets or credentials in source
- No plaintext password storage (argon2id required)
- No disabled TLS or cert validation (including tests)
- File permission enforcement on credential files
- Input validation on all gRPC request fields
- Path canonicalization before filesystem access
- Rate limiting on authentication endpoints

**Deployment**:
- Relay TLS cert from trusted CA (not self-signed in prod)
- JWT signing key in env var or HSM (not in config files)
- mTLS CA key offline or in HSM
- Database encryption at rest for relay
- Log sanitization (no secrets in output)
- Firewall limiting relay to required ports

---

## References

- [TOPOLOGY.md](./TOPOLOGY.md) -- connection modes, relay architecture
- [DAEMON.md](./DAEMON.md) -- permission bridge, subprocess management
- [SCHEMAS.md](./SCHEMAS.md) -- certificates, tokens, permission_grants
- [PROTOCOL.md](./PROTOCOL.md) -- gRPC API, auth metadata headers
- [CLIENTS.md](./CLIENTS.md) -- client secure storage
- [OVERVIEW.md](./OVERVIEW.md) -- system overview
