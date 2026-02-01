# Protocol Bridge, Streaming, and Reconnection

**Version**: 0.2.0
**Last Updated**: 2026-02-01
**Parent**: [PROTOCOL.md](./PROTOCOL.md)

## Protocol Bridge: Layer 1 <-> Layer 2

The daemon translates between gRPC streaming events and NDJSON control
messages. It is the only component that speaks both protocols.

```
                        DAEMON PROCESS
                    +-------------------+
  Client (gRPC)     |  gRPC Server      |    Claude subprocess
  =================>|  (tonic)          |
  AgentRequest      |       |           |
                    |       v           |
                    |  +-------------+  |    stdin (NDJSON)
                    |  |   Bridge    | =====> control_response, user
                    |  |   Logic     |  |
                    |  |             | <===== system, assistant, stream_event,
                    |  +-------------+  |     control_request, result
                    |       |           |    stdout (NDJSON)
  AgentEvent        |       v           |
  <=================|  gRPC Stream      |
  (streaming)       +-------------------+
```

### Translation: Layer 1 -> Layer 2

| NDJSON from Claude (stdout) | gRPC to Client |
|-----------------------------|----------------|
| `stream_event` (text_delta) | `AgentEvent.TextDelta` |
| `stream_event` (content_block_start, tool_use) | `AgentEvent.ToolCallStart` |
| `assistant` (tool_use blocks) | `AgentEvent.ToolCallStart` (if not already emitted) |
| Tool execution completes internally | `AgentEvent.ToolCallResult` |
| `control_request` (can_use_tool) | `AgentEvent.PermissionRequest` |
| `control_request` (AskUserQuestion) | `AgentEvent.UserQuestion` |
| `result` (success/error) | `AgentEvent.UsageReport` + `AgentEvent.TurnComplete` |
| Internal state changes | `AgentEvent.StatusChange` |

### Translation: Layer 2 -> Layer 1

| gRPC from Client | NDJSON to Claude (stdin) |
|-------------------|--------------------------|
| `AgentRequest.StartConversation` | Spawn subprocess, first `user` message |
| `AgentRequest.UserMessage` | `user` message |
| `AgentRequest.PermissionResponse(ALLOW)` | `control_response` with `behavior: "allow"` |
| `AgentRequest.PermissionResponse(DENY)` | `control_response` with `behavior: "deny"` |
| `AgentRequest.UserQuestionResponse` | `control_response` with `updatedInput.answers` |
| `AgentRequest.CancelRequest` | Send SIGINT to subprocess |

---

## Streaming Pattern: Full Conversation Turn

Complete message flow for a turn with auto-allowed tool execution.

```
  Client                    Daemon                    Claude
    |                         |                          |
    |--- AgentRequest ------->|                          |
    |    (UserMessage:        |--- user (NDJSON) ------->|
    |     "Build the project")|                          |
    |                         |<--- stream_event --------|
    |<-- AgentEvent ----------|    (text_delta)          |
    |    (StatusChange:       |                          |
    |     THINKING)           |                          |
    |<-- AgentEvent ----------|                          |
    |    (TextDelta)          |                          |
    |                         |<--- control_request -----|
    |                         |    (Bash, "cargo build") |
    |                         |                          |
    |                         |  [Permission Engine:     |
    |                         |   "Bash(cargo *)" match  |
    |                         |   -> AUTO ALLOW]         |
    |                         |                          |
    |<-- AgentEvent ----------|--- control_response ---->|
    |    (ToolCallStart)      |    (behavior: "allow")   |
    |<-- AgentEvent ----------|                          |
    |    (StatusChange:       |  [Bash executes]         |
    |     EXECUTING_TOOL)     |                          |
    |<-- AgentEvent ----------|                          |
    |    (ToolCallResult)     |                          |
    |                         |<--- assistant ------------|
    |<-- AgentEvent ----------|    (stop_reason:end_turn)|
    |    (TextDelta, final)   |                          |
    |                         |<--- result --------------|
    |<-- AgentEvent ----------|                          |
    |    (UsageReport)        |                          |
    |<-- AgentEvent ----------|                          |
    |    (TurnComplete)       |                          |
    |<-- AgentEvent ----------|                          |
    |    (StatusChange: IDLE) |                          |
```

