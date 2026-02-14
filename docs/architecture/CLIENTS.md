# Client Applications

**Version**: 0.1.0-alpha.1
**Last Updated**: 2026-02-14
**Status**: Implemented

BetCode ships two clients: a **Rust CLI** (ratatui TUI) and a **Flutter
mobile/web app**. Both are thin presentation layers over gRPC.

**Both clients connect to the betcode-daemon via gRPC. They do NOT talk to
the Anthropic API or Claude Code directly. The daemon owns the agent loop,
tool execution, and permission enforcement. Clients multiplex its I/O.**

Related docs:
[DAEMON.md](./DAEMON.md) | [PROTOCOL.md](./PROTOCOL.md) |
[TOPOLOGY.md](./TOPOLOGY.md) | [SECURITY.md](./SECURITY.md) |
[SCHEMAS.md](./SCHEMAS.md)

---

## CLI Client (betcode-cli)

### Crate Structure

```
betcode-cli/
├── src/
│   ├── main.rs              # clap argument parsing
│   ├── commands/
│   │   ├── mod.rs
│   │   ├── chat.rs          # Interactive TUI conversation
│   │   ├── run.rs           # Headless mode (-p "prompt")
│   │   ├── session.rs       # list, resume, compact, clear
│   │   ├── machine.rs       # list, switch
│   │   ├── worktree.rs      # create, switch, remove, list
│   │   ├── config.rs        # settings management
│   │   └── daemon.rs        # start, stop, status
│   ├── tui/
│   │   ├── mod.rs           # ratatui app loop, event dispatch
│   │   ├── input.rs         # Multi-line input, history, shortcuts
│   │   ├── render.rs        # Markdown + code block rendering
│   │   ├── permission.rs    # Permission prompt UI
│   │   ├── diff.rs          # Git diff viewer (inline + side-by-side)
│   │   └── progress.rs      # Tool execution indicators
│   └── connection.rs        # Local socket or relay connection
```

### TUI Design Philosophy

- **Performance over fanciness**: ratatui gives native terminal speed, no
  React/Ink overhead. Differential rendering writes only changed characters.
- **No PTY emulation**: renders structured `AgentEvent` data from gRPC, not
  raw terminal output. Full control over layout and scrolling.
- **Full keyboard navigation**: every action reachable without a mouse.
- **Cross-platform**: Windows (ConPTY + named pipe), macOS, Linux. Platform
  differences isolated to `connection.rs`.

### CLI Commands

```
betcode                          # Start interactive chat
betcode -p "prompt"              # Headless mode (single prompt, exits)
betcode --resume <session-id>    # Resume session in TUI
betcode --model opus             # Model override
betcode --continue               # Resume most recent session
betcode session list|resume|compact|clear
betcode machine list|switch <id>
betcode worktree list|create <branch>|switch <id>|remove <id>
betcode daemon start|stop|status
betcode config edit|show
```

### Connection Logic

```
1. Local daemon (socket/named pipe)?  --> Connect directly (fastest)
2. No daemon, relay URL configured?   --> Connect via relay
3. Neither?                           --> Offer: betcode daemon start
```

JWT for relay auth stored in OS keyring (preferred) or
`$BETCODE_CONFIG_DIR/auth.json` (fallback).
See [SECURITY.md](./SECURITY.md) for the full auth flow.

### TUI Architecture

Three concurrent tasks under `tokio::select!`:

1. **Terminal events** (crossterm): keyboard, resize. Dedicated thread.
2. **gRPC stream**: `AgentEvent` messages update state, trigger re-render.
3. **Tick timer** (100ms): drives spinner animations.

```
┌─ Status Bar ──────────────────────────────────────────┐
│ [session: abc123] [model: opus] [tokens: 1.2k/4k]    │
├───────────────────────────────────────────────────────┤
│ Conversation Pane (scrollable, vim keys j/k)          │
│                                                       │
│ > User: Fix the auth bug in login.rs                  │
│                                                       │
│ Assistant: I'll investigate...                         │
│ ┌─ Read src/auth/login.rs ─────────────────────────┐  │
│ │ (collapsed, Enter to expand)                     │  │
│ └──────────────────────────────────────────────────┘  │
├───────────────────────────────────────────────────────┤
│ Input Pane (multi-line, Ctrl+Enter to send)           │
│ > _                                                   │
└───────────────────────────────────────────────────────┘
```

