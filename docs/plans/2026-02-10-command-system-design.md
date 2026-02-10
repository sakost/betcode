# Command System & Autocomplete Design

**Date**: 2026-02-10
**Branch**: `feature/command-system`
**Status**: Design approved, ready for implementation

## Overview

A two-layer system providing a daemon-side command registry with plugin support and a CLI-side autocomplete UI with ghost text and floating overlay. Commands, agents, files, and bash execution are all supported through unified completion triggers.

## Architecture

### Three Layers

1. **Registry (daemon)** — Holds all known commands from three sources: built-in, Claude Code, and external plugins.
2. **Cache (CLI)** — Fetches full command registry on session connect, caches locally. Dynamic data (agents, files) queried on-demand.
3. **UI (CLI)** — Ghost text inline + floating overlay popup in the ratatui TUI.

### Three Execution Lanes (never blocking each other)

1. **Claude Code lane** — The main NDJSON stream between daemon and Claude Code subprocess. User messages go here unless intercepted.
2. **Service command lane** — A separate async task in the daemon. `/cd`, `/pwd`, `!<cmd>` etc. execute here. Results sent to CLI via the same gRPC stream but tagged as `ServiceResponse` messages.
3. **Completion lane** — `GetCommandRegistry`, `ListAgents`, `ListPath` RPCs. Stateless request/response.

## Commands

### Service Commands (BetCode-native, immediate execution)

| Command | Scope | Description |
|---------|-------|-------------|
| `/cd <path>` | daemon | Change session working directory |
| `/pwd` | daemon | Print current working directory |
| `/exit` | CLI-only | Disconnect CLI, daemon keeps running |
| `/exit-daemon` | daemon | Graceful daemon shutdown, all sessions terminate |
| `/reload-commands` | daemon | Re-discover CC commands, re-poll plugins, rebuild file index |
| `!<cmd>` | daemon | Execute shell command, stream output to chat |

### Plugin Management (CLI subcommands)

| Command | Description |
|---------|-------------|
| `betcode plugin add <name> <socket>` | Declare plugin in config, connect live |
| `betcode plugin remove <name>` | Remove declaration, disconnect |
| `betcode plugin list` | Show all plugins with status |
| `betcode plugin status <name>` | Detailed health info |
| `betcode plugin enable <name>` | Re-enable suspended plugin |
| `betcode plugin disable <name>` | Temporarily suspend |

## Completion System

### Triggers

| Trigger | Behavior |
|---------|----------|
| `/` | Show commands from cache (BetCode + Claude Code + plugins) |
| `@` | Context-aware: path-like text -> file mode, otherwise -> agent mode |
| `@/` or `@./` | Force file path mode |
| `@@` | Force agent mode |
| `Tab` (on any text) | Toggle floating overlay for general fuzzy completion |

### `@` Context-Aware Disambiguation

Auto-detection rules (in priority order):

1. `@/` or `@./` or `@../` -> immediately enters **file path mode** (explicit sub-trigger)
2. `@<text containing / or .ext>` (e.g., `@src/main`, `@README.md`) -> **file path mode** (detected path-like pattern)
3. `@<text>` with no path characters -> **agent mode** (fuzzy match against agent names)
4. **Mixed/ambiguous**: show both categories in popup -- agents on top, files below, separated by divider. Fuzzy scorer runs across both pools.

### Rendering

**Layer 1 -- Ghost text (always active after trigger):**

The top-ranked completion candidate appears as dimmed/gray text appended to cursor position. Updates as user types. Lightweight -- single styled span in the input line widget.

**Layer 2 -- Floating overlay (on Tab toggle):**

A floating panel **above** the input line showing N visible items (configurable, default 8) in a scrollable list:

- Each item shows: icon/category badge, name, brief description
- Currently selected item is highlighted
- Fuzzy match characters are bolded/colored
- **Virtualized rendering**: only visible N items are rendered per frame. Full candidate list in `Vec`, only a window slice drawn. No UI thread overload.

