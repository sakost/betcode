# Command System & Autocomplete Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a command registry (daemon), autocomplete UI (CLI), plugin system, file indexing, agent completion, and bash execution to BetCode.

**Architecture:** Three-layer system — daemon-side command registry + CLI-side cached commands + TUI autocomplete (ghost text + floating overlay). Three execution lanes (Claude Code, service commands, completions) never block each other. Plugins connect via gRPC over Unix sockets.

**Tech Stack:** Rust, tokio, tonic (gRPC), ratatui 0.30, prost, notify (fs watching), nucleo (fuzzy matching), clap 4

**Design doc:** `docs/plans/2026-02-10-command-system-design.md`

---

## Phase 1: Proto Definitions & Core Types

### Task 1: CommandService Proto

**Files:**
- Create: `proto/betcode/v1/commands.proto`
- Modify: `crates/betcode-proto/build.rs` (add new proto to compile list)

**Step 1: Write the proto file**

Create `proto/betcode/v1/commands.proto` with all messages and the `CommandService` definition. Include:
- `CommandEntry` message with name, description, `CommandCategory` enum (SERVICE/CLAUDE_CODE/PLUGIN), `ExecutionMode` enum (LOCAL/PASSTHROUGH/PLUGIN), source string, optional args_schema
- `AgentInfo` message with name, `AgentKind` enum (CLAUDE_INTERNAL/DAEMON_ORCHESTRATED/TEAM_MEMBER), `AgentStatus` enum (reuse from common.proto or define IDLE/WORKING/DONE/FAILED), optional session_id
- `PathEntry` message with path, `PathKind` enum (FILE/DIRECTORY/SYMLINK), size, modified_at
- `ServiceCommandOutput` message with oneof (stdout_line, stderr_line, exit_code, error)
- `PluginInfo` message with name, status string, enabled bool, socket_path, command_count, health details
- All Request/Response wrappers for: GetCommandRegistry, ListAgents, ListPath, ExecuteServiceCommand (server-streaming), ListPlugins, GetPluginStatus, AddPlugin, RemovePlugin, EnablePlugin, DisablePlugin
- `CommandService` service definition

Reference existing proto style from `proto/betcode/v1/agent.proto` and `proto/betcode/v1/common.proto` for conventions (package, options, import style).

**Step 2: Write the plugin proto file**

Create `proto/betcode/v1/plugin.proto` with:
- `RegisterRequest` (empty or with plugin metadata)
- `RegisterResponse` with repeated `CommandDefinition` (name, description, args_schema JSON string)
- `ExecuteRequest` with command name and args JSON string
- `ExecuteResponse` with oneof (stdout_line, stderr_line, exit_code, error)
- `HealthCheckRequest` (empty)
- `HealthCheckResponse` with status bool and optional message
- `PluginService` service definition

**Step 3: Add protos to build.rs**

Modify `crates/betcode-proto/build.rs` to include `proto/betcode/v1/commands.proto` and `proto/betcode/v1/plugin.proto` in the tonic compile list.

**Step 4: Verify proto compilation**

Run: `cargo build -p betcode-proto`
Expected: Clean compile, generated Rust types available

**Step 5: Commit**

```bash
git add proto/betcode/v1/commands.proto proto/betcode/v1/plugin.proto crates/betcode-proto/build.rs
git commit -m "feat(proto): add CommandService and PluginService proto definitions"
```

---

### Task 2: Core Command Types & Trait

**Files:**
- Create: `crates/betcode-core/src/commands/mod.rs`
- Create: `crates/betcode-core/src/commands/builtin.rs`
- Modify: `crates/betcode-core/src/lib.rs` (add `pub mod commands;`)

**Step 1: Write the failing test**

