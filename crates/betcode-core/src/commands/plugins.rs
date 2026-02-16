//! Discovery of skills and commands from installed Claude Code plugins.

use std::path::Path;

use super::{CommandCategory, CommandEntry, ExecutionMode};

/// Discovers skills and commands from all enabled plugins.
///
/// Reads `{claude_dir}/plugins/installed_plugins.json` for install paths and
/// `{claude_dir}/settings.json` for the enabled/disabled filter.
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
        if skills_dir.is_dir()
            && let Ok(dirs) = std::fs::read_dir(&skills_dir)
        {
            for dir_entry in dirs.flatten() {
                let skill_path = dir_entry.path();
                if skill_path.is_dir()
                    && skill_path.join("SKILL.md").exists()
                    && let Some(skill_name) = skill_path.file_name().and_then(|n| n.to_str())
                {
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

        // Discover commands from commands/*.md
        let cmds_dir = install_path.join("commands");
        if cmds_dir.is_dir()
            && let Ok(files) = std::fs::read_dir(&cmds_dir)
        {
            for file_entry in files.flatten() {
                let file_path = file_entry.path();
                if file_path.extension().and_then(|e| e.to_str()) == Some("md")
                    && let Some(cmd_name) = file_path.file_stem().and_then(|n| n.to_str())
                {
                    let full_name = format!("{plugin_name}:{cmd_name}");
                    entries.push(
                        CommandEntry::new(
                            &full_name,
                            &format!("Plugin command: {cmd_name}"),
                            CommandCategory::Plugin,
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

    entries
}

/// Extract plugin name (before `@`) from a plugin ID like `"superpowers@superpowers-dev"`.
fn plugin_name_from_id(plugin_id: &str) -> String {
    plugin_id.split('@').next().unwrap_or(plugin_id).to_string()
}

/// Read enabled plugin IDs from `settings.json`.
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

/// Read installed plugin paths from `installed_plugins.json`.
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
        if let Some(first) = versions.as_array().and_then(|a| a.first())
            && let Some(path_str) = first.get("installPath").and_then(|v| v.as_str())
        {
            result.push((plugin_id.clone(), std::path::PathBuf::from(path_str)));
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[allow(clippy::unwrap_used)]
    fn setup_plugin_fixture(dir: &std::path::Path) {
        let settings = serde_json::json!({
            "enabledPlugins": {
                "superpowers@superpowers-dev": true,
                "disabled-plugin@some-marketplace": false
            }
        });
        fs::write(dir.join("settings.json"), settings.to_string()).unwrap();

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

        let skills_dir = superpowers_path.join("skills");
        fs::create_dir_all(skills_dir.join("brainstorming")).unwrap();
        fs::write(skills_dir.join("brainstorming/SKILL.md"), "# Brainstorm").unwrap();
        fs::create_dir_all(skills_dir.join("writing-plans")).unwrap();
        fs::write(skills_dir.join("writing-plans/SKILL.md"), "# Plans").unwrap();

        let cmds_dir = superpowers_path.join("commands");
        fs::create_dir_all(&cmds_dir).unwrap();
        fs::write(cmds_dir.join("brainstorm.md"), "# Brainstorm command").unwrap();

        let disabled_skills = disabled_path.join("skills/some-skill");
        fs::create_dir_all(&disabled_skills).unwrap();
        fs::write(disabled_skills.join("SKILL.md"), "# Disabled").unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_discovers_skills_from_enabled_plugins() {
        let dir = TempDir::new().unwrap();
        setup_plugin_fixture(dir.path());

        let entries = discover_plugin_entries(dir.path());
        let skill_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.category == CommandCategory::Skill)
            .collect();

        assert_eq!(skill_entries.len(), 2);
        assert!(
            skill_entries
                .iter()
                .any(|e| e.name == "superpowers:brainstorming")
        );
        assert!(
            skill_entries
                .iter()
                .any(|e| e.name == "superpowers:writing-plans")
        );
        assert!(
            skill_entries
                .iter()
                .all(|e| e.group.as_deref() == Some("superpowers"))
        );
        assert!(
            skill_entries
                .iter()
                .all(|e| e.source == "superpowers@superpowers-dev")
        );
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_discovers_commands_from_enabled_plugins() {
        let dir = TempDir::new().unwrap();
        setup_plugin_fixture(dir.path());

        let entries = discover_plugin_entries(dir.path());
        let cmd_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.category == CommandCategory::Plugin)
            .collect();

        assert_eq!(cmd_entries.len(), 1);
        assert_eq!(cmd_entries[0].name, "superpowers:brainstorm");
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_ignores_disabled_plugins() {
        let dir = TempDir::new().unwrap();
        setup_plugin_fixture(dir.path());

        let entries = discover_plugin_entries(dir.path());
        assert!(!entries.iter().any(|e| e.name.contains("disabled")));
        assert!(!entries.iter().any(|e| e.name.contains("some-skill")));
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_handles_missing_files_gracefully() {
        let dir = TempDir::new().unwrap();
        let entries = discover_plugin_entries(dir.path());
        assert!(entries.is_empty());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_handles_malformed_json_gracefully() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("settings.json"), "not valid json{{{").unwrap();
        let entries = discover_plugin_entries(dir.path());
        assert!(entries.is_empty());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_handles_missing_enabled_plugins_key() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("settings.json"), r#"{"other": "data"}"#).unwrap();
        let entries = discover_plugin_entries(dir.path());
        assert!(entries.is_empty());
    }
}
