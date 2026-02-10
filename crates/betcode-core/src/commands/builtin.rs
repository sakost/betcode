use super::{CommandCategory, CommandEntry, ExecutionMode};

/// Returns the list of built-in service commands.
pub fn builtin_commands() -> Vec<CommandEntry> {
    vec![
        CommandEntry::new(
            "cd",
            "Change working directory",
            CommandCategory::Service,
            ExecutionMode::Local,
            "built-in",
        ),
        CommandEntry::new(
            "pwd",
            "Print working directory",
            CommandCategory::Service,
            ExecutionMode::Local,
            "built-in",
        ),
        CommandEntry::new(
            "exit",
            "Exit the betcode session",
            CommandCategory::Service,
            ExecutionMode::Local,
            "built-in",
        ),
        CommandEntry::new(
            "exit-daemon",
            "Stop the betcode daemon",
            CommandCategory::Service,
            ExecutionMode::Local,
            "built-in",
        ),
        CommandEntry::new(
            "reload-commands",
            "Reload the command registry",
            CommandCategory::Service,
            ExecutionMode::Local,
            "built-in",
        ),
    ]
}
