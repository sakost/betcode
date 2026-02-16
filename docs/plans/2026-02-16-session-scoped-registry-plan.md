# Session-Scoped Command Registry Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `CommandRegistry` session-aware so multiple concurrent sessions can have different MCP tools, plugins, and skills without overwriting each other.

**Architecture:** Layered registry — one shared base layer (builtins, service commands, Claude Code commands) plus per-session layers (MCP, plugins, skills). `GetCommandRegistry` merges base + session at read time. Session layers are populated from `system_init` and cleaned up on subprocess exit.

**Tech Stack:** Rust, tonic/prost (proto changes), tokio `RwLock`, `HashMap`

---

### Task 1: Proto changes — add `session_id` to request messages

**Files:**
- Modify: `proto/betcode/v1/commands.proto:155,191-194`

**Step 1: Update `GetCommandRegistryRequest`**

```protobuf
message GetCommandRegistryRequest {
  string session_id = 1;
}
```

**Step 2: Update `ExecuteServiceCommandRequest`**

```protobuf
message ExecuteServiceCommandRequest {
  string command = 1;
  repeated string args = 2;
  string session_id = 3;
}
```

**Step 3: Regenerate proto**

Run: `cargo build -p betcode-proto`
Expected: PASS — generated code updated

**Step 4: Fix all compilation errors from changed proto types**

The following call sites construct these request types and need updating:

- `crates/betcode-cli/src/connection.rs:1113` — `GetCommandRegistryRequest {}` → add `session_id`
- `crates/betcode-cli/src/connection.rs:1188` — `ExecuteServiceCommandRequest { command, args }` → add `session_id`
- `crates/betcode-cli/src/tui/mod.rs:66` — `GetCommandRegistryRequest {}` → add `session_id`
- `crates/betcode-cli/src/tui/mod.rs:389` — `ExecuteServiceCommandRequest { command, args }` → add `session_id`
- `crates/betcode-daemon/src/server/command_svc.rs:498` — test `GetCommandRegistryRequest {}`
- `crates/betcode-daemon/src/server/command_svc.rs:486-488` — test `ExecuteServiceCommandRequest`
- `crates/betcode-daemon/src/tunnel/handler_tests.rs:1786,1853,1876` — `GetCommandRegistryRequest {}`
- `crates/betcode-daemon/src/tunnel/handler_tests.rs:2107-2109,2182-2184` — `ExecuteServiceCommandRequest`
- `crates/betcode-relay/src/server/command_proxy_tests.rs` — test stubs

For CLI callers, pass the current `session_id` from `App.session_id`. The `spawn_registry_fetch` function and `ServiceCommandExec` handler need the session ID threaded through. For test call sites, use `"test-session".to_string()`.

Run: `cargo build --workspace`
Expected: PASS

**Step 5: Commit**

```bash
git add -A && git commit -m "feat(proto): add session_id to GetCommandRegistry and ExecuteServiceCommand requests"
```

---

### Task 2: Refactor `CommandRegistry` to layered model

**Files:**
- Modify: `crates/betcode-daemon/src/commands/mod.rs`

**Step 1: Write failing tests for the new API**

Add to `mod tests`:

