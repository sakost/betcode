# Security Architecture

**Version**: 0.2.0
**Last Updated**: 2026-02-01
**Status**: Design Phase

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
stderr or a configured log sink). These events support incident
investigation and compliance review.

**Logged events**:
- Authentication: login success/failure, token refresh, token revocation
- Authorization: permission grant/deny (including auto-approve for subagents)
- Session lifecycle: create, resume, complete, error, input lock transfer
- Subagent lifecycle: spawn, complete, fail, cancel (with auto_approve flag)
- Machine lifecycle: register, deregister, certificate issue/revoke
- Rate limit: threshold exceeded events

**Not logged** (to avoid secret exposure): API key values, JWT tokens,
request/response payloads, file contents from tool execution.

Each log entry includes: timestamp, event type, actor (user_id or
machine_id), session_id (if applicable), outcome (success/failure),
and a human-readable description.

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
