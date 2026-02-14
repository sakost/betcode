use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use super::{CommandCategory, CommandEntry, ExecutionMode};

/// Pre-compiled regex for extracting slash-command names from help text.
#[allow(clippy::expect_used)]
static COMMAND_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/([a-zA-Z][\w-]*)").expect("static regex is valid"));

/// Returns a hardcoded list of known Claude Code slash commands.
pub fn hardcoded_cc_commands(version: &str) -> Vec<CommandEntry> {
    let _ = version; // reserved for future version-specific command sets
    let commands = [
        ("help", "Show help information"),
        ("clear", "Clear conversation history"),
        ("compact", "Compact conversation context"),
        ("exit", "Exit Claude Code"),
        ("config", "View or modify configuration"),
        ("model", "Switch AI model"),
        ("permissions", "Manage tool permissions"),
        ("status", "Show session status"),
        ("context", "Manage context files"),
        ("resume", "Resume a previous conversation"),
        ("memory", "Manage persistent memory"),
        ("doctor", "Diagnose configuration issues"),
        ("cost", "Show token usage and costs"),
        ("mcp", "Manage MCP servers"),
        ("hooks", "Manage hooks"),
        ("plugins", "Manage plugins"),
        ("fast", "Toggle fast mode"),
        ("vim", "Toggle vim keybindings"),
    ];

    commands
        .into_iter()
        .map(|(name, desc)| {
            CommandEntry::new(
                name,
                desc,
                CommandCategory::ClaudeCode,
                ExecutionMode::Passthrough,
                "claude-code",
            )
        })
        .collect()
}

/// Collects file stems (without extension) from all `.md` files in a directory.
///
/// Returns an empty `Vec` if the directory does not exist or is unreadable.
fn collect_md_stems(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|e| e.to_str()) == Some("md"))
                .then(|| path.file_stem()?.to_str().map(String::from))?
        })
        .collect()
}

/// Discovers user-defined commands from `.claude/commands/*.md` files.
pub fn discover_user_commands(working_dir: &Path) -> Vec<CommandEntry> {
    collect_md_stems(&working_dir.join(".claude").join("commands"))
        .into_iter()
        .map(|stem| {
            CommandEntry::new(
                &stem,
                &format!("User command: {stem}"),
                CommandCategory::ClaudeCode,
                ExecutionMode::Passthrough,
                "user",
            )
        })
        .collect()
}

/// Discovers agent names from `.claude/agents/*.md` files.
///
/// Each `.md` file stem is treated as an agent name.
pub fn discover_agents(working_dir: &Path) -> Vec<String> {
    collect_md_stems(&working_dir.join(".claude").join("agents"))
}

/// Parses Claude Code `/help` output, extracting command names.
///
/// Returns `(known, unknown)` where `known` are commands that exist in the
/// hardcoded list and `unknown` are newly discovered commands.
pub fn parse_help_output(
    help_text: &str,
    hardcoded: &[CommandEntry],
) -> (Vec<CommandEntry>, Vec<CommandEntry>) {
    let re = &*COMMAND_RE;

    let mut known = Vec::new();
    let mut unknown = Vec::new();

    let mut seen = std::collections::HashSet::new();

    for cap in re.captures_iter(help_text) {
        let name = &cap[1];
        if !seen.insert(name.to_string()) {
            continue;
        }
        if let Some(existing) = hardcoded.iter().find(|c| c.name == name) {
            known.push(existing.clone());
        } else {
            unknown.push(CommandEntry::new(
                name,
                &format!("Discovered command: {name}"),
                CommandCategory::ClaudeCode,
                ExecutionMode::Passthrough,
                "claude-code",
            ));
        }
    }

    (known, unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn test_discover_agents_from_directory() {
        let dir = TempDir::new().unwrap();
        let agents_dir = dir.path().join(".claude").join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        std::fs::write(agents_dir.join("researcher.md"), "# Researcher").unwrap();
        std::fs::write(agents_dir.join("code-reviewer.md"), "# Code Reviewer").unwrap();
        let agents = discover_agents(dir.path());
        assert_eq!(agents.len(), 2);
        assert!(agents.contains(&"researcher".to_string()));
        assert!(agents.contains(&"code-reviewer".to_string()));
    }

    #[test]
    fn test_discover_agents_missing_dir() {
        let dir = TempDir::new().unwrap();
        let agents = discover_agents(dir.path());
        assert!(agents.is_empty());
    }

    #[test]
    fn test_parse_help_output() {
        let help_text = r"
Usage: claude [options]

Commands:
  /help        Show help
  /clear       Clear conversation
  /compact     Compact conversation
  /unknown-new Some new command
        ";
        let hardcoded = hardcoded_cc_commands("1.0.0");
        let (known, unknown) = parse_help_output(help_text, &hardcoded);
        assert!(known.iter().any(|c| c.name == "help"));
        assert!(unknown.iter().any(|c| c.name == "unknown-new"));
    }
}
