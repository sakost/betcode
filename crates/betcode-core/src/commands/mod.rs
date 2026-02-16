pub mod builtin;
pub mod discovery;
pub mod matcher;
pub mod mcp;
pub mod plugins;

pub use builtin::builtin_commands;
pub use discovery::{
    discover_agents, discover_user_commands, hardcoded_cc_commands, parse_help_output,
};
pub use mcp::mcp_tools_to_entries;
pub use plugins::discover_plugin_entries;

/// Category of a command, determining its origin and handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandCategory {
    /// Built-in service commands handled by betcode itself.
    Service,
    /// Commands forwarded to Claude Code.
    ClaudeCode,
    /// Commands provided by plugins.
    Plugin,
    /// Skill commands (e.g. slash commands from skills).
    Skill,
    /// MCP tool commands.
    Mcp,
}

/// How a command is executed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Executed locally by betcode.
    Local,
    /// Passed through to Claude Code.
    Passthrough,
    /// Executed by a plugin.
    Plugin,
}

/// A single command entry in the command registry.
#[derive(Debug, Clone)]
pub struct CommandEntry {
    pub name: String,
    pub description: String,
    pub category: CommandCategory,
    pub execution_mode: ExecutionMode,
    pub source: String,
    pub args_schema: Option<String>,
    /// Logical group this command belongs to (e.g. MCP server name or skill namespace).
    pub group: Option<String>,
    /// Human-readable display name for the command.
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

    /// Set the logical group for this command.
    #[must_use]
    pub fn with_group(mut self, group: &str) -> Self {
        self.group = Some(group.to_string());
        self
    }

    /// Set the human-readable display name for this command.
    #[must_use]
    pub fn with_display_name(mut self, display_name: &str) -> Self {
        self.display_name = Some(display_name.to_string());
        self
    }
}

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

    #[test]
    fn test_builtin_commands_list() {
        let builtins = builtin_commands();
        let names: Vec<&str> = builtins.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"cd"));
        assert!(names.contains(&"pwd"));
        assert!(names.contains(&"exit"));
        assert!(names.contains(&"exit-daemon"));
        assert!(names.contains(&"reload-remote"));
    }
}
