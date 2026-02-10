pub mod builtin;

pub use builtin::builtin_commands;

/// Category of a command, determining its origin and handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandCategory {
    /// Built-in service commands handled by betcode itself.
    Service,
    /// Commands forwarded to Claude Code.
    ClaudeCode,
    /// Commands provided by plugins.
    Plugin,
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
        }
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
