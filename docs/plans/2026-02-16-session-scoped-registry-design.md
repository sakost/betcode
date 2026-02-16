# Session-Scoped Command Registry Design

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `CommandRegistry` session-aware so multiple concurrent sessions can have different MCP tools, plugins, and skills without overwriting each other.

**Architecture:** Layered registry — one shared base layer (builtins, service commands) plus per-session layers (MCP, plugins, skills). Queries merge base + session at read time.

**Tech Stack:** Rust, tonic/prost (proto changes), tokio RwLock

---

## Problem

`CommandRegistry` is a single `Arc<RwLock<CommandRegistry>>` shared across all sessions. When session B starts, `registry.clear_source("mcp")` wipes session A's MCP entries. Different worktrees with different `.claude/` configs and MCP servers conflict.

## Data Model

```rust
pub struct CommandRegistry {
    /// Daemon-wide entries: builtins (cd, pwd, exit-daemon, reload-remote)
    /// and Claude Code capabilities discovered at startup.
    base_entries: Vec<CommandEntry>,

    /// Per-session entries keyed by session_id.
    /// Contains MCP tools, plugins, and skills specific to each session's
    /// working directory and Claude Code instance.
    session_layers: HashMap<String, Vec<CommandEntry>>,
}
```

### Key Methods

| Old | New | Behavior |
|-----|-----|----------|
| `get_all()` | `get_for_session(session_id: &str)` | Returns base + session entries merged |
| `add(entry)` | `add(entry)` (unchanged) | Adds to base layer |
| — | `set_session_entries(session_id, entries)` | Replaces entire session layer |
| `clear_source("mcp")` | (removed) | Replaced by `set_session_entries` |
| `clear_plugin_sources()` | (removed) | Replaced by `set_session_entries` |
| — | `remove_session(session_id)` | Drops session layer on exit |
| `search(query, max)` | `search_for_session(session_id, query, max)` | Searches base + session |

### Proto Changes

```protobuf
message GetCommandRegistryRequest {
  string session_id = 1;  // required
}

message ExecuteServiceCommandRequest {
  string command = 1;
  repeated string args = 2;
  string session_id = 3;  // new
}
```

## Data Flow

### Session Start (pipeline.rs)

1. `system_init` arrives from Claude Code subprocess
2. `EventBridge` extracts MCP entries from `init.tools` (unchanged)
3. Plugin/skill entries discovered from `init.cwd`'s `.claude/` directory
4. All session-scoped entries set in one call:
   ```rust
   let entries = [mcp_entries, plugin_entries, skill_entries].concat();
   let mut registry = command_registry.write().await;
   registry.set_session_entries(session_id, entries);
   ```

### Plugin/Skill Discovery

Moves from daemon startup to session init. Each session discovers plugins from its own `cwd`'s `.claude/` directory (provided in `system_init`). Different worktrees naturally get different plugins.

Daemon startup still discovers builtins and Claude Code capabilities for the base layer.

### `reload-remote` Service Command

1. Runs `claude_code --capabilities` → updates **base** entries
2. Re-discovers plugins/skills from calling session's `cwd` → updates that **session layer**
3. MCP tools not refreshed here (they come from `system_init` only)

### Session End (cleanup)

When subprocess exits (EOF on stdout reader):
```rust
{
    let mut registry = command_registry.write().await;
    registry.remove_session(&session_id);
}
```

Covers: normal completion, crash, kill, `/clear` triggering new session.

### CLI Side

The TUI already tracks `session_id`. Passes it in `GetCommandRegistryRequest` and `ExecuteServiceCommandRequest`. On `/clear` (dual-dispatch), old session layer cleaned up on subprocess exit, new session populates its own layer on `system_init`.

## Edge Cases

- **Stale sessions on daemon restart:** `HashMap` starts empty. No stale cleanup needed.
- **Duplicate `system_init` for same session:** `set_session_entries` replaces previous layer. No accumulation.
- **Unknown session_id in query:** Returns base entries only (no session layer found).
- **Memory:** ~10-50 entries per session. 10 concurrent sessions = ~500 entries. Negligible.

## Testing

### Unit Tests (CommandRegistry)

- `get_for_session_returns_base_plus_session` — base builtins + session MCP, query returns both
- `get_for_session_unknown_session_returns_base_only` — no layer → just base
- `sessions_are_isolated` — A and B have different MCP, querying each returns only their own + base
- `remove_session_cleans_up` — removal drops entries, other sessions unaffected
- `set_session_entries_replaces_previous` — calling twice overwrites, doesn't accumulate
- `search_for_session_searches_both_layers` — fuzzy search hits base and session entries

### Integration Test (pipeline)

- Two mock subprocesses with different `system_init` MCP tools
- Verify `get_for_session(A)` vs `get_for_session(B)` return different tools
- Kill subprocess A, verify A's layer removed, B unaffected

### CLI Test

- `GetCommandRegistryRequest` includes `session_id` from app state