**Key bindings**: `y/a/n` for permissions, `d` toggles diff mode,
`Tab` cycles focus, `Esc` closes overlays, `/` searches conversation.

### Markdown and Code Rendering

- Headers: bold, colored by level.
- Code blocks: syntax-highlighted via `syntect`, language from fence label.
- Edit tool results: diff coloring (green additions, red deletions).
- Inline code, bold, italic, lists, links all mapped to ratatui styles.

### Headless / SDK Mode

```bash
betcode -p "Fix all lint errors" \
  --output-format json \
  --model sonnet \
  --allowed-tools Read,Edit,Bash \
  --max-turns 20
```

| Format | Description |
|--------|-------------|
| `text` | Plain text, assistant messages only |
| `json` | Single JSON object on completion |
| `stream-json` | Newline-delimited JSON events (real-time) |

Exit codes: `0` success, `1` agent error, `2` connection failure,
`3` permission denied.

Tools not in `--allowed-tools` are auto-denied in headless mode.

---

## Flutter App (betcode_app) -- Separate Repository

The Flutter mobile app lives in its own repository. See the
[betcode_app repo](https://github.com/sakost/betcode-app) for build instructions
and its own `CLAUDE.md`.

### Directory Structure

```
betcode_app/
├── lib/
│   ├── main.dart
│   ├── app.dart
│   ├── generated/                    # Protobuf generated code (DO NOT EDIT)
│   ├── core/
│   │   ├── grpc/                     # Channel lifecycle, reconnection, interceptors
│   │   ├── sync/                     # Offline queue processor, connectivity monitor
│   │   ├── storage/                  # drift (SQLite) ORM, secure token storage
│   │   ├── auth/                     # JWT lifecycle
│   │   └── router.dart               # go_router navigation
│   ├── features/
│   │   ├── auth/                     # Authentication screens
│   │   ├── conversation/             # Agent chat: streaming, tools, perms
│   │   ├── machines/                 # Machine list, status, switch
│   │   ├── worktrees/                # Worktree CRUD per machine
│   │   ├── git_repos/                # Git repository browsing
│   │   ├── gitlab/                   # Pipelines, MRs, issues
│   │   ├── settings/                 # Permissions, MCP, models, relay
│   │   └── sessions/                 # Session list, search, resume
│   └── shared/
│       ├── theme/
│       └── widgets/
```

### Key Dependencies

```yaml
dependencies:
  grpc: ^5.1.0                    # gRPC client
  protobuf: ^6.0.0                # Runtime
  flutter_riverpod: ^3.2.1        # State management
  drift: ^2.31.0                  # SQLite ORM
  flutter_secure_storage: ^10.0.0
  connectivity_plus: ^7.0.0
  flutter_markdown_plus: ^1.0.7
  flutter_highlight: ^0.7.0
  go_router: ^17.1.0
```

### State Management (Riverpod)

```dart
// Connection state -> drives status badges and overlays
final connectionProvider = StreamProvider<ConnectionState>((ref) {
  return ref.read(grpcClientManager).connectionState;
});

// Conversation stream -> rebuilds chat on each AgentEvent
final conversationProvider = StreamProvider.family<AgentEvent, String>(
  (ref, sessionId) {
    return ref.read(agentServiceClient).converse(sessionId);
  },
);

// Machine/worktree/session lists -> fetched from daemon, cached locally
final machinesProvider = FutureProvider<List<Machine>>((ref) { ... });
final worktreesProvider = FutureProvider.family<List<WorktreeInfo>, String>(
  (ref, machineId) { ... },
);
final sessionsProvider = FutureProvider<List<SessionSummary>>((ref) { ... });

// Sync queue status -> pending count, errors, last sync time
final syncStatusProvider = StreamProvider<SyncStatus>((ref) {
  return ref.read(syncEngine).statusStream;
});
```

### Offline Sync Engine (Mobile-First)

```
User action --> write to local drift DB (instant feel)
  --> insert into sync_queue table with priority
  --> sync engine checks connectivity
      ONLINE:  replay as gRPC calls (priority order, then FIFO within priority)
               success: mark synced
               failure: exponential backoff (1s -> 5s -> 30s -> 5min)
      OFFLINE: accumulate in queue
               on network return: 3s stability delay, then process
```

**Priority Queue Order:**

| Priority | Event Type | Rationale |
|----------|------------|-----------|
| 0 (highest) | Permission responses | Unblocks agent work |
| 1 | User question responses | Unblocks agent work |
| 2 | Cancel requests | Time-sensitive |
| 3 | User messages | Primary user intent |
| 4 | Session management | Can wait |
| 5 (lowest) | Status/heartbeat | Background sync |

Queue limit: 500 pending events. Daemon is source of truth for conflicts.
Client schema in [SCHEMAS.md](./SCHEMAS.md) (`sync_queue`, `cached_sessions`).

**Queue TTL**: 7 days (configurable, matches permission TTL). Events older than TTL
are purged with user notification. This allows weekend disconnects without data loss.

### Offline Queue Conflict Resolution

The mobile client queues user actions when disconnected. On reconnect, these actions
replay against the daemon's authoritative state. This section defines conflict
resolution for all scenarios.

#### Conflict Resolution Principles

1. **Daemon is source of truth**: All session state lives in the daemon's SQLite.
2. **Client queue contains intents**: Queued items are "user wanted to do X", not
   "X happened".
3. **Stale intents are rejected**: If daemon state has diverged, the intent may no
   longer apply.
4. **User is notified**: Rejected actions surface as sync errors in the UI.

#### Conflict Scenarios

**Scenario 1: Message sent to closed session**

```
Given: Client queued UserMessage for session S1 while offline
  And: While offline, session S1 was closed by another client (CLI)
When: Client reconnects and replays the queued message
Then: Daemon returns NOT_FOUND error
  And: Client removes message from queue
  And: Client shows notification: "Session no longer exists. Message not sent."
  And: Client offers to create new session with the queued message content
```

**Scenario 2: Permission response for expired request**

```
Given: Client queued PermissionResponse for request R1 while offline
  And: Request R1 expired after 7-day TTL (or configured TTL) before client reconnected
When: Client reconnects and replays the permission response
Then: Daemon returns FAILED_PRECONDITION (request_id not found or already expired)
  And: Client removes response from queue
  And: Client shows notification: "Permission request expired after 7 days.
       You can ask Claude to retry the operation."
  And: Client offers "Retry Operation" action that sends a new user message
```

**Note**: With the mobile-first 7-day default TTL (configurable 1h-30d), this scenario
is rare. The TTL auto-extends on any client activity, so users who reconnect within
the TTL period will find their permission requests still pending and actionable.

**Scenario 3: Message sent while another client has input lock**

```
Given: Client A queued UserMessage while offline
  And: Client B connected and acquired input lock
When: Client A reconnects and replays the queued message
Then: Daemon returns PERMISSION_DENIED (no input lock)
  And: Client A keeps message in queue with status = 'blocked'
  And: Client A shows notification: "Another device is using this session."
  And: Client A offers "Request Input Lock" action
  And: If input lock acquired, Client A auto-retries queued message
```

**Scenario 4: Duplicate message (queue replayed twice)**

```
Given: Client queued UserMessage M1 with idempotency_key K1
  And: Client reconnected and successfully sent M1
  And: Network dropped before client marked M1 as 'sent'
When: Client reconnects again and replays M1 (same idempotency_key K1)
Then: Daemon detects duplicate (K1 already processed within 24h window)
  And: Daemon returns success (idempotent)
  And: Client marks M1 as 'sent'
```

**Scenario 5: Outdated worktree switch**

```
Given: Client queued SwitchWorktree to worktree W1 while offline
  And: While offline, worktree W1 was removed by CLI
When: Client reconnects and replays the switch request
Then: Daemon returns NOT_FOUND
  And: Client removes request from queue
  And: Client refreshes worktree list from daemon
  And: Client shows notification: "Worktree no longer exists."
```

**Scenario 6: Session compacted beyond queued resume point**

```
Given: Client cached session S1 at sequence 500
  And: While offline, session S1 was compacted (compaction_sequence = 1000)
When: Client reconnects and calls ResumeSession with from_sequence = 500
Then: Daemon returns SessionInfo with is_compacted = true
  And: Daemon does NOT replay sequences 500-999 (deleted)
  And: Client clears local cache and rebuilds from SessionInfo
  And: Client shows notification: "Session history was compacted."
```

#### Idempotency Key Implementation

All user-initiated actions include a client-generated idempotency key:

```protobuf
message UserMessage {
  string content = 1;
  repeated Attachment attachments = 2;
  string idempotency_key = 3;  // UUIDv7, generated at queue time
}
```

Daemon tracks processed keys in memory (24h TTL) and SQLite for persistence:

```sql
CREATE TABLE processed_idempotency_keys (
    key TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    processed_at INTEGER NOT NULL
);

-- Cleanup: DELETE WHERE processed_at < now() - 86400
```

#### Sync Queue State Machine

```
PENDING --> SENDING --> SENT
              |
              v
           BLOCKED (no lock) --> PENDING (lock acquired)
              |
              v
           FAILED (permanent) --> removed from queue, user notified
```

| Status | Meaning | Action |
|--------|---------|--------|
| PENDING | Awaiting sync | Process on next sync attempt |
| SENDING | In-flight to daemon | Wait for response |
| SENT | Successfully processed | Remove from queue |
| BLOCKED | Temporary failure (no lock, rate limit) | Retry with backoff |
| FAILED | Permanent failure (not found, invalid) | Remove, notify user |

#### Client UI for Conflicts

The sync status indicator shows:
- Green checkmark: All queued items synced
- Yellow spinner: Sync in progress
- Orange badge (count): Items blocked, awaiting resolution
- Red badge (count): Items failed, require user attention

Tapping the indicator opens a sync detail view listing each queued item with status
and available actions (retry, discard, view details).

### Mobile UI Screens

| Screen | Purpose |
|--------|---------|
| **Conversation** | Full agent chat with streaming, tool cards, permissions |
| **Sessions** | List/resume/delete sessions, search by content |
| **Machines** | View machines, switch active, status badges |
| **Worktrees** | Create/switch/remove worktrees per machine |
| **GitLab** | Pipeline jobs, MR diffs, issues |
| **Settings** | Permissions, MCP servers, models, relay config |

### Input Lock Transfer

Multiple clients can observe the same session, but only one holds the
**input lock** (can send messages and permission responses).

```
1. CLI (Client A) is chatting. It holds the input lock.
2. Flutter (Client B) opens same session -> read-only stream.
3. User taps "Request Input Lock" on Flutter.
4. Daemon notifies CLI: "Mobile requesting control" (10s timeout).
5a. CLI releases or times out -> lock transfers to Flutter.
    CLI switches to read-only. Flutter input bar activates.
5b. CLI rejects -> Flutter stays read-only.
```

No lock required for: viewing history, browsing machines/worktrees,
reading GitLab data, changing settings.

### Push Notifications (Mobile-First Design)

Firebase (Android) / APNs (iOS) for events needing attention:

| Event | Notification | Reminder Schedule |
|-------|-------------|-------------------|
| `PermissionRequest` | "BetCode needs permission to run: cargo test" | Initial, +1h, +24h |
| `UserQuestion` | "BetCode asking: Which branch to target?" | Initial, +1h, +24h |
| `StatusChange(ERROR)` | "Session error: rate limit exceeded" | Initial only |
| Task completion | "Finished: Fixed 12 lint errors" | Initial only |

Relay dispatches notifications when no client holds the input lock.
Configurable per event type in Settings.

#### Rate Limiting

Push notifications are rate-limited to prevent notification spam:

| Limit | Value | Scope | Rationale |
|-------|-------|-------|-----------|
| Max notifications per hour | 5 | Per session | Prevents runaway agent spam |
| Max notifications per day | 20 | Per user | Prevents notification fatigue |
| Reminder cooldown | 1 hour minimum | Per request | Prevents reminder spam |
| Collapse window | 5 minutes | Per session | Groups rapid-fire requests |

**Collapse behavior**: Multiple permission requests within 5 minutes collapse into a single
notification: "BetCode needs 3 permissions" with an expandable list. Tapping opens the
client to the permission queue.

**Rate limit exceeded behavior**: When rate limits are hit, the relay:
1. Continues buffering events for client delivery (no data loss)
2. Stops sending push notifications (prevents spam)
3. Logs a warning for observability
4. Sends a single "summary" notification after 1 hour: "BetCode has 7 pending items"

#### No Valid Device Tokens

When a user has no valid push tokens (uninstalled app, revoked permissions, etc.):

| Scenario | Relay Behavior | User Impact |
|----------|----------------|-------------|
| No tokens registered | Skip push, rely on email fallback | Gets email notification |
| All tokens invalid | Remove tokens, skip push, email fallback | Gets email notification |
| Some tokens invalid | Remove invalid, send to valid | Normal push delivery |
| Token refresh in progress | Queue notification, retry in 30s | Slight delay |

**Email Fallback** (optional, requires user opt-in):

```protobuf
message NotificationPreferences {
  bool push_enabled = 1;           // Default: true
  bool email_fallback = 2;         // Default: false
  string email_address = 3;        // Required if email_fallback = true
  EmailFrequency email_frequency = 4;
}

enum EmailFrequency {
  EMAIL_FREQUENCY_UNSPECIFIED = 0;
  IMMEDIATE = 1;      // Send email immediately when push fails
  HOURLY_DIGEST = 2;  // Batch into hourly digest
  DAILY_DIGEST = 3;   // Batch into daily digest
}
```

Email notifications are minimal: subject line + deep link. No sensitive content in email body.

#### Localization Strategy

The relay sends **localization keys**, not translated strings. The client renders the
final localized text. This enables:
- Client-side language preference without relay coordination
- Offline-capable localization (strings bundled in app)
- No relay dependency on translation services

**Push Payload Format:**

```json
{
  "notification_id": "notif_01HQ5X...",
  "event_type": "permission_request",
  "session_id": "sess_01HQ5...",
  "localization": {
    "title_key": "notification.permission.title",
    "body_key": "notification.permission.body",
    "body_args": {
      "tool_name": "Bash",
      "command_preview": "cargo test"
    }
  },
  "fallback": {
    "title": "Permission Required",
    "body": "BetCode wants to run: cargo test"
  }
}
```

**Client Localization Flow:**

```dart
// Flutter client
String localizeNotification(Map<String, dynamic> payload) {
  final localization = payload['localization'];
  final key = localization['body_key'];
  final args = localization['body_args'];

  // Try client-side localization first
  if (AppLocalizations.hasKey(key)) {
    return AppLocalizations.of(context).translate(key, args);
  }

  // Fall back to server-provided English text
  return payload['fallback']['body'];
}
```

**Supported Localization Keys:**

| Key | English Default | Args |
|-----|-----------------|------|
| `notification.permission.title` | "Permission Required" | - |
| `notification.permission.body` | "BetCode wants to run: {tool_name}" | `tool_name`, `command_preview` |
| `notification.permission.reminder` | "Reminder: Permission pending for {duration}" | `tool_name`, `duration` |
| `notification.question.title` | "Question from BetCode" | - |
| `notification.question.body` | "{question}" | `question`, `session_name` |
| `notification.error.title` | "Session Error" | - |
| `notification.error.body` | "{error_message}" | `error_message`, `session_name` |
| `notification.complete.title` | "Task Complete" | - |
| `notification.complete.body` | "Finished: {summary}" | `summary`, `session_name` |
| `notification.summary.title` | "BetCode Activity" | - |
| `notification.summary.body` | "You have {count} pending items" | `count` |

### Platform Notes

| Platform | Transport | Secure Storage | Background Sync | Push |
|----------|-----------|---------------|-----------------|------|
| Android (SDK 24+) | OkHttp | Keystore | WorkManager | FCM |
| iOS (15+) | URLSession | Keychain | BGTaskScheduler | APNs |
| Web | gRPC-Web | Encrypted localStorage | None | None |

Web has no offline sync (tab lifecycle is unpredictable) and no push.

---

## Shared Client Patterns

### gRPC Streaming Protocol

Both clients use the `Converse` bidirectional stream (see [PROTOCOL_L2.md](./PROTOCOL_L2.md)):

```
Client                              Daemon
  │── StartConversation ───────────>│
  │<── SessionInfo ─────────────────│
  │── UserMessage ─────────────────>│
  │<── StatusChange(THINKING) ──────│
  │<── TextDelta (streaming) ───────│
  │<── ToolCallStart ───────────────│
  │<── ToolCallResult ──────────────│
  │<── PermissionRequest ───────────│
  │── PermissionResponse ──────────>│
  │<── TextDelta ───────────────────│
  │<── UsageReport ─────────────────│
  │<── StatusChange(IDLE) ──────────│
```

Every `AgentEvent` carries a monotonic `sequence` number for reconnection.

### Reconnection Strategy

gRPC streams do not survive network interruptions. Both clients implement:

1. Record `last_received_sequence`.
2. Exponential backoff: 100ms -> 1s -> 5s -> 30s max.
3. Re-establish stream with `ResumeSession { session_id, last_sequence }`.
4. Daemon replays events from `last_sequence + 1`.
5. Client deduplicates by ignoring `sequence <= last_received`.

Events older than the most recent context compaction are not replayable.
Client receives a fresh `SessionInfo` snapshot instead.

### Permission Response Flow (Mobile-First)

1. Daemon sends `PermissionRequest { request_id, tool_name, input, expires_at }`.
2. Client shows permission UI (CLI: modal overlay, Flutter: bottom sheet + push notification).
3. User decides: `ALLOW_ONCE` | `ALLOW_SESSION` | `DENY`.
4. Client sends `PermissionResponse { request_id, decision }`.
5. **TTL**: Default 7 days (configurable 1h-30d). Auto-extends on ANY client activity.
6. **Reminders**: Push notifications at 1h and 24h if no response.
7. **Expiration**: After TTL, soft-deny allows retry. NOT a session-ending error.
8. Only the input-lock holder can respond, but ALL clients see pending requests.

**TTL Auto-Extension:**

The permission TTL automatically resets to its full duration whenever:
- The client reconnects to the session
- Any heartbeat is received for the session
- Any user message is sent to the session
- Any permission response is sent (for any request in the session)

This ensures users who are actively working never hit timeouts, while truly
abandoned requests eventually expire after the configured period.

**Permission Queue UI (Flutter):**

When multiple permission requests are pending, the Flutter client shows a queue:

```
┌─────────────────────────────────────────────────────┐
│ 3 Permissions Pending                    [Collapse] │
├─────────────────────────────────────────────────────┤
│ ● Bash: cargo test                      [Allow][Deny]│
│   Requested 2 hours ago                             │
├─────────────────────────────────────────────────────┤
│ ● Edit: src/main.rs                     [Allow][Deny]│
│   Requested 1 hour ago                              │
├─────────────────────────────────────────────────────┤
│ ● Bash: git push origin main            [Allow][Deny]│
│   Requested 5 minutes ago                           │
└─────────────────────────────────────────────────────┘
│ [Allow All]  [Deny All]  [Review Details]           │
└─────────────────────────────────────────────────────┘
```

**Offline Permission Response:**

If the client is offline when the user responds to a push notification:
1. Response is queued in `sync_queue` with high priority
2. On reconnect, queued permission responses are sent FIRST (before messages)
3. If the permission has expired server-side, client shows "Permission expired" toast
4. User can retry the operation by asking Claude to perform it again

---

## Rate Limit Client Behavior

When the daemon or relay returns a rate limit error, clients must implement backoff
and user notification.

### Rate Limit Response Format

Rate limits return gRPC status `RESOURCE_EXHAUSTED` with metadata:

```
grpc-status: 8 (RESOURCE_EXHAUSTED)
grpc-message: "Rate limit exceeded: 20 requests per hour for new sessions"
retry-after-ms: 180000
rate-limit-limit: 20
rate-limit-remaining: 0
rate-limit-reset: 1706745600
```

| Metadata Key | Type | Description |
|--------------|------|-------------|
| `retry-after-ms` | int | Minimum milliseconds before retry |
| `rate-limit-limit` | int | Total requests allowed in window |
| `rate-limit-remaining` | int | Requests remaining in current window |
| `rate-limit-reset` | int | Unix timestamp when window resets |

### Client Backoff Algorithm

```
base_delay = retry_after_ms from response (or 1000ms if not provided)
max_delay = 300000ms (5 minutes)
jitter_factor = 0.2

for attempt in 1..max_attempts:
    delay = min(base_delay * (2 ^ (attempt - 1)), max_delay)
    jitter = delay * jitter_factor * random(-1, 1)
    actual_delay = delay + jitter

    sleep(actual_delay)
    result = retry_request()

    if result.success or result.error != RESOURCE_EXHAUSTED:
        break

    base_delay = result.retry_after_ms or (base_delay * 2)
```

**Parameters:**
- `max_attempts`: 5 for interactive requests, 10 for background sync
- `max_delay`: 5 minutes (prevents excessive waits)
- `jitter_factor`: 20% (prevents thundering herd)

### User Notification

| Scenario | Notification | UI Element |
|----------|--------------|------------|
| First rate limit hit | "Slow down - too many requests. Retrying in Xs." | Toast/snackbar |
| Multiple retries | "Still rate limited. Will retry automatically." | Persistent banner |
| Max retries exhausted | "Request failed due to rate limits. Try again later." | Error dialog |
| Background sync limited | None (silent retry) | Sync status indicator |

### Per-Endpoint Limits (Reference)

From [SECURITY.md](./SECURITY.md), for client reference:

| Endpoint | Limit | Window | Scope |
|----------|-------|--------|-------|
| `Converse` (new session) | 20 | 1 hour | per user |
| `SpawnSubagent` | 50 | 1 hour | per parent session |
| Token refresh | 30 | 1 minute | per user |
| Registration/login | 10 | 1 minute | per IP |

### Proactive Rate Limit Awareness

Clients should track `rate-limit-remaining` from successful responses and warn users
before they hit limits:

```dart
// Flutter example
if (rateLimitRemaining < 3 && rateLimitRemaining > 0) {
  showWarning("You have $rateLimitRemaining sessions remaining this hour.");
}
```

### CLI Headless Mode

In headless mode (`betcode -p "prompt"`), rate limits cause immediate exit with
code 4 (RATE_LIMITED) and a stderr message including retry-after time:

```
Error: Rate limit exceeded. Retry after 180 seconds.
```

---

## Push Notification Semantics

Push notifications supplement the gRPC stream for events requiring user attention
when no client holds the input lock.

### Delivery Guarantees (Mobile-First)

| Property | Guarantee | Implementation |
|----------|-----------|----------------|
| Delivery | Best-effort, at-least-once | Relay retries failed sends |
| Ordering | Not guaranteed | FCM/APNs may reorder |
| Deduplication | Client-side | Notification ID checked against 24h cache |
| Persistence | Relay buffers for 7 days | Same as message_buffer TTL |
| Encryption | Transport-level | TLS to FCM/APNs |
| Reminders | Scheduled at 1h, 24h | For unanswered permission requests |
| Fallback | Email digest | When no valid push tokens |

**At-least-once semantics**: A notification may be delivered multiple times (FCM retry,
relay retry, network partition healing). Clients must handle duplicates gracefully.

**7-day buffer persistence** ensures users on vacation or with intermittent connectivity
still receive notifications when they reconnect, rather than losing them after 24 hours.

### Notification ID for Deduplication

Every push notification includes a unique ID:

```json
{
  "notification_id": "notif_01HQ5X...",
  "event_type": "permission_request",
  "session_id": "sess_01HQ5...",
  "request_id": "req_001",
  "timestamp": 1706745600,
  "title": "Permission Required",
  "body": "BetCode wants to run: cargo test"
}
```

Client deduplication:
1. On notification receive, check `notification_id` against local cache (1h TTL).
2. If seen, ignore notification (already processed).
3. If new, process notification and add `notification_id` to cache.

### Delivery Failure Handling

**Relay-side retry:**

```
Send to FCM/APNs
    |
    +-- Success (HTTP 200) --> Done
    |
    +-- Transient failure (HTTP 5xx, timeout)
    |       |
    |       v
    |   Retry with exponential backoff (1s, 2s, 4s, max 30s)
    |   Max 5 retries over ~1 minute
    |       |
    |       +-- Success --> Done
    |       +-- Max retries exhausted --> Log, mark failed, continue
    |
    +-- Permanent failure (HTTP 4xx, invalid token)
            |
            v
        Mark device token as invalid
        Remove from user's registered devices
        Log for user notification (stale device)
```

**Client-side handling:**
- App in foreground: Notification suppressed, gRPC stream has the data
- App in background: Notification displayed, tapping opens relevant screen
- App terminated: Notification displayed, tapping launches app with deep link

### Event Types and Priority (Mobile-First TTLs)

| Event Type | FCM Priority | APNs Priority | Collapse Key | Push TTL | Reminder Schedule |
|------------|--------------|---------------|--------------|----------|-------------------|
| `permission_request` | HIGH | time-sensitive | `perm_{session_id}` | 7d | Initial, +1h, +24h |
| `user_question` | HIGH | time-sensitive | `question_{session_id}` | 7d | Initial, +1h, +24h |
| `status_error` | HIGH | time-sensitive | `error_{session_id}` | 7d | Initial only |
| `task_complete` | NORMAL | passive | `task_{session_id}` | 7d | Initial only |
| `session_update` | NORMAL | passive | `session_{session_id}` | 24h | None |
| `permission_reminder` | NORMAL | time-sensitive | `perm_{session_id}` | 1h | N/A (is reminder) |

**Push TTL vs Permission TTL**: The push notification TTL (how long FCM/APNs retains
undelivered notifications) matches the 7-day permission TTL. This ensures users who
were offline for several days still receive the notification when their device reconnects.

**Collapse keys**: Multiple pending notifications of the same type for the same session
collapse into one. User sees "3 permission requests pending" rather than 3 separate
notifications.

### Multi-Path Delivery for Critical Events

Permission requests and errors use both push notifications AND relay-buffered
messages to maximize delivery probability:

```
Permission request arrives at daemon
    |
    v
Daemon checks: any client with input lock?
    |
    +-- Yes: Forward via gRPC stream (primary path)
    |
    +-- No:
        |
        +-- Buffer in daemon's pending_permissions map (for reconnecting clients)
        |
        +-- Send to relay: "No client for session S1, please notify user"
        |
        v
    Relay:
        +-- Buffer the permission request in message_buffer
        +-- Send push notification to all user's registered devices
        +-- When client connects, deliver buffered request via gRPC
```

This ensures the user is notified even if:
- The mobile app is terminated
- The gRPC connection drops
- Push notifications are delayed

### Device Token Management

Clients register push tokens with the relay:

```protobuf
message RegisterPushTokenRequest {
  string device_id = 1;       // Unique per device install
  string push_token = 2;      // FCM/APNs token
  PushPlatform platform = 3;  // FCM, APNS, APNS_SANDBOX
}

enum PushPlatform {
  PUSH_PLATFORM_UNSPECIFIED = 0;
  FCM = 1;
  APNS = 2;
  APNS_SANDBOX = 3;
}
```

Token refresh: Clients re-register on app launch and when FCM/APNs issues a new token.
Token invalidation: Relay removes token after 3 consecutive permanent delivery failures.

### User Preferences

Users configure which events trigger push notifications:

```protobuf
message PushPreferences {
  bool permission_requests = 1;  // Default: true
  bool user_questions = 2;       // Default: true
  bool errors = 3;               // Default: true
  bool task_completion = 4;      // Default: false
  bool session_updates = 5;      // Default: false
  repeated string muted_session_ids = 6;
}
```

Stored in relay's user preferences, synced to client's local settings.
