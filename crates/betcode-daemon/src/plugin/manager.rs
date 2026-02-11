use std::collections::HashMap;
use std::time::Duration;

use anyhow::{bail, Result};

use super::client::{PluginClient, PluginHealth, PluginStatus};

pub struct PluginSummary {
    pub name: String,
    pub enabled: bool,
    pub status: PluginStatus,
    pub command_count: usize,
}

pub struct PluginManager {
    plugins: HashMap<String, PluginClient>,
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn add_plugin(&mut self, name: &str, socket_path: &str, timeout: Duration) {
        let client = PluginClient {
            name: name.to_string(),
            socket_path: socket_path.to_string(),
            health: PluginHealth::new(3, 10),
            timeout,
            enabled: true,
        };
        self.plugins.insert(name.to_string(), client);
    }

    pub fn remove_plugin(&mut self, name: &str) {
        self.plugins.remove(name);
    }

    pub fn disable_plugin(&mut self, name: &str) -> Result<()> {
        match self.plugins.get_mut(name) {
            Some(client) => {
                client.enabled = false;
                Ok(())
            }
            None => bail!("plugin '{name}' not found"),
        }
    }

    pub fn enable_plugin(&mut self, name: &str) -> Result<()> {
        match self.plugins.get_mut(name) {
            Some(client) => {
                client.enabled = true;
                Ok(())
            }
            None => bail!("plugin '{name}' not found"),
        }
    }

    pub fn list_plugins(&self) -> Vec<PluginSummary> {
        self.plugins
            .values()
            .map(|client| PluginSummary {
                name: client.name.clone(),
                enabled: client.enabled,
                status: client.health.status(),
                command_count: 0,
            })
            .collect()
    }

    pub fn get_plugin_status(&self, name: &str) -> Option<&PluginClient> {
        self.plugins.get(name)
    }

    pub const fn get_all_plugin_commands(&self) -> Vec<String> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_plugin_manager_add_remove() {
        let mut manager = PluginManager::new();
        manager.add_plugin("test", "/tmp/test.sock", Duration::from_secs(30));
        let plugins = manager.list_plugins();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "test");
        manager.remove_plugin("test");
        assert!(manager.list_plugins().is_empty());
    }

    #[test]
    fn test_plugin_manager_enable_disable() {
        let mut manager = PluginManager::new();
        manager.add_plugin("test", "/tmp/test.sock", Duration::from_secs(30));
        manager.disable_plugin("test").unwrap();
        let plugins = manager.list_plugins();
        assert!(!plugins[0].enabled);
        manager.enable_plugin("test").unwrap();
        let plugins = manager.list_plugins();
        assert!(plugins[0].enabled);
    }

    #[test]
    fn test_plugin_manager_get_all_commands() {
        let manager = PluginManager::new();
        let commands = manager.get_all_plugin_commands();
        assert!(commands.is_empty());
    }
}
