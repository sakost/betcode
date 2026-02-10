//! Plugin management CLI subcommands.
//!
//! Provides add/remove/list/status/enable/disable actions for managing
//! betcode plugins (MCP-compatible command providers).

/// Plugin subcommand actions.
#[derive(clap::Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum PluginAction {
    /// Register a new plugin by name and Unix socket path.
    Add {
        /// Plugin name (unique identifier).
        name: String,
        /// Path to the plugin's Unix socket.
        socket: String,
    },
    /// Remove a registered plugin.
    Remove {
        /// Plugin name to remove.
        name: String,
    },
    /// List all registered plugins.
    List,
    /// Show status of a specific plugin.
    Status {
        /// Plugin name to query.
        name: String,
    },
    /// Enable a disabled plugin.
    Enable {
        /// Plugin name to enable.
        name: String,
    },
    /// Disable a plugin without removing it.
    Disable {
        /// Plugin name to disable.
        name: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Wrapper for testing subcommand parsing.
    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(subcommand)]
        action: PluginAction,
    }

    #[test]
    fn test_parse_add() {
        let cli =
            TestCli::try_parse_from(["test", "add", "my-plugin", "/tmp/plugin.sock"]).unwrap();
        assert_eq!(
            cli.action,
            PluginAction::Add {
                name: "my-plugin".to_string(),
                socket: "/tmp/plugin.sock".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_remove() {
        let cli = TestCli::try_parse_from(["test", "remove", "my-plugin"]).unwrap();
        assert_eq!(
            cli.action,
            PluginAction::Remove {
                name: "my-plugin".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_list() {
        let cli = TestCli::try_parse_from(["test", "list"]).unwrap();
        assert_eq!(cli.action, PluginAction::List);
    }

    #[test]
    fn test_parse_status() {
        let cli = TestCli::try_parse_from(["test", "status", "my-plugin"]).unwrap();
        assert_eq!(
            cli.action,
            PluginAction::Status {
                name: "my-plugin".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_enable() {
        let cli = TestCli::try_parse_from(["test", "enable", "my-plugin"]).unwrap();
        assert_eq!(
            cli.action,
            PluginAction::Enable {
                name: "my-plugin".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_disable() {
        let cli = TestCli::try_parse_from(["test", "disable", "my-plugin"]).unwrap();
        assert_eq!(
            cli.action,
            PluginAction::Disable {
                name: "my-plugin".to_string(),
            }
        );
    }

    #[test]
    fn test_add_missing_socket_fails() {
        let result = TestCli::try_parse_from(["test", "add", "my-plugin"]);
        assert!(result.is_err());
    }
}