**Category indicators:**

- `/` commands: `[bc]` for BetCode, `[cc]` for Claude Code, `[plugin-name]` for plugins
- `@` agents: colored status dot (green=idle, yellow=working, gray=done, red=failed)
- `@` files: file/folder icon
- `[cc?]` for Claude Code commands discovered only via `--help` parse (unknown to hardcoded list)

### Keybindings

**Completion keybindings:**

| Key | Action |
|-----|--------|
| `Tab` | Toggle overlay popup open/closed |
| `Up/Down` | Navigate overlay items (ghost text follows selection) |
| `Enter` or `Space` | Accept highlighted completion |
| `Escape` | Dismiss overlay, keep typed text |
| Continue typing | Filter results, ghost text updates to top match |

**Special keybindings:**

| Key | Action |
|-----|--------|
| `Ctrl+I` | Session status panel |
| `Ctrl+C` (during `!cmd`) | Cancel running bash command, not Claude session |

### Session Status Panel (Ctrl+I)

Ephemeral overlay, dismisses on any keypress. Shows:

- Current working directory
- Session ID
- Connection: local / remote (relay address)
- Active model
- Active agents/subagents count
- Pending permission requests
- Current worktree info (branch, path)
- Session uptime

## Data Flow & Caching

### Command Registry (cached)

On session connect, CLI performs a single `GetCommandRegistry` gRPC call. Daemon responds with full command list. CLI caches in memory. Refreshed only on `/reload-commands`.

### Dynamic Data (on-demand)

| Category | RPC | When |
|----------|-----|------|
| Agents | `ListAgents(query, max_results)` | `@` trigger, agent mode |
| File paths | `ListPath(query, max_results)` | `@` trigger file mode, or Tab on path-like text |

Both RPCs accept a query string. **Daemon does fuzzy matching server-side**, returns only top-N ranked results. CLI never receives more items than it can display. Debounced at ~100ms on keystroke.

## File Indexing

### Daemon-side Watched Index

On session start, daemon spawns a background task building an in-memory file index:

1. **Git-aware**: starts with `git ls-files` for tracked files, adds untracked non-ignored via `git ls-files --others --exclude-standard`.
2. **Watched**: uses `notify` crate (`inotify` on Linux, `FSEvents` on macOS, `ReadDirectoryChangesW` on Windows) for incremental updates on create/delete/rename.
3. **Trie/radix tree structure**: fast prefix and fuzzy matching without full scan.
4. **Bounded**: configurable max entries (default 100k files). Beyond that, falls back to on-demand directory listing.
5. **Refreshed** on `/reload-commands`.

## Claude Code Command Discovery

Layered approach with fallback and warnings:

### Layer 1: Hardcoded Versioned List

A `HashMap<VersionRange, Vec<Command>>` mapping Claude Code version ranges to known built-in slash commands. Updated when BetCode is updated. Version detected via `claude --version` at session start.

### Layer 2: Filesystem Scan

Read `.claude/commands/*.md` from the working directory. Each filename (minus `.md`) becomes a user-defined command.

### Layer 3: Parse `claude --help`

Supplementary check. Parse help output and cross-reference with hardcoded list:

- Commands in both: registered normally with `[cc]` badge.
- Commands in `--help` but NOT in hardcoded list: registered with `[cc?]` badge. Warning logged: `"Discovered unknown Claude Code command '/foo' via --help. Consider updating BetCode's command list."` Warning suppressible in config.
- Commands in hardcoded list but NOT in `--help`: still registered (help output may not list everything).

## Plugin System

### Design Principles

1. **Declared, not auto-discovered**: All plugins declared in `~/.config/betcode/daemon.toml`.
2. **Daemon robustness is first priority**: Plugin failures never crash the daemon.
3. **gRPC over Unix socket** (named pipe on Windows).

### Plugin Config

```toml
[[plugins]]
name = "my-plugin"
socket = "/path/to/my-plugin.sock"
enabled = true
timeout_secs = 30
```