```rust
#[test]
fn get_for_session_returns_base_plus_session() {
    let mut registry = CommandRegistry::new();
    registry.set_session_entries("s1", vec![
        CommandEntry::new("mcp-tool", "An MCP tool", CommandCategory::Mcp, ExecutionMode::Passthrough, "mcp"),
    ]);
    let all = registry.get_for_session("s1");
    assert!(all.iter().any(|e| e.name == "cd"), "base builtins present");
    assert!(all.iter().any(|e| e.name == "mcp-tool"), "session MCP present");
}

#[test]
fn get_for_session_unknown_session_returns_base_only() {
    let registry = CommandRegistry::new();
    let all = registry.get_for_session("nonexistent");
    assert!(all.iter().any(|e| e.name == "cd"), "base builtins present");
    assert!(!all.iter().any(|e| e.category == CommandCategory::Mcp), "no MCP from unknown session");
}

#[test]
fn sessions_are_isolated() {
    let mut registry = CommandRegistry::new();
    registry.set_session_entries("s1", vec![
        CommandEntry::new("tool-a", "Tool A", CommandCategory::Mcp, ExecutionMode::Passthrough, "mcp"),
    ]);
    registry.set_session_entries("s2", vec![
        CommandEntry::new("tool-b", "Tool B", CommandCategory::Mcp, ExecutionMode::Passthrough, "mcp"),
    ]);
    let s1 = registry.get_for_session("s1");
    let s2 = registry.get_for_session("s2");
    assert!(s1.iter().any(|e| e.name == "tool-a"));
    assert!(!s1.iter().any(|e| e.name == "tool-b"));
    assert!(s2.iter().any(|e| e.name == "tool-b"));
    assert!(!s2.iter().any(|e| e.name == "tool-a"));
}

#[test]
fn remove_session_cleans_up() {
    let mut registry = CommandRegistry::new();
    registry.set_session_entries("s1", vec![
        CommandEntry::new("tool-x", "X", CommandCategory::Mcp, ExecutionMode::Passthrough, "mcp"),
    ]);
    registry.remove_session("s1");
    let all = registry.get_for_session("s1");
    assert!(!all.iter().any(|e| e.name == "tool-x"));
}

#[test]
fn set_session_entries_replaces_previous() {
    let mut registry = CommandRegistry::new();
    registry.set_session_entries("s1", vec![
        CommandEntry::new("old-tool", "Old", CommandCategory::Mcp, ExecutionMode::Passthrough, "mcp"),
    ]);
    registry.set_session_entries("s1", vec![
        CommandEntry::new("new-tool", "New", CommandCategory::Mcp, ExecutionMode::Passthrough, "mcp"),
    ]);
    let all = registry.get_for_session("s1");
    assert!(!all.iter().any(|e| e.name == "old-tool"));
    assert!(all.iter().any(|e| e.name == "new-tool"));
}

#[test]
fn search_for_session_searches_both_layers() {
    let mut registry = CommandRegistry::new();
    registry.set_session_entries("s1", vec![
        CommandEntry::new("mcp-search-target", "Searchable", CommandCategory::Mcp, ExecutionMode::Passthrough, "mcp"),
    ]);
    let results = registry.search_for_session("s1", "search-target", 10);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "mcp-search-target");
}
```

Run: `cargo test -p betcode-daemon -- test_registry`
Expected: FAIL — methods don't exist yet

**Step 2: Implement the layered registry**

Replace the `CommandRegistry` struct and impl in `crates/betcode-daemon/src/commands/mod.rs`:

```rust
use std::collections::HashMap;
use betcode_core::commands::{CommandCategory, CommandEntry, builtin_commands};

/// Registry holding all available commands with a layered model.
///
/// The base layer contains daemon-wide commands (builtins, Claude Code capabilities).
/// Per-session layers contain MCP tools, plugins, and skills scoped to each session.
pub struct CommandRegistry {
    /// Daemon-wide entries shared across all sessions.
    base_entries: Vec<CommandEntry>,
    /// Per-session entries keyed by session ID.
    session_layers: HashMap<String, Vec<CommandEntry>>,
}

impl CommandRegistry {
    /// Create a new registry initialized with built-in commands.
    pub fn new() -> Self {
        Self {
            base_entries: builtin_commands(),
            session_layers: HashMap::new(),
        }
    }

    /// Add a command entry to the base layer.
    pub fn add(&mut self, entry: CommandEntry) {
        self.base_entries.push(entry);
    }

    /// Return base entries merged with the given session's entries.
    pub fn get_for_session(&self, session_id: &str) -> Vec<CommandEntry> {
        let mut entries = self.base_entries.clone();
        if let Some(session_entries) = self.session_layers.get(session_id) {
            entries.extend(session_entries.iter().cloned());
        }
        entries
    }

    /// Return only base entries (no session context).
    pub fn get_all(&self) -> Vec<CommandEntry> {
        self.base_entries.clone()
    }

    /// Search base + session entries whose name contains the query (case-insensitive).
    pub fn search_for_session(&self, session_id: &str, query: &str, max_results: usize) -> Vec<CommandEntry> {
        let query_lower = query.to_lowercase();
        self.get_for_session(session_id)
            .into_iter()
            .filter(|e| e.name.to_lowercase().contains(&query_lower))
            .take(max_results)
            .collect()
    }

    /// Search base entries only.
    pub fn search(&self, query: &str, max_results: usize) -> Vec<CommandEntry> {
        let query_lower = query.to_lowercase();
        self.base_entries
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&query_lower))
            .take(max_results)
            .cloned()
            .collect()
    }

    /// Remove all base entries with a matching source.
    pub fn clear_source(&mut self, source: &str) {
        self.base_entries.retain(|e| e.source != source);
    }

    /// Set (replace) the entire session layer for the given session.
    pub fn set_session_entries(&mut self, session_id: &str, entries: Vec<CommandEntry>) {
        self.session_layers.insert(session_id.to_string(), entries);
    }

    /// Remove a session layer entirely (cleanup on session end).
    pub fn remove_session(&mut self, session_id: &str) {
        self.session_layers.remove(session_id);
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}
```

