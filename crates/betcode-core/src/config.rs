//! Configuration resolution for BetCode.
//!
//! Implements hierarchical config resolution:
//! 1. Built-in defaults
//! 2. Global config (~/.config/betcode/settings.json)
//! 3. Project config (.betcode/settings.json)
//! 4. Environment variables
//! 5. CLI arguments (highest priority)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Complete BetCode configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub sessions: SessionConfig,
    #[serde(default)]
    pub permissions: PermissionConfig,
    #[serde(default)]
    pub feature_flags: std::collections::HashMap<String, bool>,
}

/// Daemon-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub max_subprocesses: u32,
    pub socket_path: Option<PathBuf>,
    pub port: u16,
    pub database_path: Option<PathBuf>,
    pub log_level: String,
    pub max_payload_bytes: usize,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            max_subprocesses: 5,
            socket_path: None,
            port: 50051,
            database_path: None,
            log_level: "info".to_string(),
            max_payload_bytes: 10 * 1024 * 1024, // 10 MB
        }
    }
}

/// Session default configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    pub default_model: String,
    pub auto_compact: bool,
    pub auto_compact_threshold: u32,
    pub max_messages_per_session: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            default_model: "claude-sonnet-4-20250514".to_string(),
            auto_compact: true,
            auto_compact_threshold: 180_000, // tokens
            max_messages_per_session: 10_000,
        }
    }
}

/// Permission system configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionConfig {
    /// Timeout for connected clients (seconds).
    pub connected_timeout_secs: u64,
    /// Timeout for disconnected clients (seconds). Default: 7 days.
    pub disconnected_timeout_secs: u64,
    /// Enable auto-approve for trusted directories.
    pub enable_auto_approve: bool,
    /// Directories where auto-approve is allowed.
    pub auto_approve_directories: Vec<PathBuf>,
    /// Enable activity-based TTL refresh.
    pub activity_refresh_enabled: bool,
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            connected_timeout_secs: 60,
            disconnected_timeout_secs: 7 * 24 * 60 * 60, // 7 days
            enable_auto_approve: false,
            auto_approve_directories: Vec::new(),
            activity_refresh_enabled: true,
        }
    }
}

/// Configuration source priority (lowest to highest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigSource {
    Default = 0,
    Global = 1,
    Project = 2,
    Environment = 3,
    Cli = 4,
}

/// Load configuration with hierarchical resolution.
pub fn load_config(project_dir: Option<&Path>) -> Result<Config> {
    let mut config = Config::default();

    // Load global config
    if let Some(global_path) = global_config_path() {
        if global_path.exists() {
            let global = load_config_file(&global_path)?;
            merge_config(&mut config, global);
        }
    }

    // Load project config
    if let Some(dir) = project_dir {
        let project_path = dir.join(".betcode").join("settings.json");
        if project_path.exists() {
            let project = load_config_file(&project_path)?;
            merge_config(&mut config, project);
        }
    }

    // Apply environment overrides
    apply_env_overrides(&mut config);

    Ok(config)
}

/// Get the global config directory path.
pub fn global_config_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .ok()
            .map(|h| PathBuf::from(h).join(".betcode").join("settings.json"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join("Library/Application Support/betcode/settings.json"))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config")))
            .map(|p| p.join("betcode").join("settings.json"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Get the database path for the daemon.
pub fn database_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .ok()
            .map(|h| PathBuf::from(h).join(".betcode").join("daemon.db"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join("Library/Application Support/betcode/daemon.db"))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config")))
            .map(|p| p.join("betcode").join("daemon.db"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

fn load_config_file(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        Error::Config(format!("Failed to read config file {}: {}", path.display(), e))
    })?;
    serde_json::from_str(&content).map_err(|e| {
        Error::Config(format!("Failed to parse config file {}: {}", path.display(), e))
    })
}

fn merge_config(base: &mut Config, overlay: Config) {
    // Merge daemon config
    if overlay.daemon.socket_path.is_some() {
        base.daemon.socket_path = overlay.daemon.socket_path;
    }
    if overlay.daemon.database_path.is_some() {
        base.daemon.database_path = overlay.daemon.database_path;
    }
    base.daemon.max_subprocesses = overlay.daemon.max_subprocesses;
    base.daemon.port = overlay.daemon.port;
    base.daemon.log_level = overlay.daemon.log_level;
    base.daemon.max_payload_bytes = overlay.daemon.max_payload_bytes;

    // Merge session config
    base.sessions = overlay.sessions;

    // Merge permission config
    base.permissions = overlay.permissions;

    // Merge feature flags
    base.feature_flags.extend(overlay.feature_flags);
}

fn apply_env_overrides(config: &mut Config) {
    if let Ok(val) = std::env::var("BETCODE_MAX_SUBPROCESSES") {
        if let Ok(n) = val.parse() {
            config.daemon.max_subprocesses = n;
        }
    }
    if let Ok(val) = std::env::var("BETCODE_PORT") {
        if let Ok(n) = val.parse() {
            config.daemon.port = n;
        }
    }
    if let Ok(val) = std::env::var("BETCODE_LOG_LEVEL") {
        config.daemon.log_level = val;
    }
    if let Ok(val) = std::env::var("BETCODE_DEFAULT_MODEL") {
        config.sessions.default_model = val;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_7_day_ttl() {
        let config = Config::default();
        assert_eq!(config.permissions.disconnected_timeout_secs, 7 * 24 * 60 * 60);
    }

    #[test]
    fn default_config_has_60s_connected_timeout() {
        let config = Config::default();
        assert_eq!(config.permissions.connected_timeout_secs, 60);
    }
}