### Plugin Lifecycle

1. **Registration**: Daemon connects to plugin socket as gRPC client. Plugin responds with `RegisterResponse` containing command definitions (name, description, args schema).
2. **Health check**: Periodic ping (configurable, default 30s). 2s timeout.
3. **Execution**: Daemon sends `ExecuteCommand(name, args)`. Plugin streams back output lines + exit status.
4. **Deregistration**: Plugin disconnects or daemon shuts down. Commands removed from registry, CLI cache invalidated.

### Robustness

1. **Isolation**: Each plugin connection in its own `tokio::spawn` task with error boundary.
2. **Timeouts**: Registration 5s, execution configurable (default 30s), health check 2s.
3. **Circuit breaker**: 3 consecutive failures -> `degraded` (commands grayed out in autocomplete). 10 failures -> `unavailable` (no requests until re-enabled or restart).
4. **Graceful degradation**: Plugin crash mid-execution -> error system message to CLI. Daemon continues normally.
5. **Resource limits**: Plugin output capped (default 1MB). Socket connections rate-limited.

### Plugin Proto (what plugins implement)

```protobuf
service PluginService {
  rpc Register(RegisterRequest) returns (RegisterResponse);
  rpc Execute(ExecuteRequest) returns (stream ExecuteResponse);
  rpc HealthCheck(HealthCheckRequest) returns (HealthCheckResponse);
}
```

## Agent Completion

### Data Model

```
AgentInfo {
    name: String,
    kind: AgentKind,     // ClaudeInternal | DaemonOrchestrated | TeamMember
    status: AgentStatus, // Idle | Working | Done | Failed
    session_id: Option<String>,
}
```

- **ClaudeInternal**: tracked via `parent_tool_use_id` in NDJSON stream.
- **DaemonOrchestrated**: tracked via subagent session table (Phase 2).
- **TeamMember**: tracked via team config when `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS` is enabled.

### Completion Display

Popup shows: `[status dot] agent-name [kind badge]`

- Green dot = idle, yellow = working, gray = done, red = failed
- Kind badge: `[cc]` for Claude-internal, `[bc]` for daemon-orchestrated, `[team]` for team members

## Bash Command Execution (`!` prefix)

- Daemon spawns shell command as subprocess in session's current working directory.
- Stdout/stderr streamed to CLI via gRPC `ServiceCommandOutput`.
- Output appears as system message block with distinct styling (monospace background).
- Runs asynchronously -- does NOT block Claude Code lane.
- Status bar shows spinner during execution.
- `Ctrl+C` cancels the bash command only.

## gRPC Proto Definitions

### CommandService (`proto/betcode/v1/commands.proto`)

```protobuf
service CommandService {
  // Registry
  rpc GetCommandRegistry(GetCommandRegistryRequest) returns (GetCommandRegistryResponse);

  // Completion
  rpc ListAgents(ListAgentsRequest) returns (ListAgentsResponse);
  rpc ListPath(ListPathRequest) returns (ListPathResponse);

  // Service command execution
  rpc ExecuteServiceCommand(ExecuteServiceCommandRequest) returns (stream ServiceCommandOutput);

  // Plugin management
  rpc ListPlugins(ListPluginsRequest) returns (ListPluginsResponse);
  rpc GetPluginStatus(GetPluginStatusRequest) returns (GetPluginStatusResponse);
  rpc AddPlugin(AddPluginRequest) returns (AddPluginResponse);
  rpc RemovePlugin(RemovePluginRequest) returns (RemovePluginResponse);
  rpc EnablePlugin(EnablePluginRequest) returns (EnablePluginResponse);
  rpc DisablePlugin(DisablePluginRequest) returns (DisablePluginResponse);
}

message CommandEntry {
  string name = 1;
  string description = 2;
  CommandCategory category = 3;
  ExecutionMode execution_mode = 4;
  string source = 5;
  optional string args_schema = 6;
}

enum CommandCategory {
  SERVICE = 0;
  CLAUDE_CODE = 1;
  PLUGIN = 2;
}

enum ExecutionMode {
  LOCAL = 0;
  PASSTHROUGH = 1;
  PLUGIN = 2;
}

message AgentInfo {
  string name = 1;
  AgentKind kind = 2;
  AgentStatus status = 3;
  optional string session_id = 4;
}

enum AgentKind {
  CLAUDE_INTERNAL = 0;
  DAEMON_ORCHESTRATED = 1;
  TEAM_MEMBER = 2;
}

enum AgentStatus {
  IDLE = 0;
  WORKING = 1;
  DONE = 2;
  FAILED = 3;
}

message PathEntry {
  string path = 1;
  PathKind kind = 2;
  uint64 size = 3;
  int64 modified_at = 4;
}

enum PathKind {
  FILE = 0;
  DIRECTORY = 1;
  SYMLINK = 2;
}

message ServiceCommandOutput {
  oneof output {
    string stdout_line = 1;
    string stderr_line = 2;
    int32 exit_code = 3;
    string error = 4;
  }
}
```