Remove the old `clear_plugin_sources()` method — it's no longer needed since session layers replace the entire set atomically.

Run: `cargo test -p betcode-daemon -- test_registry`
Expected: PASS — all new and existing tests pass

**Step 3: Fix callers of removed `clear_plugin_sources()`**

The only caller is `crates/betcode-daemon/src/commands/service_executor.rs:77` in `execute_reload_remote`. This will be updated in Task 4.

Run: `cargo build --workspace`
Expected: May have compile errors from `clear_plugin_sources` removal — fix in Task 4.

**Step 4: Commit**

```bash
git add -A && git commit -m "refactor(daemon): convert CommandRegistry to layered base+session model"
```

---

### Task 3: Update pipeline to use session layers

**Files:**
- Modify: `crates/betcode-daemon/src/relay/pipeline.rs:386-402`

**Step 1: Change the MCP merge block in `spawn_stdout_pipeline`**

Replace the current MCP merge (lines 386-402):

```rust
// After processing a SystemInit, merge MCP tool entries into the
// shared command registry so they appear in completions and search.
if is_system_init && !bridge.mcp_entries().is_empty() {
    let count = bridge.mcp_entries().len();
    {
        let mut registry = command_registry.write().await;
        registry.clear_source("mcp");
        for entry in bridge.mcp_entries() {
            registry.add(entry.clone());
        }
    }
    ...
}
```

With session-scoped set:

```rust
if is_system_init {
    // Build the full session layer: MCP entries + plugin/skill entries
    // discovered from the session's working directory.
    let mut session_entries: Vec<betcode_core::commands::CommandEntry> =
        bridge.mcp_entries().to_vec();

    // Discover plugins/skills from the session's cwd
    if let Some(info) = bridge.session_info() {
        let claude_dir = std::path::Path::new(&info.cwd).join(".claude");
        let plugin_entries =
            betcode_core::commands::discover_plugin_entries(&claude_dir);
        session_entries.extend(plugin_entries);
    }

    if !session_entries.is_empty() {
        let count = session_entries.len();
        {
            let mut registry = command_registry.write().await;
            registry.set_session_entries(&sid, session_entries);
        }
        debug!(
            session_id = %sid,
            count,
            "Set session layer in command registry"
        );
    }
}
```

Note: `info.cwd` is a `String` field on the proto `SessionInfo` message — check the actual field name. The `SystemInit` from NDJSON has `cwd: PathBuf` which gets converted to `SessionInfo` in `EventBridge`. Look at the `SessionInfo` proto to confirm the field name for the working directory.

**Step 2: Add session cleanup on pipeline exit**

After the stdout loop ends (around line 559 where `sessions.write().await.remove(&sid)` happens), add:

```rust
// Clean up the session layer from the command registry
{
    let mut registry = command_registry.write().await;
    registry.remove_session(&sid);
}
debug!(session_id = %sid, "Removed session layer from command registry");
```

Run: `cargo build --workspace`
Expected: PASS

**Step 3: Commit**

```bash
git add -A && git commit -m "feat(daemon): populate session layers from system_init and clean up on exit"
```

---

### Task 4: Update `execute_reload_remote` for session-scoped plugins

**Files:**
- Modify: `crates/betcode-daemon/src/commands/service_executor.rs:59-99`
- Modify: `crates/betcode-daemon/src/server/command_svc.rs:188-232`

**Step 1: Change `execute_reload_remote` signature to accept session ID**

The method currently takes `&mut CommandRegistry`. Change to also accept `session_id`:

