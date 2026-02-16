# Command Discovery & Dual-Dispatch Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enrich the command registry with dynamically-discovered skills, plugin commands, and MCP tools, and add client-side effects for dual-dispatch commands (`/compact`, `/clear`, `/model`, `/fast`).

**Architecture:** Extend the proto `CommandEntry` with `group` and `display_name` fields and two new categories (`SKILL`, `MCP`). Daemon discovers skills/plugin commands from the filesystem and MCP tools from `system_init`. CLI gains a dual-dispatch mechanism that forwards commands to Claude AND applies local TUI effects on `TurnComplete`.

**Tech Stack:** Rust, protobuf (tonic/prost), serde_json for parsing plugin manifests, existing NDJSON parser

---

## Task 1: Extend Proto CommandEntry

**Files:**
- Modify: `proto/betcode/v1/commands.proto:41-64`

**Step 1: Add new enum values and fields**

In `commands.proto`, add two new `CommandCategory` values and two new fields to `CommandEntry`:

```protobuf
enum CommandCategory {
  COMMAND_CATEGORY_UNSPECIFIED = 0;
  COMMAND_CATEGORY_SERVICE = 1;
  COMMAND_CATEGORY_CLAUDE_CODE = 2;
  COMMAND_CATEGORY_PLUGIN = 3;
  COMMAND_CATEGORY_SKILL = 4;
  COMMAND_CATEGORY_MCP = 5;
}

message CommandEntry {
  string name = 1;
  string description = 2;
  CommandCategory category = 3;
  ExecutionMode execution_mode = 4;
  string source = 5;
  optional string args_schema = 6;
  string group = 7;
  string display_name = 8;
}
```

**Step 2: Rebuild proto crate**

Run: `cargo build -p betcode-proto`
Expected: SUCCESS — new fields and enum values generated

**Step 3: Commit**

```bash
git add proto/betcode/v1/commands.proto
git commit -m "feat(proto): add Skill/MCP categories and group/display_name to CommandEntry"
```

---

## Task 2: Extend Core CommandEntry and Categories

**Files:**
- Modify: `crates/betcode-core/src/commands/mod.rs:11-60`

**Step 1: Write tests for new categories**

Add tests to `crates/betcode-core/src/commands/mod.rs` in the existing `tests` module:

```rust
#[test]
fn test_skill_command_entry() {
    let entry = CommandEntry::new(
        "superpowers:brainstorming",
        "Brainstorming skill",
        CommandCategory::Skill,
        ExecutionMode::Passthrough,
        "superpowers@superpowers-dev",
    )
    .with_group("superpowers")
    .with_display_name("superpowers:brainstorming");

    assert_eq!(entry.category, CommandCategory::Skill);
    assert_eq!(entry.group.as_deref(), Some("superpowers"));
    assert_eq!(
        entry.display_name.as_deref(),
        Some("superpowers:brainstorming")
    );
}

#[test]
fn test_mcp_command_entry() {
    let entry = CommandEntry::new(
        "chrome-devtools:take_screenshot",
        "Take a screenshot",
        CommandCategory::Mcp,
        ExecutionMode::Passthrough,
        "mcp",
    )
    .with_group("chrome-devtools")
    .with_display_name("chrome-devtools:take_screenshot");

    assert_eq!(entry.category, CommandCategory::Mcp);
    assert_eq!(entry.group.as_deref(), Some("chrome-devtools"));
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p betcode-core -- test_skill_command_entry test_mcp_command_entry`
Expected: FAIL — `CommandCategory::Skill`, `CommandCategory::Mcp`, `with_group`, `with_display_name` don't exist

**Step 3: Add new variants and fields**

In `crates/betcode-core/src/commands/mod.rs`, update:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandCategory {
    Service,
    ClaudeCode,
    Plugin,
    Skill,
    Mcp,
}

#[derive(Debug, Clone)]
pub struct CommandEntry {
    pub name: String,
    pub description: String,
    pub category: CommandCategory,
    pub execution_mode: ExecutionMode,
    pub source: String,
    pub args_schema: Option<String>,
    pub group: Option<String>,
    pub display_name: Option<String>,
}