In `crates/betcode-core/src/commands/mod.rs`, add a test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_entry_creation() {
        let entry = CommandEntry::new(
            "cd",
            "Change working directory",
            CommandCategory::Service,
            ExecutionMode::Local,
            "built-in",
        );
        assert_eq!(entry.name, "cd");
        assert_eq!(entry.category, CommandCategory::Service);
        assert_eq!(entry.execution_mode, ExecutionMode::Local);
    }

    #[test]
    fn test_builtin_commands_list() {
        let builtins = builtin_commands();
        let names: Vec<&str> = builtins.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"cd"));
        assert!(names.contains(&"pwd"));
        assert!(names.contains(&"exit"));
        assert!(names.contains(&"exit-daemon"));
        assert!(names.contains(&"reload-commands"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-core -- commands`
Expected: FAIL — module doesn't exist yet

**Step 3: Write minimal implementation**

In `crates/betcode-core/src/commands/mod.rs`:
- Define `CommandCategory` enum: `Service`, `ClaudeCode`, `Plugin`
- Define `ExecutionMode` enum: `Local`, `Passthrough`, `Plugin`
- Define `CommandEntry` struct with: `name: String`, `description: String`, `category`, `execution_mode`, `source: String`, `args_schema: Option<String>`
- Implement `CommandEntry::new()`
- `pub mod builtin;` and reexport `builtin_commands()`

In `crates/betcode-core/src/commands/builtin.rs`:
- Define `builtin_commands() -> Vec<CommandEntry>` returning the 5 built-in service commands (cd, pwd, exit, exit-daemon, reload-commands)

In `crates/betcode-core/src/lib.rs`:
- Add `pub mod commands;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-core -- commands`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-core/src/commands/ crates/betcode-core/src/lib.rs
git commit -m "feat(core): add Command types, traits, and built-in command list"
```

---

### Task 3: Claude Code Command Discovery

**Files:**
- Create: `crates/betcode-core/src/commands/discovery.rs`

**Step 1: Write the failing test**

In `crates/betcode-core/src/commands/discovery.rs`, test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_hardcoded_commands_exist() {
        let cmds = hardcoded_cc_commands("1.0.0");
        let names: Vec<&str> = cmds.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"help"));
        assert!(names.contains(&"clear"));
        assert!(names.contains(&"compact"));
    }

    #[test]
    fn test_discover_user_commands_from_directory() {
        let dir = TempDir::new().unwrap();
        let commands_dir = dir.path().join(".claude").join("commands");
        std::fs::create_dir_all(&commands_dir).unwrap();
        std::fs::write(commands_dir.join("deploy.md"), "# Deploy command").unwrap();
        std::fs::write(commands_dir.join("test-all.md"), "# Test all").unwrap();

        let cmds = discover_user_commands(dir.path());
        let names: Vec<&str> = cmds.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"deploy"));
        assert!(names.contains(&"test-all"));
        assert_eq!(cmds[0].category, CommandCategory::ClaudeCode);
        assert_eq!(cmds[0].execution_mode, ExecutionMode::Passthrough);
    }

    #[test]
    fn test_discover_user_commands_missing_dir() {
        let dir = TempDir::new().unwrap();
        let cmds = discover_user_commands(dir.path());
        assert!(cmds.is_empty());
    }

    #[test]
    fn test_parse_help_output() {
        let help_text = r#"
Usage: claude [options]

Commands:
  /help        Show help
  /clear       Clear conversation
  /compact     Compact conversation
  /unknown-new Some new command
        "#;
        let hardcoded = hardcoded_cc_commands("1.0.0");
        let (known, unknown) = parse_help_output(help_text, &hardcoded);
        assert!(known.iter().any(|c| c.name == "help"));
        assert!(unknown.iter().any(|c| c.name == "unknown-new"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-core -- discovery`
Expected: FAIL — functions don't exist

**Step 3: Write minimal implementation**

In `crates/betcode-core/src/commands/discovery.rs`:
- `hardcoded_cc_commands(version: &str) -> Vec<CommandEntry>` — return known CC commands for version. Start with a baseline list (help, clear, compact, exit, config, etc.). No need for complex version ranges yet — a single list covering current CC version.
- `discover_user_commands(working_dir: &Path) -> Vec<CommandEntry>` — read `.claude/commands/*.md`, strip `.md` extension, create `CommandEntry` with `ClaudeCode` category and `Passthrough` execution mode.
- `parse_help_output(help_text: &str, hardcoded: &[CommandEntry]) -> (Vec<CommandEntry>, Vec<CommandEntry>)` — regex parse `/command` patterns from help text. Return (known, unknown) where known means it exists in hardcoded list, unknown means it doesn't.

Add `tempfile` to dev-dependencies of betcode-core `Cargo.toml`.

In `crates/betcode-core/src/commands/mod.rs`:
- Add `pub mod discovery;` and reexport the three functions.

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-core -- discovery`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-core/src/commands/discovery.rs crates/betcode-core/Cargo.toml
git commit -m "feat(core): add Claude Code command discovery (hardcoded + fs + help parse)"
```

---

## Phase 2: Daemon Command Registry & Service Executor

### Task 4: CommandRegistry

**Files:**
- Create: `crates/betcode-daemon/src/commands/mod.rs`
- Modify: `crates/betcode-daemon/src/lib.rs` or `main.rs` (add `mod commands;`)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_loads_builtins() {
        let registry = CommandRegistry::new();
        let entries = registry.get_all();
        assert!(entries.iter().any(|e| e.name == "cd"));
        assert!(entries.iter().any(|e| e.name == "pwd"));
    }

    #[test]
    fn test_registry_add_commands() {
        let mut registry = CommandRegistry::new();
        let cmd = CommandEntry::new(
            "deploy",
            "Deploy the app",
            CommandCategory::ClaudeCode,
            ExecutionMode::Passthrough,
            "claude-code",
        );
        registry.add(cmd);
        assert!(registry.get_all().iter().any(|e| e.name == "deploy"));
    }

    #[test]
    fn test_registry_fuzzy_search() {
        let registry = CommandRegistry::new();
        let results = registry.search("pw", 10);
        assert!(!results.is_empty());
        assert_eq!(results[0].name, "pwd");
    }

    #[test]
    fn test_registry_clear_and_reload() {
        let mut registry = CommandRegistry::new();
        let initial_count = registry.get_all().len();
        registry.clear_source("claude-code");
        let after_clear = registry.get_all().len();
        // Built-ins should remain, only claude-code cleared
        assert_eq!(after_clear, initial_count); // no cc commands added yet
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-daemon -- commands`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-daemon/src/commands/mod.rs`:
- `CommandRegistry` struct holding `Vec<CommandEntry>` (or `HashMap<String, CommandEntry>`)
- `new()` — initializes with `builtin_commands()` from betcode-core
- `add(entry)` — add a command
- `get_all() -> Vec<CommandEntry>` — return clone of all entries
- `search(query, max_results) -> Vec<CommandEntry>` — fuzzy match using substring for now (nucleo will come in Phase 3)
- `clear_source(source: &str)` — remove all entries with matching source

Add `pub mod commands;` to daemon's module root.

Add `nucleo` dependency to workspace Cargo.toml and betcode-daemon Cargo.toml (for later use, but set up now).

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-daemon -- commands`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-daemon/src/commands/ crates/betcode-daemon/src/main.rs Cargo.toml crates/betcode-daemon/Cargo.toml
git commit -m "feat(daemon): add CommandRegistry with built-in commands and fuzzy search"
```

---

### Task 5: Service Command Executor

**Files:**
- Create: `crates/betcode-daemon/src/commands/service_executor.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_execute_pwd() {
        let dir = TempDir::new().unwrap();
        let mut executor = ServiceExecutor::new(dir.path().to_path_buf());
        let result = executor.execute_pwd().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), dir.path().to_string_lossy().to_string());
    }

    #[tokio::test]
    async fn test_execute_cd_valid() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut executor = ServiceExecutor::new(dir.path().to_path_buf());
        let result = executor.execute_cd("sub").await;
        assert!(result.is_ok());
        assert_eq!(executor.cwd(), sub);
    }

    #[tokio::test]
    async fn test_execute_cd_invalid() {
        let dir = TempDir::new().unwrap();
        let mut executor = ServiceExecutor::new(dir.path().to_path_buf());
        let result = executor.execute_cd("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_execute_bash() {
        let dir = TempDir::new().unwrap();
        let executor = ServiceExecutor::new(dir.path().to_path_buf());
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        executor.execute_bash("echo hello", tx).await.unwrap();

        let mut found_hello = false;
        while let Some(output) = rx.recv().await {
            if let ServiceOutput::Stdout(line) = output {
                if line.contains("hello") {
                    found_hello = true;
                }
            }
        }
        assert!(found_hello);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-daemon -- service_executor`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-daemon/src/commands/service_executor.rs`:
- `ServiceOutput` enum: `Stdout(String)`, `Stderr(String)`, `ExitCode(i32)`, `Error(String)`
- `ServiceExecutor` struct with `cwd: PathBuf`
- `new(cwd: PathBuf) -> Self`
- `cwd(&self) -> &Path`
- `execute_pwd(&self) -> Result<String>`
- `execute_cd(&mut self, path: &str) -> Result<()>` — resolve relative to cwd, validate directory exists, update cwd
- `execute_bash(&self, cmd: &str, output_tx: mpsc::Sender<ServiceOutput>) -> Result<()>` — spawn tokio::process::Command with shell, stream stdout/stderr line-by-line via channel, send ExitCode at end

In `crates/betcode-daemon/src/commands/mod.rs`:
- Add `pub mod service_executor;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-daemon -- service_executor`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-daemon/src/commands/service_executor.rs
git commit -m "feat(daemon): add ServiceExecutor for /cd, /pwd, and !bash commands"
```

---

### Task 6: Claude Code Discovery in Daemon

**Files:**
- Create: `crates/betcode-daemon/src/commands/cc_discovery.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_discover_cc_version() {
        // This test requires `claude` to be installed; skip if not available
        let result = detect_cc_version().await;
        // Just verify it returns Ok or a specific error, don't require claude installed
        // In CI, this will return Err which is fine
        let _ = result;
    }

    #[tokio::test]
    async fn test_full_discovery_with_mock_dir() {
        let dir = TempDir::new().unwrap();
        let commands_dir = dir.path().join(".claude").join("commands");
        std::fs::create_dir_all(&commands_dir).unwrap();
        std::fs::write(commands_dir.join("my-cmd.md"), "# Custom").unwrap();

        let result = discover_all_cc_commands(dir.path(), None).await;
        assert!(result.commands.iter().any(|c| c.name == "my-cmd"));
        // Should also have hardcoded builtins
        assert!(result.commands.iter().any(|c| c.name == "help"));
    }

    #[test]
    fn test_parse_version_string() {
        assert_eq!(parse_version("claude v1.0.22 (anthropic-2024-12-01)"), Some("1.0.22".to_string()));
        assert_eq!(parse_version("claude v2.1.0"), Some("2.1.0".to_string()));
        assert_eq!(parse_version("unexpected output"), None);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-daemon -- cc_discovery`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-daemon/src/commands/cc_discovery.rs`:
- `DiscoveryResult` struct: `commands: Vec<CommandEntry>`, `warnings: Vec<String>`
- `detect_cc_version() -> Result<String>` — run `claude --version`, parse with `parse_version()`
- `parse_version(output: &str) -> Option<String>` — regex extract version number
- `discover_all_cc_commands(working_dir: &Path, help_output: Option<&str>) -> DiscoveryResult`:
  1. Get hardcoded commands via `hardcoded_cc_commands(version)`
  2. Get user commands via `discover_user_commands(working_dir)`
  3. If help_output provided, parse and cross-reference — log warnings for unknown commands
  4. Merge all into single list, deduplicate by name

In `crates/betcode-daemon/src/commands/mod.rs`:
- Add `pub mod cc_discovery;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-daemon -- cc_discovery`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-daemon/src/commands/cc_discovery.rs
git commit -m "feat(daemon): add Claude Code command discovery with version detection"
```

---

## Phase 3: Completion Engine & File Index

### Task 7: Fuzzy Matcher (shared)

**Files:**
- Create: `crates/betcode-core/src/commands/matcher.rs`
- Modify: `crates/betcode-core/Cargo.toml` (add nucleo dependency)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzzy_match_exact() {
        let items = vec!["cd", "pwd", "exit", "exit-daemon", "reload-commands"];
        let results = fuzzy_match("cd", &items, 10);
        assert_eq!(results[0].text, "cd");
    }

    #[test]
    fn test_fuzzy_match_substring() {
        let items = vec!["cd", "pwd", "exit", "exit-daemon", "reload-commands"];
        let results = fuzzy_match("rl", &items, 10);
        assert!(results.iter().any(|r| r.text == "reload-commands"));
    }

    #[test]
    fn test_fuzzy_match_fzf_style() {
        let items = vec!["reload-commands", "remove-plugin", "restart"];
        let results = fuzzy_match("rc", &items, 10);
        // "reload-commands" should score higher than "restart" for "rc"
        assert_eq!(results[0].text, "reload-commands");
    }

    #[test]
    fn test_fuzzy_match_max_results() {
        let items: Vec<String> = (0..100).map(|i| format!("item-{}", i)).collect();
        let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
        let results = fuzzy_match("item", &refs, 5);
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_match_result_has_positions() {
        let items = vec!["reload-commands"];
        let results = fuzzy_match("rc", &items, 10);
        assert!(!results[0].match_positions.is_empty());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-core -- matcher`
Expected: FAIL

**Step 3: Write minimal implementation**

Add `nucleo-matcher` to betcode-core's Cargo.toml.

In `crates/betcode-core/src/commands/matcher.rs`:
- `MatchResult` struct: `text: String`, `score: u32`, `match_positions: Vec<usize>`
- `fuzzy_match(query: &str, items: &[&str], max_results: usize) -> Vec<MatchResult>`:
  - Use `nucleo_matcher::Matcher` with fzf-v2 algorithm
  - Score all items, collect non-zero scores
  - Sort by score descending
  - Truncate to max_results
  - Return with match positions for UI highlighting

In `crates/betcode-core/src/commands/mod.rs`:
- Add `pub mod matcher;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-core -- matcher`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-core/src/commands/matcher.rs crates/betcode-core/Cargo.toml Cargo.toml
git commit -m "feat(core): add fzf-style fuzzy matcher using nucleo"
```

---

### Task 8: File Index with Filesystem Watching

**Files:**
- Create: `crates/betcode-daemon/src/completion/mod.rs`
- Create: `crates/betcode-daemon/src/completion/file_index.rs`
- Modify: `crates/betcode-daemon/Cargo.toml` (add notify dependency)
- Modify: `crates/betcode-daemon/src/main.rs` (add `mod completion;`)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_file_index_build() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file1.rs"), "").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "").unwrap();

        let index = FileIndex::build(dir.path(), 1000).await.unwrap();
        assert!(index.search("file1", 10).iter().any(|p| p.path.contains("file1.rs")));
        assert!(index.search("main", 10).iter().any(|p| p.path.contains("main.rs")));
    }

    #[tokio::test]
    async fn test_file_index_respects_max_entries() {
        let dir = TempDir::new().unwrap();
        for i in 0..20 {
            std::fs::write(dir.path().join(format!("file{}.txt", i)), "").unwrap();
        }

        let index = FileIndex::build(dir.path(), 10).await.unwrap();
        assert!(index.entry_count() <= 10);
    }

    #[tokio::test]
    async fn test_file_index_fuzzy_search() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("README.md"), "").unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();

        let index = FileIndex::build(dir.path(), 1000).await.unwrap();
        let results = index.search("rdm", 10);
        assert!(results.iter().any(|p| p.path.contains("README")));
    }

    #[tokio::test]
    async fn test_file_index_returns_path_kind() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("file.rs"), "").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let index = FileIndex::build(dir.path(), 1000).await.unwrap();
        let files = index.search("file.rs", 10);
        assert_eq!(files[0].kind, PathKind::File);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-daemon -- file_index`
Expected: FAIL

**Step 3: Write minimal implementation**

Add `notify = "7"` to betcode-daemon's Cargo.toml.

In `crates/betcode-daemon/src/completion/file_index.rs`:
- `PathKind` enum: `File`, `Directory`, `Symlink`
- `IndexedPath` struct: `path: String` (relative to root), `kind: PathKind`
- `FileIndex` struct: `entries: Vec<IndexedPath>`, `root: PathBuf`
- `build(root: &Path, max_entries: usize) -> Result<Self>`:
  1. Try `git ls-files` + `git ls-files --others --exclude-standard` first
  2. Fallback to walkdir if not a git repo
  3. Collect up to max_entries
- `search(&self, query: &str, max_results: usize) -> Vec<IndexedPath>`:
  - Use `fuzzy_match` from betcode-core
- `entry_count(&self) -> usize`
- `start_watching(&self) -> Result<notify::RecommendedWatcher>`:
  - Set up notify watcher on root dir
  - On create/delete/rename events, update entries vec
  - Use `Arc<RwLock<Vec<IndexedPath>>>` for concurrent access

In `crates/betcode-daemon/src/completion/mod.rs`:
- `pub mod file_index;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-daemon -- file_index`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-daemon/src/completion/ crates/betcode-daemon/Cargo.toml Cargo.toml
git commit -m "feat(daemon): add file index with git-aware building and fuzzy search"
```

---

### Task 9: Agent Lister

**Files:**
- Create: `crates/betcode-daemon/src/completion/agent_lister.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_info_creation() {
        let agent = AgentInfo {
            name: "researcher".to_string(),
            kind: AgentKind::ClaudeInternal,
            status: AgentStatus::Working,
            session_id: Some("sess-123".to_string()),
        };
        assert_eq!(agent.name, "researcher");
    }

    #[test]
    fn test_agent_lister_search() {
        let mut lister = AgentLister::new();
        lister.update(AgentInfo {
            name: "researcher".to_string(),
            kind: AgentKind::ClaudeInternal,
            status: AgentStatus::Working,
            session_id: None,
        });
        lister.update(AgentInfo {
            name: "team-lead".to_string(),
            kind: AgentKind::TeamMember,
            status: AgentStatus::Idle,
            session_id: None,
        });

        let results = lister.search("res", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "researcher");
    }

    #[test]
    fn test_agent_lister_empty_query_returns_all() {
        let mut lister = AgentLister::new();
        lister.update(AgentInfo {
            name: "a".to_string(),
            kind: AgentKind::ClaudeInternal,
            status: AgentStatus::Idle,
            session_id: None,
        });
        lister.update(AgentInfo {
            name: "b".to_string(),
            kind: AgentKind::TeamMember,
            status: AgentStatus::Working,
            session_id: None,
        });
        let results = lister.search("", 10);
        assert_eq!(results.len(), 2);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-daemon -- agent_lister`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-daemon/src/completion/agent_lister.rs`:
- `AgentKind` enum: `ClaudeInternal`, `DaemonOrchestrated`, `TeamMember`
- `AgentStatus` enum: `Idle`, `Working`, `Done`, `Failed`
- `AgentInfo` struct: `name`, `kind`, `status`, `session_id`
- `AgentLister` struct: `agents: HashMap<String, AgentInfo>`
- `new() -> Self`
- `update(info: AgentInfo)` — insert/update by name
- `remove(name: &str)`
- `search(query: &str, max_results: usize) -> Vec<AgentInfo>`:
  - If empty query, return all (up to max)
  - Otherwise fuzzy match against names using betcode-core matcher

In `crates/betcode-daemon/src/completion/mod.rs`:
- Add `pub mod agent_lister;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-daemon -- agent_lister`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-daemon/src/completion/agent_lister.rs
git commit -m "feat(daemon): add AgentLister for @-prefix completion"
```

---

## Phase 4: gRPC CommandService Handler

### Task 10: CommandService gRPC Implementation

**Files:**
- Create: `crates/betcode-daemon/src/server/command_svc.rs`
- Modify: `crates/betcode-daemon/src/server/mod.rs` (register new service)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use betcode_proto::betcode::v1::*;

    #[tokio::test]
    async fn test_get_command_registry() {
        let registry = Arc::new(RwLock::new(CommandRegistry::new()));
        let file_index = Arc::new(RwLock::new(FileIndex::empty()));
        let agent_lister = Arc::new(RwLock::new(AgentLister::new()));
        let service = CommandServiceImpl::new(registry, file_index, agent_lister);

        let request = tonic::Request::new(GetCommandRegistryRequest {});
        let response = service.get_command_registry(request).await.unwrap();
        let entries = response.into_inner().commands;
        assert!(entries.iter().any(|e| e.name == "cd"));
    }

    #[tokio::test]
    async fn test_list_path() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("test.rs"), "").unwrap();

        let registry = Arc::new(RwLock::new(CommandRegistry::new()));
        let file_index = Arc::new(RwLock::new(
            FileIndex::build(dir.path(), 1000).await.unwrap()
        ));
        let agent_lister = Arc::new(RwLock::new(AgentLister::new()));
        let service = CommandServiceImpl::new(registry, file_index, agent_lister);

        let request = tonic::Request::new(ListPathRequest {
            query: "test".to_string(),
            max_results: 10,
        });
        let response = service.list_path(request).await.unwrap();
        assert!(!response.into_inner().entries.is_empty());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-daemon -- command_svc`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-daemon/src/server/command_svc.rs`:
- `CommandServiceImpl` struct holding `Arc<RwLock<CommandRegistry>>`, `Arc<RwLock<FileIndex>>`, `Arc<RwLock<AgentLister>>`
- Implement tonic-generated `CommandService` trait:
  - `get_command_registry()` — read-lock registry, convert to proto messages
  - `list_agents()` — read-lock agent lister, search, convert to proto
  - `list_path()` — read-lock file index, search, convert to proto
  - `execute_service_command()` — server-streaming: parse command, dispatch to ServiceExecutor, stream output as ServiceCommandOutput
  - Plugin management RPCs — stub with `unimplemented!()` for now (Phase 6)

In `crates/betcode-daemon/src/server/mod.rs`:
- Add `pub mod command_svc;`
- Register `CommandServiceImpl` alongside existing `AgentServiceImpl` in the server builder

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-daemon -- command_svc`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-daemon/src/server/command_svc.rs crates/betcode-daemon/src/server/mod.rs
git commit -m "feat(daemon): add CommandService gRPC handler with registry, paths, and agents"
```

---

## Phase 5: CLI Autocomplete UI

### Task 11: Completion Controller (trigger detection + debounce)

**Files:**
- Create: `crates/betcode-cli/src/completion/mod.rs`
- Create: `crates/betcode-cli/src/completion/controller.rs`
- Modify: `crates/betcode-cli/src/app/state.rs` (add completion state to App)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_trigger_slash() {
        let trigger = detect_trigger("/hel", 4);
        assert_eq!(trigger, Some(CompletionTrigger::Command { query: "hel".to_string() }));
    }

    #[test]
    fn test_detect_trigger_at_agent() {
        let trigger = detect_trigger("@res", 4);
        assert_eq!(trigger, Some(CompletionTrigger::Agent { query: "res".to_string() }));
    }

    #[test]
    fn test_detect_trigger_at_file_explicit() {
        let trigger = detect_trigger("@/src/main", 10);
        assert_eq!(trigger, Some(CompletionTrigger::File { query: "/src/main".to_string() }));
    }

    #[test]
    fn test_detect_trigger_at_file_implicit() {
        let trigger = detect_trigger("@README.md", 10);
        assert_eq!(trigger, Some(CompletionTrigger::File { query: "README.md".to_string() }));
    }

    #[test]
    fn test_detect_trigger_at_force_agent() {
        let trigger = detect_trigger("@@res", 5);
        assert_eq!(trigger, Some(CompletionTrigger::Agent { query: "res".to_string() }));
    }

    #[test]
    fn test_detect_trigger_bang() {
        let trigger = detect_trigger("!ls -la", 7);
        assert_eq!(trigger, Some(CompletionTrigger::Bash { cmd: "ls -la".to_string() }));
    }

    #[test]
    fn test_detect_trigger_none() {
        let trigger = detect_trigger("hello world", 11);
        assert_eq!(trigger, None);
    }

    #[test]
    fn test_at_path_detection() {
        assert!(looks_like_path("src/main.rs"));
        assert!(looks_like_path("README.md"));
        assert!(looks_like_path("./file"));
        assert!(looks_like_path("../file"));
        assert!(!looks_like_path("researcher"));
        assert!(!looks_like_path("team-lead"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-cli -- controller`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-cli/src/completion/controller.rs`:
- `CompletionTrigger` enum: `Command { query }`, `Agent { query }`, `File { query }`, `Bash { cmd }`, `Mixed { query }` (both agents and files)
- `detect_trigger(input: &str, cursor_pos: usize) -> Option<CompletionTrigger>`:
  - Extract the token at/before cursor position
  - `/...` → Command
  - `@@...` → Agent (forced)
  - `@/...` or `@./...` or `@../...` → File (forced)
  - `@text` with path chars → File
  - `@text` without path chars → Agent (or Mixed if ambiguous)
  - `!...` → Bash
  - Otherwise → None
- `looks_like_path(text: &str) -> bool` — contains `/`, or matches `*.ext` pattern, or starts with `./` or `../`

In `crates/betcode-cli/src/completion/mod.rs`:
- `pub mod controller;`

Add `mod completion;` to CLI's main.rs or lib.

In `crates/betcode-cli/src/app/state.rs`:
- Add to `App` struct:
  - `completion_state: CompletionState`
- Define `CompletionState`:
  - `popup_visible: bool`
  - `items: Vec<CompletionItem>`
  - `selected_index: usize`
  - `ghost_text: Option<String>`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-cli -- controller`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-cli/src/completion/ crates/betcode-cli/src/app/state.rs crates/betcode-cli/src/main.rs
git commit -m "feat(cli): add completion trigger detection and CompletionState"
```

---

### Task 12: Ghost Text Renderer

**Files:**
- Create: `crates/betcode-cli/src/completion/ghost.rs`
- Modify: `crates/betcode-cli/src/ui/render.rs` (integrate ghost text into input line)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style};

    #[test]
    fn test_ghost_spans_basic() {
        let spans = ghost_text_spans("hel", Some("help"));
        // Should produce: typed text "hel" (normal) + ghost "p" (dimmed)
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content, "p");
        assert_eq!(spans[1].style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn test_ghost_spans_no_completion() {
        let spans = ghost_text_spans("hello", None);
        assert_eq!(spans.len(), 1); // just the typed text
    }

    #[test]
    fn test_ghost_spans_exact_match() {
        let spans = ghost_text_spans("help", Some("help"));
        // Exact match — no ghost suffix needed
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn test_ghost_completion_extraction() {
        assert_eq!(ghost_suffix("hel", "help"), Some("p"));
        assert_eq!(ghost_suffix("/cd", "/cd"), None);
        assert_eq!(ghost_suffix("/re", "/reload-commands"), Some("load-commands"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-cli -- ghost`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-cli/src/completion/ghost.rs`:
- `ghost_suffix(typed: &str, completion: &str) -> Option<&str>` — if completion starts with typed text (case-insensitive), return the remaining suffix. Otherwise None.
- `ghost_text_spans(typed: &str, completion: Option<&str>) -> Vec<Span>`:
  - If no completion or exact match, return just typed text span
  - Otherwise, return typed text span (normal style) + suffix span (DarkGray/dimmed style)

In `crates/betcode-cli/src/completion/mod.rs`:
- Add `pub mod ghost;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-cli -- ghost`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-cli/src/completion/ghost.rs
git commit -m "feat(cli): add ghost text renderer for inline completion preview"
```

---

### Task 13: Floating Overlay Popup Widget

**Files:**
- Create: `crates/betcode-cli/src/completion/popup.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_completion_item_display() {
        let item = CompletionItem {
            text: "reload-commands".to_string(),
            description: "Re-discover all commands".to_string(),
            category: CompletionCategory::Command,
            source_badge: "[bc]".to_string(),
            match_positions: vec![0, 7],
        };
        assert_eq!(item.text, "reload-commands");
    }

    #[test]
    fn test_popup_state_navigation() {
        let items = vec![
            CompletionItem::simple("cd", "Change dir", CompletionCategory::Command),
            CompletionItem::simple("pwd", "Print dir", CompletionCategory::Command),
            CompletionItem::simple("exit", "Exit CLI", CompletionCategory::Command),
        ];
        let mut state = PopupState::new(items, 8);
        assert_eq!(state.selected_index(), 0);

        state.move_down();
        assert_eq!(state.selected_index(), 1);

        state.move_down();
        assert_eq!(state.selected_index(), 2);

        state.move_down(); // wrap around
        assert_eq!(state.selected_index(), 0);

        state.move_up(); // wrap to end
        assert_eq!(state.selected_index(), 2);
    }

    #[test]
    fn test_popup_visible_window() {
        let items: Vec<CompletionItem> = (0..20)
            .map(|i| CompletionItem::simple(&format!("item-{}", i), "", CompletionCategory::Command))
            .collect();
        let state = PopupState::new(items, 5); // visible window = 5

        let visible = state.visible_items();
        assert_eq!(visible.len(), 5);
    }

    #[test]
    fn test_popup_accept_returns_selected() {
        let items = vec![
            CompletionItem::simple("cd", "", CompletionCategory::Command),
            CompletionItem::simple("pwd", "", CompletionCategory::Command),
        ];
        let mut state = PopupState::new(items, 8);
        state.move_down();
        let accepted = state.accept();
        assert_eq!(accepted.text, "pwd");
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-cli -- popup`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-cli/src/completion/popup.rs`:
- `CompletionCategory` enum: `Command`, `Agent`, `File`, `Plugin`
- `CompletionItem` struct: `text`, `description`, `category`, `source_badge`, `match_positions: Vec<usize>`
- `CompletionItem::simple(text, desc, cat) -> Self` — convenience constructor
- `PopupState` struct: `items: Vec<CompletionItem>`, `selected: usize`, `visible_count: usize`, `scroll_offset: usize`
- `new(items, visible_count) -> Self`
- `selected_index() -> usize`
- `move_up()` / `move_down()` — with wraparound
- `visible_items() -> &[CompletionItem]` — returns the window slice `[scroll_offset..scroll_offset+visible_count]`
- `accept() -> CompletionItem` — return selected item
- `render(area: Rect, buf: &mut Buffer)` — ratatui StatefulWidget impl rendering the popup:
  - Background: dark panel
  - Each visible item: category badge + name (with match chars highlighted) + description
  - Selected item highlighted

In `crates/betcode-cli/src/completion/mod.rs`:
- Add `pub mod popup;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-cli -- popup`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-cli/src/completion/popup.rs
git commit -m "feat(cli): add floating overlay popup widget with virtualized rendering"
```

---

### Task 14: Input Handling Integration

**Files:**
- Modify: `crates/betcode-cli/src/tui/input.rs` (add completion keybindings)
- Modify: `crates/betcode-cli/src/ui/render.rs` (render ghost text + popup)
- Modify: `crates/betcode-cli/src/app/state.rs` (completion state transitions)

**Step 1: Write the failing test**

Add tests to existing `crates/betcode-cli/src/tui/input.rs` test module (if one exists) or create a new one:

```rust
#[cfg(test)]
mod completion_input_tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn test_tab_toggles_popup() {
        let mut app = App::default();
        app.input = "/he".to_string();
        app.cursor_pos = 3;
        // Simulate trigger detection
        app.update_completion_state();

        handle_completion_key(&mut app, key(KeyCode::Tab));
        assert!(app.completion_state.popup_visible);

        handle_completion_key(&mut app, key(KeyCode::Tab));
        assert!(!app.completion_state.popup_visible);
    }

    #[test]
    fn test_escape_closes_popup() {
        let mut app = App::default();
        app.completion_state.popup_visible = true;

        handle_completion_key(&mut app, key(KeyCode::Esc));
        assert!(!app.completion_state.popup_visible);
    }

    #[test]
    fn test_enter_accepts_completion() {
        let mut app = App::default();
        app.input = "/he".to_string();
        app.completion_state.popup_visible = true;
        app.completion_state.items = vec![
            CompletionItem::simple("help", "Show help", CompletionCategory::Command),
        ];
        app.completion_state.selected_index = 0;

        let action = handle_completion_key(&mut app, key(KeyCode::Enter));
        assert_eq!(action, CompletionAction::Accept("help".to_string()));
    }

    #[test]
    fn test_ctrl_i_shows_status_panel() {
        let mut app = App::default();
        handle_term_event_ext(&mut app, ctrl_key(KeyCode::Char('i')));
        assert!(app.show_status_panel);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-cli -- completion_input`
Expected: FAIL

**Step 3: Write implementation**

In `crates/betcode-cli/src/tui/input.rs`:
- Add `handle_completion_key(app: &mut App, key: KeyEvent) -> CompletionAction` function:
  - `Tab` → toggle `popup_visible`
  - `Up/Down` → move selection in popup
  - `Enter` or `Space` (when popup visible) → accept selected item
  - `Escape` → close popup
- Add `Ctrl+I` handling to main `handle_term_event()` → set `app.show_status_panel = true`
- When input changes, call `app.update_completion_state()` which runs trigger detection

In `crates/betcode-cli/src/app/state.rs`:
- Add `show_status_panel: bool` to App
- Add `update_completion_state(&mut self)` method:
  - Calls `detect_trigger(self.input, self.cursor_pos)`
  - Based on trigger type, updates `completion_state.items` from cache or marks for async fetch
  - Updates `ghost_text` to first item's text

In `crates/betcode-cli/src/ui/render.rs`:
- In the input line rendering: integrate `ghost_text_spans()` to show ghost text
- After rendering main layout: if `completion_state.popup_visible`, render popup overlay **above** the input line
- If `show_status_panel`, render status panel overlay (dismiss on any key)

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-cli -- completion_input`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-cli/src/tui/input.rs crates/betcode-cli/src/ui/render.rs crates/betcode-cli/src/app/state.rs
git commit -m "feat(cli): integrate completion keybindings, ghost text, and popup into TUI"
```

---

### Task 15: Session Status Panel

**Files:**
- Create: `crates/betcode-cli/src/ui/status_panel.rs`
- Modify: `crates/betcode-cli/src/ui/mod.rs` (add module)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn test_status_panel_render() {
        let info = SessionStatusInfo {
            cwd: "/home/user/project".to_string(),
            session_id: "sess-abc123".to_string(),
            connection: "local".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            active_agents: 2,
            pending_permissions: 0,
            worktree: Some("feature/auth".to_string()),
            uptime_secs: 3600,
        };

        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| {
            render_status_panel(f, f.area(), &info);
        }).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content = buffer_to_string(&buffer);
        assert!(content.contains("/home/user/project"));
        assert!(content.contains("sess-abc123"));
        assert!(content.contains("claude-sonnet"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-cli -- status_panel`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-cli/src/ui/status_panel.rs`:
- `SessionStatusInfo` struct with all fields
- `render_status_panel(frame: &mut Frame, area: Rect, info: &SessionStatusInfo)`:
  - Render a centered bordered panel (Block with title "Session Status")
  - Each field as a labeled row: `CWD: /home/user/project`
  - Format uptime as `Xh Ym Zs`
  - Optional fields (worktree) shown only if present

In `crates/betcode-cli/src/ui/mod.rs`:
- Add `pub mod status_panel;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-cli -- status_panel`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-cli/src/ui/status_panel.rs crates/betcode-cli/src/ui/mod.rs
git commit -m "feat(cli): add Ctrl+I session status panel overlay"
```

---

## Phase 6: CLI Command Cache & gRPC Integration

### Task 16: CLI Command Cache

**Files:**
- Create: `crates/betcode-cli/src/commands/mod.rs`
- Create: `crates/betcode-cli/src/commands/cache.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_load_and_search() {
        let entries = vec![
            CachedCommand { name: "cd".into(), description: "Change dir".into(), category: "SERVICE".into(), source: "built-in".into() },
            CachedCommand { name: "help".into(), description: "Show help".into(), category: "CLAUDE_CODE".into(), source: "claude-code".into() },
        ];
        let mut cache = CommandCache::new();
        cache.load(entries);

        let results = cache.search("cd", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "cd");
    }

    #[test]
    fn test_cache_is_empty_initially() {
        let cache = CommandCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.search("anything", 10).len(), 0);
    }

    #[test]
    fn test_cache_reload_replaces() {
        let mut cache = CommandCache::new();
        cache.load(vec![CachedCommand { name: "old".into(), description: "".into(), category: "SERVICE".into(), source: "built-in".into() }]);
        assert_eq!(cache.search("old", 10).len(), 1);

        cache.load(vec![CachedCommand { name: "new".into(), description: "".into(), category: "SERVICE".into(), source: "built-in".into() }]);
        assert_eq!(cache.search("old", 10).len(), 0);
        assert_eq!(cache.search("new", 10).len(), 1);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-cli -- cache`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-cli/src/commands/cache.rs`:
- `CachedCommand` struct: `name`, `description`, `category`, `source`
- `CommandCache` struct: `entries: Vec<CachedCommand>`
- `new() -> Self`
- `is_empty() -> bool`
- `load(entries: Vec<CachedCommand>)` — replaces all entries
- `search(query: &str, max_results: usize) -> Vec<&CachedCommand>` — fuzzy match using betcode-core matcher

In `crates/betcode-cli/src/commands/mod.rs`:
- `pub mod cache;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-cli -- cache`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-cli/src/commands/
git commit -m "feat(cli): add CommandCache for local command completion"
```

---

### Task 17: CLI gRPC Client for Completions

**Files:**
- Modify: `crates/betcode-cli/src/connection.rs` (add command service methods)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // These are integration tests that need a running daemon.
    // For unit tests, verify the client methods exist and have correct signatures.
    #[test]
    fn test_connection_has_command_methods() {
        // Compile-time check: these methods must exist
        let _: fn(&DaemonConnection) -> _ = DaemonConnection::fetch_command_registry;
        let _: fn(&DaemonConnection, &str, u32) -> _ = DaemonConnection::list_agents;
        let _: fn(&DaemonConnection, &str, u32) -> _ = DaemonConnection::list_path;
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-cli -- connection`
Expected: FAIL — methods don't exist

**Step 3: Write minimal implementation**

In `crates/betcode-cli/src/connection.rs`:
- Add `command_client: Option<CommandServiceClient<Channel>>` field to `DaemonConnection`
- Initialize alongside existing agent client
- `fetch_command_registry(&self) -> Result<Vec<CommandEntry>>` — call `GetCommandRegistry` RPC
- `list_agents(&self, query: &str, max_results: u32) -> Result<Vec<AgentInfo>>` — call `ListAgents` RPC
- `list_path(&self, query: &str, max_results: u32) -> Result<Vec<PathEntry>>` — call `ListPath` RPC
- `execute_service_command(&self, command: &str, args: &str) -> Result<Streaming<ServiceCommandOutput>>` — call `ExecuteServiceCommand` RPC (server-streaming)

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-cli -- connection`
Expected: PASS (compile-time checks)

**Step 5: Commit**

```bash
git add crates/betcode-cli/src/connection.rs
git commit -m "feat(cli): add CommandService gRPC client methods to DaemonConnection"
```

---

### Task 18: Wire Completions to gRPC (async fetch loop)

**Files:**
- Modify: `crates/betcode-cli/src/tui/mod.rs` (add completion fetch channel)
- Modify: `crates/betcode-cli/src/app/state.rs` (connect completion state to async results)

**Step 1: Design the async flow**

The CLI has a two-thread architecture:
- OS thread: reads crossterm events, sends to channel
- Tokio runtime: receives events, processes gRPC, renders

For completions:
- When trigger is detected and items need fetching (agents, files), send a `CompletionRequest` to a dedicated tokio task
- The task calls the appropriate gRPC RPC (debounced at 100ms)
- Results come back via a `CompletionResponse` channel
- Main loop picks up responses and updates `CompletionState`

**Step 2: Write the integration**

In `crates/betcode-cli/src/tui/mod.rs`:
- Add a `tokio::sync::mpsc` channel pair: `completion_tx`, `completion_rx`
- Spawn a dedicated `completion_fetcher` task that:
  - Receives `CompletionRequest { trigger, query }`
  - Debounces (100ms since last request)
  - Calls appropriate RPC: `list_agents` or `list_path`
  - Sends back `CompletionResponse { items }`
- In the main event loop `select!`, add a branch for `completion_rx.recv()`
- On initial connect, call `fetch_command_registry` and load into `CommandCache`

In `crates/betcode-cli/src/app/state.rs`:
- `update_completion_state()` now:
  - For `/` triggers: search local `CommandCache` (instant)
  - For `@`/file triggers: send `CompletionRequest` to fetcher channel
  - Ghost text updates from cached results or pending results

**Step 3: Verify full flow**

This is a wiring task — write a manual integration test or verify via `cargo build -p betcode-cli`.

Run: `cargo build -p betcode-cli`
Expected: Clean compile

**Step 4: Commit**

```bash
git add crates/betcode-cli/src/tui/mod.rs crates/betcode-cli/src/app/state.rs
git commit -m "feat(cli): wire completion UI to gRPC with async fetch and debounce"
```

---

## Phase 7: Service Command Execution in CLI

### Task 19: Local Command Interception

**Files:**
- Modify: `crates/betcode-cli/src/tui/input.rs` (intercept service commands before sending to daemon)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod service_command_tests {
    use super::*;

    #[test]
    fn test_is_service_command() {
        assert!(is_service_command("/cd /tmp"));
        assert!(is_service_command("/pwd"));
        assert!(is_service_command("/exit"));
        assert!(is_service_command("/exit-daemon"));
        assert!(is_service_command("!ls -la"));
        assert!(!is_service_command("hello world"));
        assert!(!is_service_command("/help")); // Claude Code command, not service
    }

    #[test]
    fn test_parse_service_command() {
        let cmd = parse_service_command("/cd /tmp").unwrap();
        assert_eq!(cmd, ServiceCommand::Cd { path: "/tmp".to_string() });

        let cmd = parse_service_command("/pwd").unwrap();
        assert_eq!(cmd, ServiceCommand::Pwd);

        let cmd = parse_service_command("/exit").unwrap();
        assert_eq!(cmd, ServiceCommand::Exit);

        let cmd = parse_service_command("!echo hello").unwrap();
        assert_eq!(cmd, ServiceCommand::Bash { cmd: "echo hello".to_string() });
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-cli -- service_command`
Expected: FAIL

**Step 3: Write implementation**

In `crates/betcode-cli/src/tui/input.rs` (or a new file `crates/betcode-cli/src/commands/service.rs`):
- `ServiceCommand` enum: `Cd { path }`, `Pwd`, `Exit`, `ExitDaemon`, `ReloadCommands`, `Bash { cmd }`
- `is_service_command(input: &str) -> bool` — check against known service command prefixes
- `parse_service_command(input: &str) -> Option<ServiceCommand>` — parse input into typed command
- In the submit handler (when user presses Enter):
  1. Check `is_service_command(input)`
  2. If yes: parse and dispatch locally (e.g., `/exit` → exit process) or via `execute_service_command` RPC
  3. If no: send as `UserMessage` to Claude Code via existing path

For `/exit`: directly trigger CLI shutdown, no daemon involvement.
For others: call `ExecuteServiceCommand` RPC, stream results into chat as system messages.

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-cli -- service_command`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-cli/src/tui/input.rs crates/betcode-cli/src/commands/
git commit -m "feat(cli): intercept and execute service commands (/cd, /pwd, /exit, !bash)"
```

---

## Phase 8: Plugin System

### Task 20: Plugin Config

**Files:**
- Create: `crates/betcode-daemon/src/plugin/mod.rs`
- Create: `crates/betcode-daemon/src/plugin/config.rs`
- Modify: `crates/betcode-daemon/src/main.rs` (add `mod plugin;`)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_plugin_config() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("daemon.toml");
        std::fs::write(&config_path, r#"
[[plugins]]
name = "test-plugin"
socket = "/tmp/test.sock"
enabled = true
timeout_secs = 30
"#).unwrap();

        let config = PluginConfig::load(&config_path).unwrap();
        assert_eq!(config.plugins.len(), 1);
        assert_eq!(config.plugins[0].name, "test-plugin");
        assert!(config.plugins[0].enabled);
    }

    #[test]
    fn test_add_plugin_to_config() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("daemon.toml");
        std::fs::write(&config_path, "").unwrap();

        let mut config = PluginConfig::load(&config_path).unwrap();
        config.add_plugin("new-plugin", "/tmp/new.sock");
        config.save(&config_path).unwrap();

        let reloaded = PluginConfig::load(&config_path).unwrap();
        assert_eq!(reloaded.plugins.len(), 1);
    }

    #[test]
    fn test_remove_plugin_from_config() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("daemon.toml");
        std::fs::write(&config_path, r#"
[[plugins]]
name = "test-plugin"
socket = "/tmp/test.sock"
enabled = true
timeout_secs = 30
"#).unwrap();

        let mut config = PluginConfig::load(&config_path).unwrap();
        config.remove_plugin("test-plugin");
        config.save(&config_path).unwrap();

        let reloaded = PluginConfig::load(&config_path).unwrap();
        assert!(reloaded.plugins.is_empty());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-daemon -- plugin::config`
Expected: FAIL

**Step 3: Write minimal implementation**

Add `toml = "0.8"` to betcode-daemon's Cargo.toml.

In `crates/betcode-daemon/src/plugin/config.rs`:
- `PluginDeclaration` struct: `name: String`, `socket: String`, `enabled: bool`, `timeout_secs: u64`
- `PluginConfig` struct: `plugins: Vec<PluginDeclaration>`
- `load(path: &Path) -> Result<Self>` — read TOML file, deserialize. If file empty/missing, return empty config.
- `save(path: &Path) -> Result<()>` — serialize and write
- `add_plugin(name, socket)` — add with enabled=true, timeout=30
- `remove_plugin(name)` — remove by name
- `enable_plugin(name)` / `disable_plugin(name)` — toggle enabled

In `crates/betcode-daemon/src/plugin/mod.rs`:
- `pub mod config;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-daemon -- plugin::config`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-daemon/src/plugin/ crates/betcode-daemon/Cargo.toml Cargo.toml
git commit -m "feat(daemon): add plugin config management (TOML-based)"
```

---

### Task 21: Plugin Client (gRPC over Unix Socket)

**Files:**
- Create: `crates/betcode-daemon/src/plugin/client.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_health_state_machine() {
        let mut health = PluginHealth::new(3, 10);
        assert_eq!(health.status(), PluginStatus::Healthy);

        health.record_failure();
        health.record_failure();
        health.record_failure();
        assert_eq!(health.status(), PluginStatus::Degraded);

        for _ in 0..7 {
            health.record_failure();
        }
        assert_eq!(health.status(), PluginStatus::Unavailable);

        health.reset();
        assert_eq!(health.status(), PluginStatus::Healthy);
    }

    #[test]
    fn test_plugin_health_success_resets_failures() {
        let mut health = PluginHealth::new(3, 10);
        health.record_failure();
        health.record_failure();
        health.record_success();
        assert_eq!(health.status(), PluginStatus::Healthy);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-daemon -- plugin::client`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-daemon/src/plugin/client.rs`:
- `PluginStatus` enum: `Healthy`, `Degraded`, `Unavailable`
- `PluginHealth` struct: `consecutive_failures: u32`, `degraded_threshold: u32`, `unavailable_threshold: u32`
- `new(degraded_threshold, unavailable_threshold) -> Self`
- `status() -> PluginStatus`
- `record_failure()` / `record_success()` / `reset()`
- `PluginClient` struct (to be filled in later with actual gRPC):
  - `name: String`
  - `socket_path: String`
  - `health: PluginHealth`
  - `timeout: Duration`
  - `commands: Vec<CommandEntry>` — commands registered by this plugin

For the actual gRPC Unix socket connection, use `tonic::transport::Endpoint::from_shared` with `unix://` scheme. This can be wired up when integration testing with a real plugin.

In `crates/betcode-daemon/src/plugin/mod.rs`:
- Add `pub mod client;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-daemon -- plugin::client`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-daemon/src/plugin/client.rs
git commit -m "feat(daemon): add PluginClient with circuit breaker health tracking"
```

---

### Task 22: Plugin Manager

**Files:**
- Create: `crates/betcode-daemon/src/plugin/manager.rs`

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_plugin_manager_add_remove() {
        let manager = PluginManager::new();
        manager.add_plugin("test", "/tmp/test.sock", Duration::from_secs(30)).await;

        let plugins = manager.list_plugins().await;
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "test");

        manager.remove_plugin("test").await;
        assert!(manager.list_plugins().await.is_empty());
    }

    #[tokio::test]
    async fn test_plugin_manager_enable_disable() {
        let manager = PluginManager::new();
        manager.add_plugin("test", "/tmp/test.sock", Duration::from_secs(30)).await;

        manager.disable_plugin("test").await.unwrap();
        let status = manager.get_plugin_status("test").await.unwrap();
        assert!(!status.enabled);

        manager.enable_plugin("test").await.unwrap();
        let status = manager.get_plugin_status("test").await.unwrap();
        assert!(status.enabled);
    }

    #[tokio::test]
    async fn test_plugin_manager_get_all_commands() {
        let manager = PluginManager::new();
        // Without connected plugins, should return empty
        let commands = manager.get_all_plugin_commands().await;
        assert!(commands.is_empty());
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-daemon -- plugin::manager`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-daemon/src/plugin/manager.rs`:
- `PluginManager` struct: `plugins: Arc<RwLock<HashMap<String, PluginClient>>>`
- `new() -> Self`
- `add_plugin(name, socket_path, timeout)` — create PluginClient, store. Connection attempt happens async (don't block).
- `remove_plugin(name)` — remove and drop client
- `disable_plugin(name)` / `enable_plugin(name)` — toggle
- `list_plugins() -> Vec<PluginSummary>` — return name, status, enabled, command count
- `get_plugin_status(name) -> PluginDetailedStatus` — health, last check time, failure count
- `get_all_plugin_commands() -> Vec<CommandEntry>` — aggregate commands from all healthy+enabled plugins
- `start_health_checks(interval: Duration)` — spawn periodic health check task

Each plugin operation is isolated in its own `tokio::spawn` with `catch_unwind` and timeout.

In `crates/betcode-daemon/src/plugin/mod.rs`:
- Add `pub mod manager;`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-daemon -- plugin::manager`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-daemon/src/plugin/manager.rs
git commit -m "feat(daemon): add PluginManager with lifecycle, enable/disable, and health checks"
```

---

## Phase 9: CLI Plugin Subcommand

### Task 23: `betcode plugin` CLI Commands

**Files:**
- Create: `crates/betcode-cli/src/commands/plugin_cmd.rs`
- Modify: `crates/betcode-cli/src/main.rs` (add Plugin subcommand to clap)

**Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_parse_plugin_add() {
        let args = PluginAction::try_parse_from(["plugin", "add", "my-plugin", "/tmp/plugin.sock"]);
        assert!(args.is_ok());
        match args.unwrap() {
            PluginAction::Add { name, socket } => {
                assert_eq!(name, "my-plugin");
                assert_eq!(socket, "/tmp/plugin.sock");
            },
            _ => panic!("expected Add"),
        }
    }

    #[test]
    fn test_parse_plugin_list() {
        let args = PluginAction::try_parse_from(["plugin", "list"]);
        assert!(matches!(args.unwrap(), PluginAction::List));
    }

    #[test]
    fn test_parse_plugin_remove() {
        let args = PluginAction::try_parse_from(["plugin", "remove", "my-plugin"]);
        assert!(matches!(args.unwrap(), PluginAction::Remove { name } if name == "my-plugin"));
    }

    #[test]
    fn test_parse_plugin_enable_disable() {
        let args = PluginAction::try_parse_from(["plugin", "enable", "p1"]);
        assert!(matches!(args.unwrap(), PluginAction::Enable { name } if name == "p1"));

        let args = PluginAction::try_parse_from(["plugin", "disable", "p1"]);
        assert!(matches!(args.unwrap(), PluginAction::Disable { name } if name == "p1"));
    }

    #[test]
    fn test_parse_plugin_status() {
        let args = PluginAction::try_parse_from(["plugin", "status", "p1"]);
        assert!(matches!(args.unwrap(), PluginAction::Status { name } if name == "p1"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p betcode-cli -- plugin_cmd`
Expected: FAIL

**Step 3: Write minimal implementation**

In `crates/betcode-cli/src/commands/plugin_cmd.rs`:
- `PluginAction` enum (clap Subcommand derive):
  - `Add { name: String, socket: String }`
  - `Remove { name: String }`
  - `List`
  - `Status { name: String }`
  - `Enable { name: String }`
  - `Disable { name: String }`
- `handle_plugin_command(action: PluginAction, connection: &DaemonConnection) -> Result<()>`:
  - Each variant calls the corresponding gRPC RPC
  - Prints results to stdout in a readable table format

In `crates/betcode-cli/src/main.rs`:
- Add `Plugin { action: PluginAction }` to the existing `Commands` enum
- In the match block, dispatch to `handle_plugin_command()`

**Step 4: Run test to verify it passes**

Run: `cargo test -p betcode-cli -- plugin_cmd`
Expected: PASS

**Step 5: Commit**

```bash
git add crates/betcode-cli/src/commands/plugin_cmd.rs crates/betcode-cli/src/main.rs
git commit -m "feat(cli): add `betcode plugin` subcommands (add/remove/list/status/enable/disable)"
```

---

## Phase 10: Integration & Polish

### Task 24: Wire File Index Watching

**Files:**
- Modify: `crates/betcode-daemon/src/completion/file_index.rs` (add watch functionality)
- Modify: `crates/betcode-daemon/src/server/mod.rs` (start file index on session creation)

**Step 1: Implement file watching**

In `file_index.rs`, add:
- `FileIndex::start_watching(&self) -> Result<()>`:
  - Create `notify::RecommendedWatcher` watching the root directory recursively
  - On `Create` event: add entry to index
  - On `Remove` event: remove entry from index
  - On `Rename` event: update path
  - Use `Arc<RwLock<Vec<IndexedPath>>>` for interior mutability

**Step 2: Wire into session startup**

In the daemon's session startup flow (around `SessionRelay` creation):
- Build `FileIndex` for the session's working directory
- Start watching
- Pass `Arc<RwLock<FileIndex>>` to `CommandServiceImpl`

**Step 3: Verify build**

Run: `cargo build -p betcode-daemon`
Expected: Clean compile

**Step 4: Commit**

```bash
git add crates/betcode-daemon/src/completion/file_index.rs crates/betcode-daemon/src/server/
git commit -m "feat(daemon): wire file index watching into session lifecycle"
```

---

### Task 25: Wire Registry Reload (`/reload-commands`)

**Files:**
- Modify: `crates/betcode-daemon/src/commands/service_executor.rs`
- Modify: `crates/betcode-daemon/src/server/command_svc.rs`

**Step 1: Implement reload**

In `service_executor.rs`, add:
- `execute_reload_commands(&self, registry, file_index, plugin_manager)`:
  1. Clear CC commands from registry
  2. Re-run `discover_all_cc_commands()` and add to registry
  3. Re-poll all enabled plugins for their command lists
  4. Rebuild file index
  5. Return success message

In `command_svc.rs`, handle `ExecuteServiceCommand` for "reload-commands":
- Call executor's reload method
- Stream back a success/failure ServiceCommandOutput

**Step 2: Verify build**

Run: `cargo build -p betcode-daemon`
Expected: Clean compile

**Step 3: Commit**

```bash
git add crates/betcode-daemon/src/commands/service_executor.rs crates/betcode-daemon/src/server/command_svc.rs
git commit -m "feat(daemon): implement /reload-commands for full registry refresh"
```

---

### Task 26: End-to-End Integration Test

**Files:**
- Create: `crates/betcode-daemon/tests/command_integration.rs`

**Step 1: Write integration test**

```rust
//! Integration test for the command system.
//! Tests the full flow: daemon starts, CLI connects, fetches registry,
//! executes service commands, and receives responses.

use betcode_daemon::commands::CommandRegistry;
use betcode_daemon::completion::file_index::FileIndex;
use betcode_daemon::completion::agent_lister::AgentLister;
use betcode_daemon::server::command_svc::CommandServiceImpl;
use betcode_proto::betcode::v1::*;
use std::sync::Arc;
use tokio::sync::RwLock;
use tempfile::TempDir;

#[tokio::test]
async fn test_full_command_flow() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
    std::fs::create_dir(dir.path().join(".claude")).unwrap();
    std::fs::create_dir(dir.path().join(".claude/commands")).unwrap();
    std::fs::write(dir.path().join(".claude/commands/deploy.md"), "# Deploy").unwrap();

    let mut registry = CommandRegistry::new();
    // Simulate CC discovery
    let cc_cmds = betcode_core::commands::discovery::discover_user_commands(dir.path());
    for cmd in cc_cmds {
        registry.add(cmd);
    }

    let file_index = FileIndex::build(dir.path(), 1000).await.unwrap();
    let agent_lister = AgentLister::new();

    let service = CommandServiceImpl::new(
        Arc::new(RwLock::new(registry)),
        Arc::new(RwLock::new(file_index)),
        Arc::new(RwLock::new(agent_lister)),
    );

    // Test: registry contains builtins + user command
    let resp = service
        .get_command_registry(tonic::Request::new(GetCommandRegistryRequest {}))
        .await
        .unwrap();
    let commands = resp.into_inner().commands;
    assert!(commands.iter().any(|c| c.name == "cd"));
    assert!(commands.iter().any(|c| c.name == "deploy"));

    // Test: file path search works
    let resp = service
        .list_path(tonic::Request::new(ListPathRequest {
            query: "main".to_string(),
            max_results: 10,
        }))
        .await
        .unwrap();
    assert!(!resp.into_inner().entries.is_empty());
}
```

**Step 2: Run integration test**

Run: `cargo test -p betcode-daemon --test command_integration`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/betcode-daemon/tests/command_integration.rs
git commit -m "test(daemon): add end-to-end integration test for command system"
```

---

### Task 27: Final Verification & Cleanup

**Step 1: Run all tests**

Run: `cargo test --workspace`
Expected: All tests pass

**Step 2: Run clippy**

Run: `cargo clippy --workspace -- -D warnings`
Expected: No warnings

**Step 3: Run rustfmt**

Run: `cargo fmt --all`
Expected: No changes (or apply formatting)

**Step 4: Final commit**

```bash
git add -A
git commit -m "style: apply rustfmt and clippy fixes"
```

---

## Task Dependency Graph

```
Phase 1: Proto & Core Types
  Task 1 (proto) ──→ Task 10 (gRPC handler)
  Task 2 (core types) ──→ Task 3 (CC discovery)
                      ──→ Task 4 (registry)
                      ──→ Task 7 (matcher)

Phase 2: Daemon
  Task 3 (CC discovery) ──→ Task 6 (daemon CC discovery)
  Task 4 (registry) ──→ Task 10 (gRPC handler)
  Task 5 (executor) ──→ Task 10 (gRPC handler)

Phase 3: Completion Engine
  Task 7 (matcher) ──→ Task 8 (file index)
                   ──→ Task 9 (agent lister)

Phase 4: gRPC Handler
  Task 10 ──→ Task 17 (CLI gRPC client)

Phase 5: CLI UI
  Task 11 (controller) ──→ Task 14 (input integration)
  Task 12 (ghost) ──→ Task 14
  Task 13 (popup) ──→ Task 14

Phase 6: CLI Integration
  Task 16 (cache) ──→ Task 18 (async wiring)
  Task 17 (gRPC client) ──→ Task 18

Phase 7: Service Commands
  Task 19 ──→ Task 25 (reload wiring)

Phase 8: Plugins
  Task 20 (config) ──→ Task 21 (client) ──→ Task 22 (manager)

Phase 9: CLI Plugin
  Task 22 (manager) ──→ Task 23 (CLI subcommands)

Phase 10: Integration
  All above ──→ Task 24, 25, 26, 27
```

## Parallelization Opportunities

These task groups can be worked on simultaneously:

- **Group A**: Tasks 1-3 (proto + core types + discovery)
- **Group B**: Tasks 11-13 (CLI UI components — no daemon dependency)
- **Group C**: Tasks 20-21 (plugin config + client — independent of UI)

After Group A completes, Tasks 4-6 and 7-9 can run in parallel.
After Groups B and C complete, Task 14 and Tasks 22-23 can run in parallel.
