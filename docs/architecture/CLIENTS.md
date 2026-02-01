# Client Applications

**Version**: 0.2.0
**Last Updated**: 2026-02-01
**Status**: Design Phase

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

## Flutter App (betcode_app)

### Directory Structure

```
betcode_app/
├── lib/
│   ├── main.dart
│   ├── app.dart
│   ├── generated/                    # Protobuf generated code
│   ├── core/
│   │   ├── grpc/
│   │   │   ├── client_manager.dart   # Channel lifecycle, reconnection
│   │   │   ├── relay_client.dart     # Relay connection with JWT
│   │   │   └── interceptors.dart     # JWT auth, logging, retry
│   │   ├── sync/
│   │   │   ├── sync_engine.dart      # Offline queue processor
│   │   │   └── connectivity.dart     # Network state monitor
│   │   ├── storage/
│   │   │   ├── database.dart         # drift (SQLite) ORM
│   │   │   └── secure_storage.dart   # Token/credential storage
│   │   └── auth/
│   │       └── auth_provider.dart    # JWT lifecycle
│   ├── features/
│   │   ├── conversation/             # Agent chat: streaming, tools, perms
│   │   ├── machines/                 # Machine list, status, switch
│   │   ├── worktrees/                # Worktree CRUD per machine
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
  grpc: ^3.1.0              # gRPC client
  protobuf: ^3.1.0          # Runtime
  flutter_riverpod: ^2.0.0  # State management
  drift: ^2.0.0             # SQLite ORM
  flutter_secure_storage: ^9.0.0
  connectivity_plus: ^6.0.0
  flutter_markdown: ^0.7.0
  flutter_highlight: ^0.8.0
  go_router: ^14.0.0
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

### Offline Sync Engine

```
User action --> write to local drift DB (instant feel)
  --> insert into sync_queue table
  --> sync engine checks connectivity
      ONLINE:  replay as gRPC calls (FIFO)
               success: mark synced
               failure: exponential backoff (1s -> 5s -> 30s -> 5min)
      OFFLINE: accumulate in queue
               on network return: 3s stability delay, then process
```

Queue limit: 500 pending events. Daemon is source of truth for conflicts.
Client schema in [SCHEMAS.md](./SCHEMAS.md) (`sync_queue`, `cached_sessions`).

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

### Push Notifications

Firebase (Android) / APNs (iOS) for events needing attention:

| Event | Notification |
|-------|-------------|
| `PermissionRequest` | "BetCode needs permission to run: cargo test" |
| `UserQuestion` | "BetCode asking: Which branch to target?" |
| `StatusChange(ERROR)` | "Session error: rate limit exceeded" |
| Task completion | "Finished: Fixed 12 lint errors" |

Relay dispatches notifications when no client holds the input lock.
Configurable per event type in Settings.

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

### Permission Response Flow

1. Daemon sends `PermissionRequest { request_id, tool_name, input }`.
2. Client shows permission UI (CLI: modal overlay, Flutter: bottom sheet).
3. User decides: `ALLOW_ONCE` | `ALLOW_SESSION` | `DENY`.
4. Client sends `PermissionResponse { request_id, decision }`.
5. Timeout: 60 seconds with no response -> auto-DENY (configurable).
6. Only the input-lock holder sees interactive permission prompts.