impl CommandEntry {
    pub fn new(
        name: &str,
        description: &str,
        category: CommandCategory,
        execution_mode: ExecutionMode,
        source: &str,
    ) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            category,
            execution_mode,
            source: source.to_string(),
            args_schema: None,
            group: None,
            display_name: None,
        }
    }

    pub fn with_group(mut self, group: &str) -> Self {
        self.group = Some(group.to_string());
        self
    }

    pub fn with_display_name(mut self, display_name: &str) -> Self {
        self.display_name = Some(display_name.to_string());
        self
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p betcode-core -- test_skill_command_entry test_mcp_command_entry`
Expected: PASS

**Step 5: Fix compilation in dependent crates**

Run: `cargo build --workspace`
Expected: May have compile errors in `betcode-daemon` where `core_entry_to_proto` and `core_category_to_proto` need updating.

Update `crates/betcode-daemon/src/server/command_svc.rs:294-319`:

```rust
fn core_entry_to_proto(
    entry: betcode_core::commands::CommandEntry,
) -> betcode_proto::v1::CommandEntry {
    betcode_proto::v1::CommandEntry {
        name: entry.name,
        description: entry.description,
        category: core_category_to_proto(&entry.category) as i32,
        execution_mode: core_exec_mode_to_proto(&entry.execution_mode) as i32,
        source: entry.source,
        args_schema: entry.args_schema,
        group: entry.group.unwrap_or_default(),
        display_name: entry.display_name.unwrap_or_default(),
    }
}

const fn core_category_to_proto(
    cat: &betcode_core::commands::CommandCategory,
) -> betcode_proto::v1::CommandCategory {
    match cat {
        betcode_core::commands::CommandCategory::Service => {
            betcode_proto::v1::CommandCategory::Service
        }
        betcode_core::commands::CommandCategory::ClaudeCode => {
            betcode_proto::v1::CommandCategory::ClaudeCode
        }
        betcode_core::commands::CommandCategory::Plugin => {
            betcode_proto::v1::CommandCategory::Plugin
        }
        betcode_core::commands::CommandCategory::Skill => {
            betcode_proto::v1::CommandCategory::Skill
        }
        betcode_core::commands::CommandCategory::Mcp => {
            betcode_proto::v1::CommandCategory::Mcp
        }
    }
}
```

**Step 6: Verify full workspace builds and tests pass**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS

**Step 7: Commit**

```bash
git add crates/betcode-core/src/commands/mod.rs crates/betcode-daemon/src/server/command_svc.rs
git commit -m "feat(core): add Skill/Mcp categories and group/display_name fields to CommandEntry"
```

---

## Task 3: Discover Skills and Plugin Commands from Filesystem

**Files:**
- Create: `crates/betcode-core/src/commands/plugins.rs`
- Modify: `crates/betcode-core/src/commands/mod.rs:1-8` (add module + re-export)

**Step 1: Write tests for plugin discovery**

Create `crates/betcode-core/src/commands/plugins.rs` with tests first:

```rust
//! Discovery of skills and commands from installed Claude Code plugins.

use std::path::Path;

use super::{CommandCategory, CommandEntry, ExecutionMode};

/// Discovers skills and commands from all enabled plugins.
///
/// Reads `~/.claude/plugins/installed_plugins.json` for install paths and
/// `~/.claude/settings.json` for the enabled/disabled filter.
pub fn discover_plugin_entries(claude_dir: &Path) -> Vec<CommandEntry> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_plugin_fixture(dir: &Path) {
        // settings.json with one enabled plugin
        let settings = serde_json::json!({
            "enabledPlugins": {
                "superpowers@superpowers-dev": true,
                "disabled-plugin@some-marketplace": false
            }
        });
        fs::write(dir.join("settings.json"), settings.to_string()).unwrap();

        // installed_plugins.json
        let superpowers_path = dir.join("plugins/cache/superpowers-dev/superpowers/1.0.0");
        let disabled_path = dir.join("plugins/cache/some-marketplace/disabled-plugin/1.0.0");
        fs::create_dir_all(&superpowers_path).unwrap();
        fs::create_dir_all(&disabled_path).unwrap();

        let installed = serde_json::json!({
            "version": 2,
            "plugins": {
                "superpowers@superpowers-dev": [{
                    "scope": "user",
                    "installPath": superpowers_path.to_string_lossy(),
                    "version": "1.0.0"
                }],
                "disabled-plugin@some-marketplace": [{
                    "scope": "user",
                    "installPath": disabled_path.to_string_lossy(),
                    "version": "1.0.0"
                }]
            }
        });
        fs::create_dir_all(dir.join("plugins")).unwrap();
        fs::write(
            dir.join("plugins/installed_plugins.json"),
            installed.to_string(),
        )
        .unwrap();

        // superpowers skills
        let skills_dir = superpowers_path.join("skills");
        fs::create_dir_all(skills_dir.join("brainstorming")).unwrap();
        fs::write(skills_dir.join("brainstorming/SKILL.md"), "# Brainstorm").unwrap();
        fs::create_dir_all(skills_dir.join("writing-plans")).unwrap();
        fs::write(skills_dir.join("writing-plans/SKILL.md"), "# Plans").unwrap();

        // superpowers commands
        let cmds_dir = superpowers_path.join("commands");
        fs::create_dir_all(&cmds_dir).unwrap();
        fs::write(cmds_dir.join("brainstorm.md"), "# Brainstorm command").unwrap();

        // disabled plugin also has skills (should be ignored)
        let disabled_skills = disabled_path.join("skills/some-skill");
        fs::create_dir_all(&disabled_skills).unwrap();
        fs::write(disabled_skills.join("SKILL.md"), "# Disabled").unwrap();
    }

    #[test]
    fn test_discovers_skills_from_enabled_plugins() {
        let dir = TempDir::new().unwrap();
        setup_plugin_fixture(dir.path());

        let entries = discover_plugin_entries(dir.path());
        let skill_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.category == CommandCategory::Skill)
            .collect();

        assert_eq!(skill_entries.len(), 2);
        assert!(skill_entries.iter().any(|e| e.name == "superpowers:brainstorming"));
        assert!(skill_entries.iter().any(|e| e.name == "superpowers:writing-plans"));
        assert!(skill_entries
            .iter()
            .all(|e| e.group.as_deref() == Some("superpowers")));
        assert!(skill_entries
            .iter()
            .all(|e| e.source == "superpowers@superpowers-dev"));
    }

    #[test]
    fn test_discovers_commands_from_enabled_plugins() {
        let dir = TempDir::new().unwrap();
        setup_plugin_fixture(dir.path());

        let entries = discover_plugin_entries(dir.path());
        let cmd_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.category == CommandCategory::ClaudeCode)
            .collect();

        assert_eq!(cmd_entries.len(), 1);
        assert_eq!(cmd_entries[0].name, "superpowers:brainstorm");
    }

    #[test]
    fn test_ignores_disabled_plugins() {
        let dir = TempDir::new().unwrap();
        setup_plugin_fixture(dir.path());

        let entries = discover_plugin_entries(dir.path());
        assert!(!entries.iter().any(|e| e.name.contains("disabled")));
        assert!(!entries.iter().any(|e| e.name.contains("some-skill")));
    }

    #[test]
    fn test_handles_missing_files_gracefully() {
        let dir = TempDir::new().unwrap();
        let entries = discover_plugin_entries(dir.path());
        assert!(entries.is_empty());
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p betcode-core -- test_discovers_skills test_discovers_commands test_ignores_disabled test_handles_missing`
Expected: FAIL — `discover_plugin_entries` has `todo!()`

**Step 3: Implement `discover_plugin_entries`**

Replace the `todo!()` in `discover_plugin_entries` with:

```rust
pub fn discover_plugin_entries(claude_dir: &Path) -> Vec<CommandEntry> {
    let enabled = read_enabled_plugins(claude_dir);
    if enabled.is_empty() {
        return Vec::new();
    }

    let installed = read_installed_plugins(claude_dir);

    let mut entries = Vec::new();
    for (plugin_id, install_path) in &installed {
        if !enabled.contains(plugin_id) {
            continue;
        }
        let plugin_name = plugin_name_from_id(plugin_id);

        // Discover skills from skills/ subdirectories
        let skills_dir = install_path.join("skills");
        if skills_dir.is_dir() {
            if let Ok(dirs) = std::fs::read_dir(&skills_dir) {
                for dir_entry in dirs.flatten() {
                    let skill_path = dir_entry.path();
                    if skill_path.is_dir() && skill_path.join("SKILL.md").exists() {
                        if let Some(skill_name) = skill_path.file_name().and_then(|n| n.to_str()) {
                            let full_name = format!("{plugin_name}:{skill_name}");
                            entries.push(
                                CommandEntry::new(
                                    &full_name,
                                    &format!("Skill: {skill_name}"),
                                    CommandCategory::Skill,
                                    ExecutionMode::Passthrough,
                                    plugin_id,
                                )
                                .with_group(&plugin_name)
                                .with_display_name(&full_name),
                            );
                        }
                    }
                }
            }
        }

        // Discover commands from commands/*.md
        let cmds_dir = install_path.join("commands");
        if cmds_dir.is_dir() {
            if let Ok(files) = std::fs::read_dir(&cmds_dir) {
                for file_entry in files.flatten() {
                    let file_path = file_entry.path();
                    if file_path.extension().and_then(|e| e.to_str()) == Some("md") {
                        if let Some(cmd_name) = file_path.file_stem().and_then(|n| n.to_str()) {
                            let full_name = format!("{plugin_name}:{cmd_name}");
                            entries.push(
                                CommandEntry::new(
                                    &full_name,
                                    &format!("Plugin command: {cmd_name}"),
                                    CommandCategory::ClaudeCode,
                                    ExecutionMode::Passthrough,
                                    plugin_id,
                                )
                                .with_group(&plugin_name)
                                .with_display_name(&full_name),
                            );
                        }
                    }
                }
            }
        }
    }

    entries
}

/// Extract plugin name (before @) from a plugin ID like "superpowers@superpowers-dev".
fn plugin_name_from_id(plugin_id: &str) -> String {
    plugin_id
        .split('@')
        .next()
        .unwrap_or(plugin_id)
        .to_string()
}

/// Read enabled plugin IDs from settings.json.
fn read_enabled_plugins(claude_dir: &Path) -> Vec<String> {
    let settings_path = claude_dir.join("settings.json");
    let Ok(content) = std::fs::read_to_string(&settings_path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Vec::new();
    };

    let Some(enabled_map) = json.get("enabledPlugins").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    enabled_map
        .iter()
        .filter(|(_, v)| v.as_bool() == Some(true))
        .map(|(k, _)| k.clone())
        .collect()
}

/// Read installed plugin paths from installed_plugins.json.
fn read_installed_plugins(claude_dir: &Path) -> Vec<(String, std::path::PathBuf)> {
    let installed_path = claude_dir.join("plugins/installed_plugins.json");
    let Ok(content) = std::fs::read_to_string(&installed_path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return Vec::new();
    };

    let Some(plugins_map) = json.get("plugins").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for (plugin_id, versions) in plugins_map {
        // Take the first (latest) version entry
        if let Some(first) = versions.as_array().and_then(|a| a.first()) {
            if let Some(path_str) = first.get("installPath").and_then(|v| v.as_str()) {
                result.push((plugin_id.clone(), std::path::PathBuf::from(path_str)));
            }
        }
    }
    result
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p betcode-core -- test_discovers_skills test_discovers_commands test_ignores_disabled test_handles_missing`
Expected: PASS

**Step 5: Wire into module**

In `crates/betcode-core/src/commands/mod.rs`, add at the top:

```rust
pub mod plugins;
```

And add to the `pub use` block:

```rust
pub use plugins::discover_plugin_entries;
```

**Step 6: Build workspace**

Run: `cargo build --workspace`
Expected: PASS

**Step 7: Commit**

```bash
git add crates/betcode-core/src/commands/plugins.rs crates/betcode-core/src/commands/mod.rs
git commit -m "feat(core): discover skills and commands from installed Claude Code plugins"
```

---

## Task 4: Extract MCP Tools from system_init

**Files:**
- Modify: `crates/betcode-core/src/ndjson/types.rs` (no changes needed — `ToolSchema` already has `name` and `description`)
- Create: `crates/betcode-core/src/commands/mcp.rs`
- Modify: `crates/betcode-core/src/commands/mod.rs` (add module + re-export)

**Step 1: Write tests for MCP tool parsing**

Create `crates/betcode-core/src/commands/mcp.rs`:

```rust
//! Extraction of MCP tool entries from Claude Code's system_init tools list.

use crate::ndjson::ToolSchema;

use super::{CommandCategory, CommandEntry, ExecutionMode};

/// Converts MCP tool schemas from system_init into command entries.
///
/// Only tools with names matching the `mcp__server__tool` convention are included.
pub fn mcp_tools_to_entries(tools: &[ToolSchema]) -> Vec<CommandEntry> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(name: &str, desc: Option<&str>) -> ToolSchema {
        ToolSchema {
            name: name.to_string(),
            description: desc.map(String::from),
            input_schema: None,
        }
    }

    #[test]
    fn test_extracts_mcp_tools() {
        let tools = vec![
            make_tool("mcp__chrome-devtools__take_screenshot", Some("Take a screenshot")),
            make_tool("mcp__chrome-devtools__click", Some("Click an element")),
            make_tool("mcp__tavily__tavily-search", Some("Search the web")),
            make_tool("Read", Some("Read a file")),       // not MCP
            make_tool("Write", Some("Write a file")),     // not MCP
        ];

        let entries = mcp_tools_to_entries(&tools);

        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| e.category == CommandCategory::Mcp));

        let chrome = entries.iter().find(|e| e.name == "chrome-devtools:take_screenshot");
        assert!(chrome.is_some());
        let chrome = chrome.unwrap();
        assert_eq!(chrome.group.as_deref(), Some("chrome-devtools"));
        assert_eq!(chrome.source, "mcp");

        let tavily = entries.iter().find(|e| e.name == "tavily:tavily-search");
        assert!(tavily.is_some());
        assert_eq!(tavily.unwrap().group.as_deref(), Some("tavily"));
    }

    #[test]
    fn test_no_mcp_tools() {
        let tools = vec![
            make_tool("Read", None),
            make_tool("Bash", None),
        ];
        let entries = mcp_tools_to_entries(&tools);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_malformed_mcp_name_skipped() {
        let tools = vec![
            make_tool("mcp__", None),                    // no server or tool
            make_tool("mcp__server", None),              // no tool name
            make_tool("mcp__server__tool", Some("OK")),  // valid
        ];
        let entries = mcp_tools_to_entries(&tools);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "server:tool");
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p betcode-core -- test_extracts_mcp test_no_mcp test_malformed_mcp`
Expected: FAIL — `todo!()`

**Step 3: Implement**

```rust
pub fn mcp_tools_to_entries(tools: &[ToolSchema]) -> Vec<CommandEntry> {
    tools
        .iter()
        .filter_map(|tool| {
            let rest = tool.name.strip_prefix("mcp__")?;
            let (server, tool_name) = rest.split_once("__")?;
            if server.is_empty() || tool_name.is_empty() {
                return None;
            }
            let display_name = format!("{server}:{tool_name}");
            let description = tool
                .description
                .clone()
                .unwrap_or_else(|| format!("MCP tool: {tool_name}"));
            Some(
                CommandEntry::new(
                    &display_name,
                    &description,
                    CommandCategory::Mcp,
                    ExecutionMode::Passthrough,
                    "mcp",
                )
                .with_group(server)
                .with_display_name(&display_name),
            )
        })
        .collect()
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p betcode-core -- test_extracts_mcp test_no_mcp test_malformed_mcp`
Expected: PASS

**Step 5: Wire into module**

Add to `crates/betcode-core/src/commands/mod.rs`:

```rust
pub mod mcp;
```

And re-export:

```rust
pub use mcp::mcp_tools_to_entries;
```

**Step 6: Build workspace**

Run: `cargo build --workspace`
Expected: PASS

**Step 7: Commit**

```bash
git add crates/betcode-core/src/commands/mcp.rs crates/betcode-core/src/commands/mod.rs
git commit -m "feat(core): extract MCP tool entries from system_init tools list"
```

---

## Task 5: Integrate MCP Discovery into EventBridge

**Files:**
- Modify: `crates/betcode-daemon/src/subprocess/bridge.rs:91-110`

The EventBridge receives `SystemInit` which contains the tools list. We need to extract MCP entries and make them available to the command registry.

**Step 1: Add MCP entries field to EventBridge**

In `crates/betcode-daemon/src/subprocess/bridge.rs`, add a field to the `EventBridge` struct:

```rust
pub struct EventBridge {
    sequence: u64,
    pending_tools: HashMap<String, String>,
    session_info: Option<SessionInfo>,
    pending_question_inputs: HashMap<String, serde_json::Value>,
    pending_permission_inputs: HashMap<String, serde_json::Value>,
    /// MCP tool entries extracted from the last system_init message.
    mcp_entries: Vec<betcode_core::commands::CommandEntry>,
}
```

Initialize as `mcp_entries: Vec::new()` in `new()` and `with_start_sequence()`.

**Step 2: Populate in handle_system_init**

In `handle_system_init`, after creating `SessionInfo`, add:

```rust
self.mcp_entries = betcode_core::commands::mcp_tools_to_entries(&init.tools);
```

**Step 3: Add accessor**

```rust
/// Returns MCP tool entries discovered from the most recent system_init.
pub fn mcp_entries(&self) -> &[betcode_core::commands::CommandEntry] {
    &self.mcp_entries
}
```

**Step 4: Build and test**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-daemon/src/subprocess/bridge.rs
git commit -m "feat(daemon): extract MCP entries from system_init in EventBridge"
```

---

## Task 6: Merge MCP Entries into Command Registry on Session Start

**Files:**
- Modify: `crates/betcode-daemon/src/subprocess/manager.rs` (or wherever the bridge output is consumed and events are forwarded)

This task depends on understanding how the daemon's subprocess manager consumes EventBridge output and has access to the command registry. The pattern will be:

1. After `EventBridge::convert(Message::SystemInit(...))` is called
2. Check `bridge.mcp_entries()` — if non-empty, merge into the shared `CommandRegistry`
3. The registry is an `Arc<RwLock<CommandRegistry>>` shared with `CommandServiceImpl`

**Step 1: Find the event processing loop**

The subprocess manager's stdout reader task calls `bridge.convert(msg)` for each NDJSON line. Locate where `SessionInfo` events are produced and add MCP registry merging there.

**Step 2: After converting a SystemInit, merge MCP entries**

```rust
let events = bridge.convert(msg);
if !bridge.mcp_entries().is_empty() {
    let mut registry = registry.write().await;
    // Clear previous MCP entries for this session
    registry.clear_source("mcp");
    for entry in bridge.mcp_entries() {
        registry.add(entry.clone());
    }
}
```

Note: The exact code will depend on whether `CommandRegistry` already has `clear_source()` and `add()` methods. If not, they need to be added.

**Step 3: Build and test**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/betcode-daemon/src/subprocess/manager.rs
git commit -m "feat(daemon): merge MCP tool entries into command registry on session start"
```

---

## Task 7: Integrate Plugin Discovery into Daemon Startup

**Files:**
- Modify: daemon startup code where the command registry is initially populated

**Step 1: Add plugin discovery to registry initialization**

During daemon startup (where `hardcoded_cc_commands`, `builtin_commands`, and `discover_user_commands` are called), also call:

```rust
let claude_dir = dirs::home_dir()
    .map(|h| h.join(".claude"))
    .unwrap_or_default();
let plugin_entries = betcode_core::commands::discover_plugin_entries(&claude_dir);
for entry in plugin_entries {
    registry.add(entry);
}
```

**Step 2: Add plugin re-discovery to reload-remote**

In the `reload-remote` service command handler, also re-run `discover_plugin_entries` and merge results.

**Step 3: Build and test**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS

**Step 4: Commit**

```bash
git add crates/betcode-daemon/src/
git commit -m "feat(daemon): discover plugin skills and commands on startup and reload"
```

---

## Task 8: Dual-Dispatch Command Infrastructure in CLI

**Files:**
- Modify: `crates/betcode-cli/src/app/state.rs:156-198` (add fields to App)
- Modify: `crates/betcode-cli/src/tui/input.rs:369-430` (add dual-dispatch routing)

**Step 1: Define ClientCommand enum and add state fields**

In `crates/betcode-cli/src/app/state.rs`, add:

```rust
/// Commands that need client-side effects after Claude processes them.
#[derive(Debug, Clone)]
pub enum ClientCommand {
    /// /compact — clear old messages, insert compaction divider.
    Compact,
    /// /model <name> — update model in status bar.
    ModelSwitch(String),
    /// /fast — toggle fast mode indicator.
    FastToggle,
}
```

Add fields to the `App` struct:

```rust
pub struct App {
    // ... existing fields ...

    /// Pending dual-dispatch command awaiting TurnComplete to apply.
    pub pending_client_command: Option<ClientCommand>,
    /// Compaction summary text shown in detail panel on the divider.
    pub compaction_summary: Option<String>,
    /// Index of the compaction divider message in `messages`, if any.
    pub compaction_divider_index: Option<usize>,
}
```

Initialize as `None` in `App::new()`.

**Step 2: Add dual-dispatch routing in input handler**

In `crates/betcode-cli/src/tui/input.rs`, modify the slash command dispatch section. Replace the catch-all `_` arm (lines 411-429) with:

```rust
_ => {
    // Check for dual-dispatch commands
    let client_cmd = match command.as_str() {
        "compact" => Some(ClientCommand::Compact),
        "model" => args.first().map(|m| ClientCommand::ModelSwitch(m.clone())),
        "fast" => Some(ClientCommand::FastToggle),
        _ => None,
    };
    app.pending_client_command = client_cmd;

    // Forward to Claude Code subprocess
    let _ = tx
        .send(AgentRequest {
            request: Some(
                betcode_proto::v1::agent_request::Request::Message(
                    betcode_proto::v1::UserMessage {
                        content: trimmed.to_string(),
                        attachments: Vec::new(),
                        agent_id: String::new(),
                    },
                ),
            ),
        })
        .await;
    app.agent_busy = true;
}
```

**Step 3: Handle TurnComplete for dual-dispatch**

In `crates/betcode-cli/src/app/state.rs`, find the `TurnComplete` event handler (around line 562) and extend it:

```rust
Some(Event::TurnComplete(_)) => {
    self.finish_streaming();
    self.agent_busy = false;
    self.execute_pending_client_command();
}
```

Add the method to `App`:

```rust
fn execute_pending_client_command(&mut self) {
    let Some(cmd) = self.pending_client_command.take() else {
        return;
    };
    match cmd {
        ClientCommand::Compact => {
            self.apply_compaction();
        }
        ClientCommand::ModelSwitch(model) => {
            self.model = model;
        }
        ClientCommand::FastToggle => {
            // Toggle fast mode indicator in status if needed
        }
    }
}
```

**Step 4: Build**

Run: `cargo build --workspace`
Expected: PASS (the `apply_compaction` method doesn't exist yet — add a stub)

**Step 5: Commit**

```bash
git add crates/betcode-cli/src/app/state.rs crates/betcode-cli/src/tui/input.rs
git commit -m "feat(cli): add dual-dispatch command infrastructure with pending client commands"
```

---

## Task 9: Implement Compaction UX

**Files:**
- Modify: `crates/betcode-cli/src/app/state.rs` (add `apply_compaction` method)
- Modify: `crates/betcode-cli/src/ui/render.rs` (render compaction divider)
- Modify: detail panel rendering (show compaction summary)

**Step 1: Add compaction divider message role**

In `crates/betcode-cli/src/app/state.rs`, add a variant to `MessageRole`:

```rust
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
    CompactionDivider,
}
```

**Step 2: Implement `apply_compaction`**

```rust
fn apply_compaction(&mut self) {
    // Capture the last assistant message as the compaction summary
    let summary = self
        .messages
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::Assistant)
        .map(|m| m.content.clone())
        .unwrap_or_else(|| "Context compacted.".to_string());

    // Remove the previous compaction divider if it exists
    if let Some(old_idx) = self.compaction_divider_index {
        if old_idx < self.messages.len() {
            self.messages.remove(old_idx);
        }
    }

    // Clear all messages before the current point
    self.messages.clear();

    // Insert the compaction divider
    self.messages.push(DisplayMessage {
        role: MessageRole::CompactionDivider,
        content: "[context compacted]".to_string(),
        streaming: false,
        is_tool_result: false,
        agent_label: None,
    });
    self.compaction_divider_index = Some(0);
    self.compaction_summary = Some(summary);
    self.scroll_to_bottom();
}
```

**Step 3: Render the divider**

In the message rendering code (`crates/betcode-cli/src/ui/render.rs`), add handling for `MessageRole::CompactionDivider`:

```rust
MessageRole::CompactionDivider => {
    // Render as a dim horizontal rule with centered text
    let divider_text = format!("┄ {} ┄", msg.content);
    let style = Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM);
    // Center the text within the available width
    let line = Line::from(Span::styled(divider_text, style));
    // ... render centered
}
```

**Step 4: Show summary in detail panel**

When the cursor/selection is on the compaction divider and the detail panel is open, show `app.compaction_summary` instead of tool call details. This goes in the detail panel rendering code.

**Step 5: Build and manual test**

Run: `cargo build --workspace`
Expected: PASS

**Step 6: Commit**

```bash
git add crates/betcode-cli/src/app/state.rs crates/betcode-cli/src/ui/render.rs
git commit -m "feat(cli): implement compaction UX with divider and summary in detail panel"
```

---

## Task 10: CLI Re-fetches Registry After Session Start

**Files:**
- Modify: `crates/betcode-cli/src/tui/mod.rs` (where event stream is processed)

**Step 1: Detect SessionInfo event and re-fetch registry**

In the TUI event loop, when a `SessionInfo` event arrives (indicating a new or resumed session), trigger a re-fetch of the command registry to pick up any new MCP entries:

```rust
Some(Event::SessionInfo(info)) => {
    app.apply_event(event);
    // Re-fetch command registry (may have new MCP entries)
    if let Some(registry_tx) = &registry_refresh_tx {
        let _ = registry_tx.try_send(());
    }
}
```

Add a background task that listens on `registry_refresh_tx` and calls `GetCommandRegistry`, updating `app.command_cache`.

**Step 2: Build and test**

Run: `cargo build --workspace && cargo test --workspace`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/betcode-cli/src/tui/mod.rs
git commit -m "feat(cli): re-fetch command registry after session init for MCP discovery"
```

---

## Summary of Tasks

| # | Task | Crate | Category |
|---|------|-------|----------|
| 1 | Extend proto CommandEntry | betcode-proto | Protocol |
| 2 | Extend core CommandEntry & categories | betcode-core + betcode-daemon | Core |
| 3 | Discover skills/commands from filesystem | betcode-core | Discovery |
| 4 | Extract MCP tools from system_init | betcode-core | Discovery |
| 5 | Integrate MCP into EventBridge | betcode-daemon | Discovery |
| 6 | Merge MCP entries into registry | betcode-daemon | Discovery |
| 7 | Plugin discovery on daemon startup | betcode-daemon | Discovery |
| 8 | Dual-dispatch infrastructure | betcode-cli | Client-side |
| 9 | Compaction UX | betcode-cli | Client-side |
| 10 | Registry re-fetch after session start | betcode-cli | Client-side |

**Dependencies:** 1 → 2 → {3, 4} → {5, 7} → 6 → 10. Task 8 → 9. Tasks 3-7 and 8-9 are independent tracks.
