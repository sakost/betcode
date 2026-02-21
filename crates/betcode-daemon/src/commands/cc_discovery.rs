use std::collections::HashSet;
use std::path::Path;

use regex::Regex;
use tracing::warn;

use betcode_core::commands::{
    CommandEntry, discover_user_commands, hardcoded_cc_commands, parse_help_output,
};

/// Result of a full command discovery pass.
pub struct DiscoveryResult {
    pub commands: Vec<CommandEntry>,
    pub warnings: Vec<String>,
}

/// Runs `claude --version` and returns the version string.
pub async fn detect_cc_version(claude_bin: &std::path::Path) -> anyhow::Result<String> {
    let output = tokio::process::Command::new(claude_bin)
        .arg("--version")
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_version(&stdout).ok_or_else(|| anyhow::anyhow!("Failed to parse Claude Code version"))
}

/// Extracts a version number (e.g. "1.0.22") from Claude Code version output.
#[allow(clippy::missing_panics_doc, clippy::expect_used)]
pub fn parse_version(output: &str) -> Option<String> {
    let re = Regex::new(r"v(\d+\.\d+\.\d+)").expect("static regex pattern is valid");
    re.captures(output).map(|cap| cap[1].to_string())
}

/// Discovers all Claude Code commands by merging hardcoded, user, and help-parsed commands.
pub fn discover_all_cc_commands(working_dir: &Path, help_output: Option<&str>) -> DiscoveryResult {
    let version = "unknown".to_string();
    let mut warnings = Vec::new();

    // Get hardcoded commands
    let hardcoded = hardcoded_cc_commands(&version);

    // Get user-defined commands
    let user_commands = discover_user_commands(working_dir);

    // Merge all commands, starting with hardcoded
    let mut all_commands = hardcoded.clone();

    // If help output provided, parse and cross-reference
    if let Some(help_text) = help_output {
        let (_known, unknown) = parse_help_output(help_text, &hardcoded);
        for cmd in &unknown {
            warn!(name = %cmd.name, "Discovered unknown command from help output");
            warnings.push(format!("Unknown command from help: {}", cmd.name));
        }
        all_commands.extend(unknown);
    }

    // Add user commands
    all_commands.extend(user_commands);

    // Deduplicate by name
    let mut seen = HashSet::new();
    all_commands.retain(|e| seen.insert(e.name.clone()));

    DiscoveryResult {
        commands: all_commands,
        warnings,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_version_string() {
        assert_eq!(
            parse_version("claude v1.0.22 (anthropic-2024-12-01)"),
            Some("1.0.22".to_string())
        );
        assert_eq!(parse_version("claude v2.1.0"), Some("2.1.0".to_string()));
        assert_eq!(parse_version("unexpected output"), None);
    }

    #[tokio::test]
    async fn test_full_discovery_with_mock_dir() {
        let dir = TempDir::new().unwrap();
        let commands_dir = dir.path().join(".claude").join("commands");
        std::fs::create_dir_all(&commands_dir).unwrap();
        std::fs::write(commands_dir.join("my-cmd.md"), "# Custom").unwrap();

        let result = discover_all_cc_commands(dir.path(), None);
        assert!(result.commands.iter().any(|c| c.name == "my-cmd"));
        assert!(result.commands.iter().any(|c| c.name == "help"));
    }
}
