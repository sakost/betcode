# Executable Specification: Permission Bridge

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Status**: Specification

---

## Purpose

Executable examples for BetCode's Permission Bridge behavior. Each scenario is
specific, testable, and covers ONE behavior.

---

## Feature: Permission Rule Matching

Rule format follows Claude Code native patterns:
- `"Bash(git *)"` - matches Bash with commands starting with `git `
- `"Edit(src/**/*.ts)"` - matches Edit with paths matching glob
- `"mcp__github__*"` - matches any MCP tool from github server
- `"Bash"` - matches ALL Bash invocations (no pattern)

### Background

```gherkin
Background:
  Given the daemon is running
  And a session "sess_001" exists with working_directory "/home/user/project"
  And a client is connected with the input lock
```

### Scenario: Exact tool name match allows request

```gherkin
Given the permission rules contain:
  | type  | rule  |
  | allow | Read  |
When Claude emits a control_request:
  | request_id | req_001 |
  | tool_name  | Read    |
  | input      | {"file_path": "/home/user/project/src/main.rs"} |
Then the daemon writes to Claude stdin within 10ms:
  """json
  {"type":"control_response","response":{"subtype":"success","request_id":"req_001","response":{"behavior":"allow"}}}
  """
And no PermissionRequest event is sent to the client
And the metrics counter "betcode_permission_decisions_total{decision=\"auto_allow\"}" increments by 1
```

### Scenario: Glob pattern match for Bash command

```gherkin
Given the permission rules contain:
  | type  | rule        |
  | allow | Bash(git *) |
When Claude emits a control_request:
  | request_id | req_002 |
  | tool_name  | Bash    |
  | input      | {"command": "git status"} |
Then the daemon writes allow to Claude stdin within 10ms
And no PermissionRequest event is sent to the client
```

### Scenario: Glob pattern match for file paths

```gherkin
Given the permission rules contain:
  | type  | rule              |
  | allow | Edit(src/**/*.ts) |
When Claude emits a control_request:
  | request_id | req_003 |
  | tool_name  | Edit    |
  | input      | {"file_path": "/home/user/project/src/components/Button.ts"} |
Then the daemon writes allow to Claude stdin within 10ms
```

### Scenario: Glob pattern does NOT match different extension

```gherkin
Given the permission rules contain:
  | type  | rule              |
  | allow | Edit(src/**/*.ts) |
When Claude emits a control_request:
  | request_id | req_004 |
  | tool_name  | Edit    |
  | input      | {"file_path": "/home/user/project/src/components/Button.tsx"} |
Then a PermissionRequest event is sent to the client:
  | request_id | req_004 |
  | tool_name  | Edit    |
And no control_response is written until the client responds
```

### Scenario: Deny rule takes precedence over allow rule

```gherkin
Given the permission rules contain:
  | type  | rule                   |
  | allow | Bash(git *)            |
  | deny  | Bash(git push --force) |
When Claude emits a control_request:
  | request_id | req_005 |
  | tool_name  | Bash    |
  | input      | {"command": "git push --force origin main"} |
Then the daemon writes to Claude stdin:
  """json
  {"type":"control_response","response":{"subtype":"success","request_id":"req_005","response":{"behavior":"deny","message":"Denied by rule: Bash(git push --force)"}}}
  """
And metrics counter "betcode_permission_decisions_total{decision=\"auto_deny\"}" increments
```

### Scenario: More specific rule takes precedence over general rule

```gherkin
Given the permission rules contain:
  | type  | rule           |
  | deny  | Bash           |
  | allow | Bash(npm test) |
When Claude emits a control_request:
  | request_id | req_006 |
  | tool_name  | Bash    |
  | input      | {"command": "npm test"} |
Then the daemon writes allow to Claude stdin
```

### Scenario: MCP tool wildcard matching

```gherkin
Given the permission rules contain:
  | type  | rule           |
  | allow | mcp__github__* |
When Claude emits a control_request:
  | request_id | req_007 |
  | tool_name  | mcp__github__create_issue |
Then the daemon writes allow to Claude stdin
```

### Scenario: MCP wildcard does NOT match different server

```gherkin
Given the permission rules contain:
  | type  | rule           |
  | allow | mcp__github__* |
When Claude emits a control_request:
  | request_id | req_008 |
  | tool_name  | mcp__gitlab__create_issue |
Then a PermissionRequest is sent to the client
```

### Scenario: Session grant overrides no-rule default

```gherkin
Given the permission rules are empty
And the session has a permission_grant:
  | tool_name | Edit  |
  | action    | allow |
When Claude emits a control_request for Edit tool
Then the daemon writes allow to Claude stdin
```

### Scenario: No matching rule forwards to client

```gherkin
Given the permission rules contain only allow for Read
When Claude emits a control_request for Write tool
Then a PermissionRequest event is sent to the client
And the pending_permissions map contains the request
```

---

## Feature: Permission Timeout Handling (Tiered Policy)

