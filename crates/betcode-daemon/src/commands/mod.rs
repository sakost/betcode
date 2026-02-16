pub mod cc_discovery;
pub mod service_executor;

use betcode_core::commands::{CommandCategory, CommandEntry, builtin_commands};

/// Registry holding all available commands.
pub struct CommandRegistry {
    entries: Vec<CommandEntry>,
}

impl CommandRegistry {
    /// Create a new registry initialized with built-in commands.
    pub fn new() -> Self {
        Self {
            entries: builtin_commands(),
        }
    }

    /// Add a command entry.
    pub fn add(&mut self, entry: CommandEntry) {
        self.entries.push(entry);
    }

    /// Return a clone of all entries.
    pub fn get_all(&self) -> Vec<CommandEntry> {
        self.entries.clone()
    }

    /// Search for commands whose name contains the query (case-insensitive substring match).
    pub fn search(&self, query: &str, max_results: usize) -> Vec<CommandEntry> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&query_lower))
            .take(max_results)
            .cloned()
            .collect()
    }

    /// Remove all entries with a matching source.
    pub fn clear_source(&mut self, source: &str) {
        self.entries.retain(|e| e.source != source);
    }

    /// Remove all entries originating from plugins.
    pub fn clear_plugin_sources(&mut self) {
        self.entries.retain(|e| {
            e.category != CommandCategory::Skill && e.category != CommandCategory::Plugin
        });
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
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

    #[test]
    fn test_clear_plugin_sources_removes_skill_and_plugin_entries() {
        let mut registry = CommandRegistry::new();

        // Add entries with various categories
        registry.add(CommandEntry::new(
            "my-skill",
            "A skill entry",
            CommandCategory::Skill,
            ExecutionMode::Passthrough,
            "plugin@marketplace",
        ));
        registry.add(CommandEntry::new(
            "my-plugin-cmd",
            "A plugin command",
            CommandCategory::Plugin,
            ExecutionMode::Passthrough,
            "plugin@marketplace",
        ));
        registry.add(CommandEntry::new(
            "my-cc-cmd",
            "A Claude Code command",
            CommandCategory::ClaudeCode,
            ExecutionMode::Passthrough,
            "claude-code",
        ));
        registry.add(CommandEntry::new(
            "my-service-cmd",
            "A service command",
            CommandCategory::Service,
            ExecutionMode::Local,
            "built-in",
        ));

        let before = registry.get_all().len();
        registry.clear_plugin_sources();
        let after = registry.get_all();

        // Skill and Plugin entries should be removed
        assert_eq!(after.len(), before - 2);
        assert!(!after.iter().any(|e| e.name == "my-skill"));
        assert!(!after.iter().any(|e| e.name == "my-plugin-cmd"));

        // ClaudeCode and Service entries should remain
        assert!(after.iter().any(|e| e.name == "my-cc-cmd"));
        assert!(after.iter().any(|e| e.name == "my-service-cmd"));
    }
}
