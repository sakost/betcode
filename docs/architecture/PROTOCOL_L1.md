# Layer 1: Claude Code SDK Control Protocol

**Version**: 0.2.0
**Last Updated**: 2026-02-01
**Parent**: [PROTOCOL.md](./PROTOCOL.md)

## Overview

The daemon spawns Claude Code as a subprocess and communicates over stdin/stdout
using newline-delimited JSON (NDJSON). Each line is a complete JSON object.
This is Claude Code's official SDK protocol for programmatic integration.

### Invocation

```bash
claude -p "$prompt" \
  --output-format stream-json \
  --input-format stream-json \
  --permission-prompt-tool stdio \
  --include-partial-messages
```

| Flag | Purpose |
|------|---------|
| `-p "$prompt"` | Initial prompt (headless mode) |
| `--output-format stream-json` | NDJSON output on stdout |
| `--input-format stream-json` | NDJSON input on stdin |
| `--permission-prompt-tool stdio` | Route permission prompts to stdin/stdout |
| `--include-partial-messages` | Emit `stream_event` deltas for real-time streaming |

---

## Claude -> Daemon (stdout)

Every line on stdout is a JSON object with a `type` field. The daemon reads
stdout line by line, parses each object, and dispatches on `type`.

### 1. `system` (init)

Emitted once at session start. Contains session ID, tools, model, and cwd.

```json
{
  "type": "system",
  "subtype": "init",
  "session_id": "abc123-def456-...",
  "tools": [
    { "name": "Bash", "description": "Execute a bash command",
      "input_schema": { "type": "object", "properties": { "command": { "type": "string" } } } }
  ],
  "model": "claude-sonnet-4-20250514",
  "cwd": "/home/user/project"
}
```

**Daemon action**: Store `session_id`, register tool schemas, confirm model.

### 2. `assistant`

Complete assistant message with full `content` array (text + tool_use blocks).

```json
{
  "type": "assistant",
  "message": {
    "role": "assistant",
    "content": [
      { "type": "text", "text": "I'll read the file for you." },
      { "type": "tool_use", "id": "toolu_01ABC", "name": "Read",
        "input": { "file_path": "/home/user/project/src/main.rs" } }
    ],
    "stop_reason": "tool_use",
    "usage": { "input_tokens": 1250, "output_tokens": 87 }
  }
}
```

**`stop_reason` values**: `"end_turn"` (done), `"tool_use"` (tools pending),
`"max_tokens"` (truncated).

**Daemon action**: Parse content blocks. Stream text to client. Enter
permission/execution flow for tool_use blocks.

### 3. `user`

Tool results echoed back, confirming incorporation into conversation.

```json
{
  "type": "user",
  "message": {
    "role": "user",
    "content": [
      { "type": "tool_result", "tool_use_id": "toolu_01ABC",
        "content": "     1\tuse std::io;\n     2\tfn main() {\n     3\t    // entry\n     4\t}\n" }
    ]
  }
}
```

**Daemon action**: Log for debugging. No client-facing event needed.

### 4. `stream_event`

Partial streaming deltas (requires `--include-partial-messages`).

```json
{
  "type": "stream_event",
  "event": {
    "type": "content_block_delta",
    "index": 0,
    "delta": { "type": "text_delta", "text": "I'll read" }
  }
}
```

**Event subtypes:**
- `content_block_start` -- beginning of text or tool_use block
- `content_block_delta` -- incremental content (`text_delta` or `input_json_delta`)
- `content_block_stop` -- end of a content block
- `message_start` / `message_delta` / `message_stop` -- message-level boundaries

**Daemon action**: Translate `text_delta` into gRPC `TextDelta`. Buffer
`input_json_delta` to reconstruct tool inputs.

### 5. `control_request`

Permission check or user input needed. Each has a unique `request_id`.

**Permission request:**
```json
{
  "type": "control_request",
  "request_id": "req_001",
  "request": {
    "subtype": "can_use_tool",
    "tool_name": "Bash",
    "input": { "command": "cargo build --release" }
  }
}
```

**AskUserQuestion request:**
```json
{
  "type": "control_request",
  "request_id": "req_002",
  "request": {
    "subtype": "can_use_tool",
    "tool_name": "AskUserQuestion",
    "input": {
      "questions": [
        { "question": "Which database?", "options": ["PostgreSQL", "SQLite"], "multi_select": false }
      ]
    }
  }
}
```

**Multiplexing**: Multiple `control_request` messages can be in flight
simultaneously. Matched by `request_id`.

**Daemon action**: Evaluate permission engine. Auto-resolve if rule matches.
Forward to client if no rule or if AskUserQuestion. Track pending requests.

### 6. `result`

Session end. Contains duration, cost, and usage summary.

```json
{
  "type": "result",
  "subtype": "success",
  "session_id": "abc123-def456-...",
  "duration_ms": 45200,
  "total_cost_usd": 0.0847,
  "usage": { "input_tokens": 15400, "output_tokens": 3200,
    "cache_creation_input_tokens": 0, "cache_read_input_tokens": 12000 }
}
```

Error variant: `"subtype": "error"` with `"error": "Rate limit exceeded"`.

**Daemon action**: Persist summary to SQLite. Emit `UsageReport` +
`StatusChange(IDLE)` to clients.

---

## Daemon -> Claude (stdin)

### 1. `control_response`

Reply to a `control_request`. `request_id` must match.

**Allow:**
```json
{
  "type": "control_response",
  "response": {
    "subtype": "success", "request_id": "req_001",
    "response": { "behavior": "allow", "updatedInput": { "command": "cargo build --release" } }
  }
}
```

`updatedInput` is optional. Used to inject sandbox constraints, absolutize
paths, or (for AskUserQuestion) include the user's `answers` map.

**Deny:**
```json
{
  "type": "control_response",
  "response": {
    "subtype": "success", "request_id": "req_001",
    "response": { "behavior": "deny", "message": "User denied permission." }
  }
}
```

**AskUserQuestion response:**
```json
{
  "type": "control_response",
  "response": {
    "subtype": "success", "request_id": "req_002",
    "response": {
      "behavior": "allow",
      "updatedInput": {
        "questions": [{ "question": "Which database?", "options": ["PostgreSQL", "SQLite"] }],
        "answers": { "Which database?": "SQLite" }
      }
    }
  }
}
```

### 2. `user` (new prompt)

Continue a multi-turn conversation on an existing subprocess.

```json
{
  "type": "user",
  "message": { "role": "user", "content": "Now run the tests please." },
  "session_id": "abc123-def456-..."
}
```

Sent after a turn completes while the subprocess remains alive.