### PluginService (`proto/betcode/v1/plugin.proto`)

```protobuf
service PluginService {
  rpc Register(RegisterRequest) returns (RegisterResponse);
  rpc Execute(ExecuteRequest) returns (stream ExecuteResponse);
  rpc HealthCheck(HealthCheckRequest) returns (HealthCheckResponse);
}
```

## Module Layout

```
crates/
  betcode-core/src/
    commands/
      mod.rs              # Command trait, CommandEntry, registry types
      builtin.rs          # Built-in service commands (cd, pwd, exit, etc.)
      discovery.rs        # CC command discovery (hardcoded + fs + help parse)

  betcode-daemon/src/
    commands/
      mod.rs              # CommandRegistry (holds all sources, serves RPCs)
      service_executor.rs # Executes service commands (cd, pwd, bash)
      cc_discovery.rs     # Spawns `claude --help`, parses, cross-references
    completion/
      mod.rs              # Completion engine (fuzzy matching, scoring)
      file_index.rs       # Watched file index (notify crate, trie)
      agent_lister.rs     # Queries session manager for active agents
    plugin/
      mod.rs              # Plugin manager (lifecycle, circuit breaker)
      client.rs           # gRPC client to plugin Unix sockets
      config.rs           # Plugin config persistence (daemon.toml)
    grpc/
      command_service.rs  # CommandService gRPC handler

  betcode-cli/src/
    commands/
      mod.rs              # CLI-side command cache + local commands (/exit)
      plugin_cmd.rs       # `betcode plugin add/remove/list/...` clap subcommands
    completion/
      mod.rs              # Completion controller (trigger detection, debounce)
      ghost.rs            # Ghost text renderer (inline dim text)
      popup.rs            # Floating overlay widget (virtualized list)
      matcher.rs          # Client-side fuzzy scorer (for cached commands)
    ui/
      status_panel.rs     # Ctrl+I session status overlay

  betcode-proto/
    proto/betcode/v1/
      commands.proto      # CommandService definition
      plugin.proto        # PluginService definition

```

## New Dependencies

- `notify` — file system watching (daemon only)
- `nucleo` or `fuzzy-matcher` — fzf-like fuzzy scoring (daemon + CLI)
- No new proto dependencies (already using tonic + prost)

## Service Command Feedback

All service commands produce dual feedback:

1. **Status bar toast** — brief ephemeral message at the bottom (e.g., "Changed to /home/user/project")
2. **Chat system message** — persisted in chat history for audit (e.g., "[system] Working directory changed to /home/user/project")

## Open Questions for Implementation

1. Exact fuzzy scoring algorithm choice (`nucleo` vs `fuzzy-matcher` vs custom) -- benchmark during implementation.
2. File index memory budget for very large repos (monorepos with >100k files) -- may need configurable depth limit.
3. Claude Code `--help` output format stability -- may need version-specific parsers.
