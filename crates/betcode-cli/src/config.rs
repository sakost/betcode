//! CLI configuration management.
//!
//! Persists relay URL, auth tokens, and active machine to `~/.betcode/config.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Persistent CLI configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CliConfig {
    /// Relay server URL (e.g., "<https://relay.betcode.io:443>").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
    /// Currently active machine ID for relay routing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_machine: Option<String>,
    /// Authentication credentials.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthConfig>,
    /// Path to custom CA certificate for verifying the relay's TLS certificate.
    /// Use this for self-signed or development certificates.
    #[serde(skip_serializing_if = "Option::is_none", alias = "relay_ca_cert")]
    pub relay_custom_ca_cert: Option<PathBuf>,
}

/// Stored authentication credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub user_id: String,
    pub username: String,
    pub access_token: String,
    pub refresh_token: String,
}

impl CliConfig {
    /// Path to the config directory: `~/.betcode/`.
    pub fn config_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".betcode"))
    }

    /// Path to the config file: `~/.betcode/config.json`.
    pub fn config_path() -> Option<PathBuf> {
        Self::config_dir().map(|d| d.join("config.json"))
    }

    /// Load config from disk. Returns default if file doesn't exist or is invalid.
    pub fn load() -> Self {
        Self::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Save config to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        let dir =
            Self::config_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("config.json");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Clear stored auth credentials.
    pub fn clear_auth(&mut self) {
        self.auth = None;
    }

    /// Whether this config has all fields needed for relay mode.
    pub const fn is_relay_mode(&self) -> bool {
        self.relay_url.is_some() && self.auth.is_some() && self.active_machine.is_some()
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_local() {
        let cfg = CliConfig::default();
        assert!(!cfg.is_relay_mode());
        assert!(cfg.active_machine.is_none());
        assert!(cfg.auth.is_none());
    }

    #[test]
    fn config_roundtrip_json() {
        let cfg = CliConfig {
            relay_url: Some("https://relay.test:443".into()),
            active_machine: Some("m1".into()),
            auth: Some(AuthConfig {
                user_id: "u1".into(),
                username: "alice".into(),
                access_token: "at".into(),
                refresh_token: "rt".into(),
            }),
            relay_custom_ca_cert: None,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let loaded: CliConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.relay_url.unwrap(), "https://relay.test:443");
        assert_eq!(loaded.active_machine.unwrap(), "m1");
        assert_eq!(loaded.auth.unwrap().username, "alice");
    }

    #[test]
    fn clear_auth_removes_credentials() {
        let mut cfg = CliConfig {
            auth: Some(AuthConfig {
                user_id: "u1".into(),
                username: "alice".into(),
                access_token: "at".into(),
                refresh_token: "rt".into(),
            }),
            ..Default::default()
        };
        cfg.clear_auth();
        assert!(cfg.auth.is_none());
    }

    #[test]
    fn config_path_contains_betcode() {
        if let Some(path) = CliConfig::config_path() {
            assert!(path.to_string_lossy().contains(".betcode"));
            assert!(path.to_string_lossy().contains("config.json"));
        }
    }

    #[test]
    fn default_config_has_no_ca_cert() {
        let cfg = CliConfig::default();
        assert!(cfg.relay_custom_ca_cert.is_none());
    }

    #[test]
    fn config_roundtrip_json_with_ca_cert() {
        let cfg = CliConfig {
            relay_url: Some("https://relay.test:443".into()),
            active_machine: Some("m1".into()),
            auth: None,
            relay_custom_ca_cert: Some(PathBuf::from("/path/to/ca.pem")),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let loaded: CliConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            loaded.relay_custom_ca_cert.as_deref(),
            Some(std::path::Path::new("/path/to/ca.pem"))
        );
    }

    #[test]
    fn config_roundtrip_json_without_ca_cert_omits_field() {
        let cfg = CliConfig {
            relay_url: Some("https://relay.test:443".into()),
            relay_custom_ca_cert: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        // Field should be skipped when None
        assert!(
            !json.contains("relay_custom_ca_cert"),
            "relay_custom_ca_cert should be omitted from JSON when None, got: {json}",
        );
        let loaded: CliConfig = serde_json::from_str(&json).unwrap();
        assert!(loaded.relay_custom_ca_cert.is_none());
    }
}