Permission timeout follows a tiered policy per [ADR-001](./decisions/ADR-001-permission-timeout.md):
- **Client connected**: 60 seconds (fast-fail for interactive sessions)
- **Client disconnected**: 7 days (mobile-first, activity extends TTL)

### Scenario: Permission times out with connected client (60s)

```gherkin
Given a client is connected with the input lock
When Claude emits a control_request:
  | request_id | req_020 |
  | tool_name  | Bash    |
And 60 seconds elapse without a PermissionResponse
Then the daemon writes to Claude stdin:
  """json
  {"type":"control_response","response":{"subtype":"success","request_id":"req_020","response":{"behavior":"deny","message":"Permission request timed out after 60 seconds."}}}
  """
And a PermissionTimeout event is sent to the client
And metrics counter "betcode_permission_decisions_total{decision=\"timeout\"}" increments
```

### Scenario: Permission persists with no client connected (7-day TTL)

```gherkin
Given no client is connected to the session
When Claude emits a control_request:
  | request_id | req_021 |
  | tool_name  | Bash    |
Then the permission request is stored in pending_permissions with 7-day TTL
And a push notification is sent to registered devices
And the daemon does NOT auto-deny after 60 seconds
# The request persists until: client responds, 7 days elapse, or session closes
```

### Scenario: Disconnected permission times out after 7 days

```gherkin
Given no client is connected to the session
And a pending permission request exists:
  | request_id | req_022 |
  | created_at | 7 days ago |
When the TTL expiry check runs
Then the daemon writes deny to Claude stdin:
  """json
  {"type":"control_response","response":{"subtype":"success","request_id":"req_022","response":{"behavior":"deny","message":"Permission request expired after 7 days."}}}
  """
And the pending permission is removed from pending_permissions
```

### Scenario: Client activity extends disconnected TTL

```gherkin
Given no client is connected to the session
And a pending permission request exists with TTL expiring in 1 day
When a client connects to the session
Then the pending permission TTL is reset to 7 days from now
And the PermissionRequest is replayed to the client with is_replay=true
```

### Scenario: Client responds before timeout

```gherkin
Given a client is connected with the input lock
When Claude emits a control_request
And the client sends PermissionResponse within 30 seconds:
  | request_id | req_022    |
  | decision   | ALLOW_ONCE |
Then the daemon writes allow to Claude stdin
And no PermissionTimeout event is sent
```

### Scenario: Session-scoped permission creates grant

```gherkin
Given a client is connected with the input lock
When Claude emits a control_request for Edit tool
And the client responds with ALLOW_SESSION
Then the daemon writes allow to Claude stdin
And a permission_grant row is inserted for the session
```

### Scenario: Timeout does not reset on reconnection

```gherkin
Given a client is connected with the input lock
When Claude emits a control_request at T+0s
And a PermissionRequest is sent to the client
And the client disconnects at T+30s
And the client reconnects at T+45s
And 15 more seconds elapse (total 60s from original request)
Then the daemon writes deny with timeout message
# Timeout started at original forward time, NOT at reconnection
```

---

## Feature: Reconnection with Pending Permissions

### Scenario: Pending permission replayed on reconnection

```gherkin
Given a client was connected with the input lock
And Claude emitted a control_request (req_030)
And a PermissionRequest was sent to the client
And the client disconnected before responding
When a client reconnects to the session
Then a PermissionRequest event is sent with:
  | request_id | req_030 |
  | is_replay  | true    |
```

### Scenario: Duplicate response after reconnection is idempotent

```gherkin
Given a permission request req_031 was already processed
And the client reconnects after network drop
When the client sends the same PermissionResponse again
Then the daemon logs "Ignoring duplicate PermissionResponse"
And no duplicate control_response is written to Claude stdin
And the daemon returns success (idempotent)
```

### Scenario: Response for expired request returns error

```gherkin
Given a permission request "req_032" timed out 10 seconds ago
When the client sends a PermissionResponse for req_032
Then the daemon returns error:
  | code    | PERMISSION_STALE           |
  | message | Permission request expired |
```

### Scenario: Multiple pending permissions replayed in order

```gherkin
Given the pending_permissions map contains:
  | request_id | received_at |
  | req_033    | T+0s        |
  | req_034    | T+5s        |
  | req_035    | T+10s       |
When a client reconnects
Then PermissionRequest events are sent in order: req_033, req_034, req_035
And all events have is_replay=true
```

---

## Undefined Behavior

1. **Malformed control_request from Claude** - Daemon may crash, log, or ignore
2. **Client sends invalid decision enum value** - May be DENY or INVALID_ARGUMENT
3. **Two clients send responses simultaneously** - First wins, second ignored

---

## Related Documents

- [DAEMON.md](./DAEMON.md) - Permission Bridge implementation
- [PROTOCOL_L1.md](./PROTOCOL_L1.md) - control_request/control_response format
- [TESTING_WORKTREES.md](./TESTING_WORKTREES.md) - Worktree test scenarios
