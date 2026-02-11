use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDeclaration {
    pub name: String,
    pub socket: String,
    pub enabled: bool,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginConfig {
    #[serde(default)]
    pub plugins: Vec<PluginDeclaration>,
}

impl PluginConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).unwrap_or_default();
        if content.trim().is_empty() {
            return Ok(Self::default());
        }
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn add_plugin(&mut self, name: &str, socket: &str) {
        self.plugins.push(PluginDeclaration {
            name: name.to_string(),
            socket: socket.to_string(),
            enabled: true,
            timeout_secs: 30,
        });
    }

    pub fn remove_plugin(&mut self, name: &str) {
        self.plugins.retain(|p| p.name != name);
    }

    pub fn enable_plugin(&mut self, name: &str) {
        if let Some(p) = self.plugins.iter_mut().find(|p| p.name == name) {
            p.enabled = true;
        }
    }

    pub fn disable_plugin(&mut self, name: &str) {
        if let Some(p) = self.plugins.iter_mut().find(|p| p.name == name) {
            p.enabled = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_plugin_config() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("daemon.toml");
        std::fs::write(
            &config_path,
            r#"
[[plugins]]
name = "test-plugin"
socket = "/tmp/test.sock"
enabled = true
timeout_secs = 30
"#,
        )
        .unwrap();
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
        std::fs::write(
            &config_path,
            r#"
[[plugins]]
name = "test-plugin"
socket = "/tmp/test.sock"
enabled = true
timeout_secs = 30
"#,
        )
        .unwrap();
        let mut config = PluginConfig::load(&config_path).unwrap();
        config.remove_plugin("test-plugin");
        config.save(&config_path).unwrap();
        let reloaded = PluginConfig::load(&config_path).unwrap();
        assert!(reloaded.plugins.is_empty());
    }
}