```rust
pub fn execute_reload_remote(
    &self,
    registry: &mut CommandRegistry,
    session_id: &str,
) -> Result<String> {
    // Clear existing CC commands from base
    registry.clear_source("claude-code");
    registry.clear_source("user");

    // Re-discover commands
    let result = discover_all_cc_commands(&self.cwd, None);
    let count = result.commands.len();
    for cmd in result.commands {
        registry.add(cmd);
    }

    // Re-discover plugins/skills for this session's working directory
    let claude_dir = self.cwd.join(".claude");
    let plugin_entries = betcode_core::commands::discover_plugin_entries(&claude_dir);
    let plugin_count = plugin_entries.len();
    registry.set_session_entries(session_id, plugin_entries);

    let mut msg = format!("Reloaded {count} commands, {plugin_count} plugin entries");
    if !result.warnings.is_empty() {
        use std::fmt::Write;
        let _ = write!(msg, " ({} warnings)", result.warnings.len());
    }
    Ok(msg)
}
```

**Step 2: Update the `reload-remote` handler in `command_svc.rs`**

At line 188-200, use `req.session_id` (now available from the proto change in Task 1):

```rust
"reload-remote" => {
    // ... existing code ...
    let session_id = req.session_id; // from Task 1 proto change
    let exec = executor.read().await;
    let cwd = exec.cwd().to_path_buf();
    let mut reg = registry.write().await;
    let cmd_msg = match exec.execute_reload_remote(&mut reg, &session_id) {
        // ... rest unchanged ...
    };
    // ... rest unchanged ...
}
```

Wait — `req` is consumed earlier by destructuring `command` and `args`. Need to also capture `session_id`. Update the destructuring around line 130-131:

```rust
let command = req.command;
let args = req.args;
let session_id = req.session_id;
```

Then pass `&session_id` to `execute_reload_remote`.

**Step 3: Update the `reload-remote` test**

Update `test_execute_reload_remote` in `service_executor.rs:206-225` to pass a session ID:

```rust
let msg = executor.execute_reload_remote(&mut registry, "test-session").unwrap();
```

**Step 4: Run tests**

Run: `cargo test --workspace`
Expected: PASS

**Step 5: Commit**

```bash
git add -A && git commit -m "feat(daemon): make reload-remote session-aware for plugin discovery"
```

---

### Task 5: Update `GrpcServer::new` — move plugin discovery out of base

**Files:**
- Modify: `crates/betcode-daemon/src/server/mod.rs:100-120`

**Step 1: Remove plugin discovery from daemon startup**

