pub mod cc_discovery;
pub mod service_executor;

use std::collections::HashMap;

use betcode_core::commands::{CommandEntry, builtin_commands};

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

    /// Return base entries merged with session-specific entries.
    pub fn get_for_session(&self, session_id: &str) -> Vec<CommandEntry> {
        self.base_entries
            .iter()
            .chain(self.session_layers.get(session_id).into_iter().flatten())
            .cloned()
            .collect()
    }

    /// Return a clone of all base entries (no session layer).
    pub fn get_all(&self) -> Vec<CommandEntry> {
        self.base_entries.clone()
    }

    /// Search for commands visible to a session whose name contains the query.
    pub fn search_for_session(
        &self,
        session_id: &str,
        query: &str,
        max_results: usize,
    ) -> Vec<CommandEntry> {
        let query_lower = query.to_lowercase();
        self.base_entries
            .iter()
            .chain(self.session_layers.get(session_id).into_iter().flatten())
            .filter(|e| e.name.to_lowercase().contains(&query_lower))
            .take(max_results)
            .cloned()
            .collect()
    }

    /// Search for commands in the base layer whose name contains the query.
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

    /// Replace the session layer for the given session ID.
    pub fn set_session_entries(&mut self, session_id: &str, entries: Vec<CommandEntry>) {
        self.session_layers.insert(session_id.to_string(), entries);
    }

    /// Update only the non-MCP entries in a session layer, preserving MCP tool entries.
    ///
    /// This is used by `reload-remote` to refresh plugins and skills without
    /// losing MCP tool entries that were set during `system_init`.
    pub fn update_session_plugin_entries(
        &mut self,
        session_id: &str,
        plugin_entries: Vec<CommandEntry>,
    ) {
        use betcode_core::commands::CommandCategory;

        let mut combined = self
            .session_layers
            .get(session_id)
            .map(|existing| {
                existing
                    .iter()
                    .filter(|e| e.category == CommandCategory::Mcp)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        combined.extend(plugin_entries);
        self.session_layers.insert(session_id.to_string(), combined);
    }

    /// Remove the session layer for the given session ID.
    pub fn remove_session(&mut self, session_id: &str) {
        self.session_layers.remove(session_id);
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use betcode_core::commands::{CommandCategory, CommandEntry, ExecutionMode};

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
        registry.clear_source("built-in");
        let after_clear = registry.get_all().len();
        assert_eq!(after_clear, 0);
    }

    // --- New layered model tests ---

    fn make_entry(name: &str, source: &str) -> CommandEntry {
        CommandEntry::new(
            name,
            &format!("{name} description"),
            CommandCategory::Mcp,
            ExecutionMode::Passthrough,
            source,
        )
    }

    #[test]
    fn get_for_session_returns_base_plus_session() {
        let mut registry = CommandRegistry::new();
        let base_count = registry.get_all().len();

        registry.set_session_entries("s1", vec![make_entry("mcp-tool-a", "mcp-server-a")]);

        let for_session = registry.get_for_session("s1");
        assert_eq!(for_session.len(), base_count + 1);
        assert!(for_session.iter().any(|e| e.name == "mcp-tool-a"));
        // Base entries still present
        assert!(for_session.iter().any(|e| e.name == "cd"));
    }

    #[test]
    fn get_for_session_unknown_session_returns_base_only() {
        let registry = CommandRegistry::new();
        let base = registry.get_all();
        let for_unknown = registry.get_for_session("nonexistent");
        assert_eq!(for_unknown.len(), base.len());
    }

    #[test]
    fn sessions_are_isolated() {
        let mut registry = CommandRegistry::new();

        registry.set_session_entries("s1", vec![make_entry("tool-for-s1", "mcp-a")]);
        registry.set_session_entries("s2", vec![make_entry("tool-for-s2", "mcp-b")]);

        let s1_entries = registry.get_for_session("s1");
        let s2_entries = registry.get_for_session("s2");

        assert!(s1_entries.iter().any(|e| e.name == "tool-for-s1"));
        assert!(!s1_entries.iter().any(|e| e.name == "tool-for-s2"));

        assert!(s2_entries.iter().any(|e| e.name == "tool-for-s2"));
        assert!(!s2_entries.iter().any(|e| e.name == "tool-for-s1"));
    }

    #[test]
    fn remove_session_cleans_up() {
        let mut registry = CommandRegistry::new();
        let base_count = registry.get_all().len();

        registry.set_session_entries("s1", vec![make_entry("tool-a", "mcp-a")]);
        assert_eq!(registry.get_for_session("s1").len(), base_count + 1);

        registry.remove_session("s1");
        assert_eq!(registry.get_for_session("s1").len(), base_count);
    }

    #[test]
    fn set_session_entries_replaces_previous() {
        let mut registry = CommandRegistry::new();

        registry.set_session_entries("s1", vec![make_entry("old-tool", "mcp-old")]);
        assert!(
            registry
                .get_for_session("s1")
                .iter()
                .any(|e| e.name == "old-tool")
        );

        registry.set_session_entries("s1", vec![make_entry("new-tool", "mcp-new")]);
        let entries = registry.get_for_session("s1");
        assert!(!entries.iter().any(|e| e.name == "old-tool"));
        assert!(entries.iter().any(|e| e.name == "new-tool"));
    }

    #[test]
    fn update_session_plugin_entries_preserves_mcp() {
        let mut registry = CommandRegistry::new();

        // Set initial session layer with MCP + plugin entries
        let mcp_entry = CommandEntry::new(
            "mcp-tool",
            "An MCP tool",
            CommandCategory::Mcp,
            ExecutionMode::Passthrough,
            "mcp-server",
        );
        let plugin_entry = CommandEntry::new(
            "old-plugin",
            "Old plugin",
            CommandCategory::Plugin,
            ExecutionMode::Passthrough,
            "my-plugin",
        );
        registry.set_session_entries("s1", vec![mcp_entry, plugin_entry]);

        // Update with new plugin entries — MCP should be preserved
        let new_plugin = CommandEntry::new(
            "new-plugin",
            "New plugin",
            CommandCategory::Plugin,
            ExecutionMode::Passthrough,
            "my-plugin",
        );
        registry.update_session_plugin_entries("s1", vec![new_plugin]);

        let entries = registry.get_for_session("s1");
        assert!(
            entries.iter().any(|e| e.name == "mcp-tool"),
            "MCP entry should be preserved"
        );
        assert!(
            entries.iter().any(|e| e.name == "new-plugin"),
            "New plugin should be present"
        );
        assert!(
            !entries.iter().any(|e| e.name == "old-plugin"),
            "Old plugin should be replaced"
        );
    }

    #[test]
    fn update_session_plugin_entries_works_with_no_existing_session() {
        let mut registry = CommandRegistry::new();
        let base_count = registry.get_all().len();

        let plugin = CommandEntry::new(
            "my-plugin",
            "A plugin",
            CommandCategory::Plugin,
            ExecutionMode::Passthrough,
            "plugin-src",
        );
        registry.update_session_plugin_entries("s1", vec![plugin]);

        let entries = registry.get_for_session("s1");
        assert_eq!(entries.len(), base_count + 1);
        assert!(entries.iter().any(|e| e.name == "my-plugin"));
    }

    #[test]
    fn remove_session_is_idempotent() {
        let mut registry = CommandRegistry::new();
        let base_count = registry.get_all().len();

        registry.set_session_entries("s1", vec![make_entry("tool-a", "src-a")]);
        registry.remove_session("s1");
        registry.remove_session("s1"); // second call is no-op

        assert_eq!(registry.get_for_session("s1").len(), base_count);
    }

    #[test]
    fn search_for_session_searches_both_layers() {
        let mut registry = CommandRegistry::new();

        registry.set_session_entries("s1", vec![make_entry("my-mcp-tool", "mcp-server")]);

        // Should find session "my-mcp-tool" matching "mcp"
        let results = registry.search_for_session("s1", "mcp", 10);
        assert!(results.iter().any(|e| e.name == "my-mcp-tool"));

        // Should find base "pwd"
        let results = registry.search_for_session("s1", "pwd", 10);
        assert!(results.iter().any(|e| e.name == "pwd"));

        // Session "s2" should NOT see s1's tools
        let results = registry.search_for_session("s2", "mcp", 10);
        assert!(!results.iter().any(|e| e.name == "my-mcp-tool"));
    }
}