### Turn with User Permission Prompt

When no allow/deny rule matches, the daemon asks the client.

```
  Client                    Daemon                    Claude
    |                         |                          |
    |                         |<--- control_request -----|
    |                         |    (Bash,"rm -rf target"|
    |                         |     request_id:"req_02")|
    |                         |                          |
    |                         |  [No matching rule->ASK] |
    |                         |                          |
    |<-- AgentEvent ----------|                          |
    |    (PermissionRequest:  |                          |
    |     request_id:"req_02")|                          |
    |<-- AgentEvent ----------|                          |
    |    (StatusChange:       |                          |
    |     WAITING_FOR_USER)   |                          |
    |                         |                          |
    |  [User reviews in UI]   |                          |
    |                         |                          |
    |--- AgentRequest ------->|                          |
    |    (PermissionResponse: |--- control_response ---->|
    |     ALLOW_SESSION)      |    (behavior: "allow")   |
    |                         |                          |
    |                         |  [Store session grant]   |
    |<-- AgentEvent ----------|                          |
    |    (ToolCallStart)      |  [Tool executes]         |
    |<-- AgentEvent ----------|                          |
    |    (ToolCallResult)     |                          |
```

---

## Reconnection Protocol

gRPC does not auto-resume server-streaming RPCs after the first message.
BetCode implements manual reconnection with sequence-based replay.

### Sequence Numbers

Every `AgentEvent` carries a monotonically increasing `sequence` (uint64).
Assigned per session. Persisted to SQLite alongside event data.

```
AgentEvent { sequence: 1, SessionInfo }
AgentEvent { sequence: 2, StatusChange(THINKING) }
AgentEvent { sequence: 3, TextDelta("Let me...") }
AgentEvent { sequence: 4, TextDelta(" check that.") }
AgentEvent { sequence: 5, ToolCallStart("Read") }
AgentEvent { sequence: 6, ToolCallResult(...) }
                    ^--- connection drops here
```

### Client Tracking

The client stores the last received `sequence`. Flutter persists it in the
drift database. The CLI holds it in memory.

### Reconnection Flow

```
  Client                    Daemon
    |                         |
    |  [Connection drops]     |
    |                         |
    |  [Backoff: 100ms,       |
    |   200ms, 400ms, 800ms,  |
    |   1.6s, 3.2s, 5s, 10s,  |
    |   30s (max)]            |
    |                         |
    |--- ResumeSession ------>|
    |    (session_id,         |
    |     from_sequence: 6)   |
    |                         |  [SQLite: SELECT * FROM messages
    |                         |   WHERE session_id=? AND sequence>6
    |                         |   ORDER BY sequence ASC]
    |                         |
    |<-- AgentEvent (seq: 7) -|  (replayed)
    |<-- AgentEvent (seq: 8) -|  (replayed)
    |<-- AgentEvent (seq: 9) -|  (live)
    |                         |
    |  [Client deduplicates   |
    |   by sequence number]   |
```

### Edge Cases

| Scenario | Behavior |
|----------|----------|
| Reconnect during tool execution | Replay up to current; new events stream live |
| Reconnect while awaiting permission | Replay `PermissionRequest`; client responds again |
| Reconnect after session ended | Replay all including `TurnComplete`; stream closes |
| Duplicate events (race) | Client deduplicates by sequence; idempotent processing |
| Session compacted between disconnect/reconnect | Replay from compaction point |

### Relay Tunnel Reconnection

The daemon-to-relay tunnel follows a similar pattern at the transport level:

1. Daemon detects tunnel failure (read error, heartbeat timeout).
2. Exponential backoff: 1s, 2s, 5s, 10s, 30s (max).
3. Re-establish `TunnelService.OpenTunnel` with mTLS credentials.
4. Re-send `RegisterRequest` to update connection registry.
5. Relay delivers buffered messages from `message_buffer` table.
6. Normal tunnel operation resumes.