At lines 109-120, the daemon discovers plugins from `~/.claude/` and adds them to the base registry. Since plugins are now session-scoped (discovered from `system_init`'s `cwd`), remove this block:

```rust
// DELETE these lines (109-120):
// Discover plugin commands from ~/.claude/
let claude_dir = dirs::home_dir().map_or_else(
    || {
        warn!("Could not determine home directory; plugin discovery will be skipped");
        std::path::PathBuf::new()
    },
    |h| h.join(".claude"),
);
let plugin_entries = betcode_core::commands::discover_plugin_entries(&claude_dir);
for entry in plugin_entries {
    registry.add(entry);
}
```

The base registry now only contains builtins + Claude Code capability commands. Plugins/MCP come from session layers.

Run: `cargo build --workspace`
Expected: PASS

**Step 2: Commit**

```bash
git add -A && git commit -m "refactor(daemon): remove global plugin discovery from daemon startup"
```

---

### Task 6: Update gRPC handler to use `session_id`

**Files:**
- Modify: `crates/betcode-daemon/src/server/command_svc.rs:73-84`

**Step 1: Update `get_command_registry` handler**

Change from `get_all()` to `get_for_session()`:

```rust
async fn get_command_registry(
    &self,
    request: Request<GetCommandRegistryRequest>,
) -> Result<Response<GetCommandRegistryResponse>, Status> {
    let session_id = &request.get_ref().session_id;
    let registry = self.registry.read().await;
    let commands = registry
        .get_for_session(session_id)
        .into_iter()
        .map(core_entry_to_proto)
        .collect();
    Ok(Response::new(GetCommandRegistryResponse { commands }))
}
```

**Step 2: Update the `test_get_command_registry` test**

```rust
#[tokio::test]
async fn test_get_command_registry() {
    let service = create_test_service().await;
    let request = tonic::Request::new(GetCommandRegistryRequest {
        session_id: "test-session".to_string(),
    });
    let response = service.get_command_registry(request).await.unwrap();
    let entries = response.into_inner().commands;
    assert!(entries.iter().any(|e| e.name == "cd"));
    assert!(entries.iter().any(|e| e.name == "pwd"));
}
```

**Step 3: Run tests**

Run: `cargo test --workspace`
Expected: PASS

**Step 4: Commit**

```bash
git add -A && git commit -m "feat(daemon): use session_id in get_command_registry gRPC handler"
```

---

### Task 7: Thread `session_id` through CLI callers

**Files:**
- Modify: `crates/betcode-cli/src/tui/mod.rs:54-99` — `spawn_registry_fetch`
- Modify: `crates/betcode-cli/src/tui/mod.rs:227-237` — initial fetch call
- Modify: `crates/betcode-cli/src/tui/mod.rs:458-468` — SessionInfo re-fetch
- Modify: `crates/betcode-cli/src/tui/mod.rs:386-392` — service command exec
- Modify: `crates/betcode-cli/src/connection.rs:1103-1121` — `get_command_registry`
- Modify: `crates/betcode-cli/src/connection.rs:1175-1192` — `execute_service_command`

**Step 1: Add `session_id` parameter to `spawn_registry_fetch`**

```rust
fn spawn_registry_fetch(
    cmd_client: Option<CommandServiceClient<Channel>>,
    auth_token: Option<String>,
    machine_id: Option<String>,
    session_id: Option<String>,
    tx: tokio::sync::mpsc::Sender<Vec<CachedCommand>>,
) {
    let Some(mut client) = cmd_client else {
        return;
    };
    tokio::spawn(async move {
        let mut request = tonic::Request::new(betcode_proto::v1::GetCommandRegistryRequest {
            session_id: session_id.unwrap_or_default(),
        });
        // ... rest unchanged
    });
}
```

**Step 2: Thread `session_id` at call sites**

At the initial fetch (line ~232), pass `app.session_id.clone()`:
```rust
spawn_registry_fetch(
    registry_cmd_client.clone(),
    registry_auth_token.clone(),
    registry_machine_id.clone(),
    app.session_id.clone(),
    cmd_registry_tx.clone(),
);
```

At the SessionInfo re-fetch (line ~463), similarly pass the current session_id. At this point `app` has been updated with the new session ID from the event.

For `execute_service_command` calls in `connection.rs`, add the session_id parameter to the method signature or pass it in `ServiceCommandExec`:

Add `session_id: String` to `ServiceCommandExec`:
```rust
pub struct ServiceCommandExec {
    pub command: String,
    pub args: Vec<String>,
    pub session_id: String,
}
```

Then use it in the service command handler (tui/mod.rs:389):
```rust
tonic::Request::new(betcode_proto::v1::ExecuteServiceCommandRequest {
    command: exec.command.clone(),
    args: exec.args,
    session_id: exec.session_id,
})
```

And populate it from `app.session_id` when creating `ServiceCommandExec` in `input.rs`.

**Step 3: Run tests**

Run: `cargo test --workspace`
Expected: PASS

**Step 4: Commit**

```bash
git add -A && git commit -m "feat(cli): pass session_id in registry and service command requests"
```

---

### Task 8: Update relay proxy tests

**Files:**
- Modify: `crates/betcode-relay/src/server/command_proxy_tests.rs`
- Modify: `crates/betcode-daemon/src/tunnel/handler_tests.rs`

**Step 1: Update all test call sites with `session_id` field**

In command proxy tests, update `GetCommandRegistryRequest` and `ExecuteServiceCommandRequest` constructors to include `session_id: "test-session".to_string()`.

In tunnel handler tests, same: add `session_id` to all request struct literals.

**Step 2: Run tests**

Run: `cargo test --workspace`
Expected: PASS

**Step 3: Commit**

```bash
git add -A && git commit -m "test: update relay and tunnel tests for session_id proto fields"
```

---

### Task 9: Remove old `clear_plugin_sources` test and dead code

**Files:**
- Modify: `crates/betcode-daemon/src/commands/mod.rs` — remove `test_clear_plugin_sources_removes_skill_and_plugin_entries`
- Modify: `crates/betcode-daemon/src/commands/mod.rs` — remove old `test_registry_clear_and_reload` (base now uses `clear_source` which still works, but review if the test still makes sense)

**Step 1: Clean up**

Remove the `test_clear_plugin_sources_removes_skill_and_plugin_entries` test (method no longer exists). Update `test_registry_clear_and_reload` to use the new API if needed.

**Step 2: Run full quality check**

Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo fmt --all -- --check`
Expected: PASS on all three

**Step 3: Commit**

```bash
git add -A && git commit -m "refactor(daemon): remove dead clear_plugin_sources code and update tests"
```
