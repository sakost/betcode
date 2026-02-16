use std::path::Path;

use crate::config::DaemonSetupConfig;

/// Generate the systemd unit file for system-level deployment.
pub fn systemd_unit_system(config: &DaemonSetupConfig) -> String {
    format!(
        r"[Unit]
Description=BetCode Daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=notify
User={user}
Group={user}
EnvironmentFile=/etc/betcode/daemon.env
ExecStart=/usr/local/bin/betcode-daemon
Restart=on-failure
RestartSec=5

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/var/lib/betcode
ReadOnlyPaths=/etc/betcode
PrivateTmp=true

[Install]
WantedBy=multi-user.target
",
        user = config.user,
    )
}

/// Generate the systemd unit file for user-level deployment.
pub fn systemd_unit_user(binary_path: &Path) -> String {
    format!(
        r"[Unit]
Description=BetCode Daemon
After=default.target

[Service]
Type=notify
EnvironmentFile=%h/.config/betcode/daemon.env
ExecStart={binary}
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
",
        binary = binary_path.display(),
    )
}

/// Generate the environment file content with all daemon configuration.
pub fn env_file(config: &DaemonSetupConfig) -> String {
    let mut lines = Vec::new();

    lines.push(format!("BETCODE_ADDR={}", config.addr));
    lines.push(format!("BETCODE_DB_PATH={}", config.db_path.display()));
    lines.push(format!("BETCODE_MAX_PROCESSES={}", config.max_processes));
    lines.push(format!("BETCODE_MAX_SESSIONS={}", config.max_sessions));
    lines.push(format!("BETCODE_MACHINE_NAME={}", config.machine_name));
    lines.push("BETCODE_LOG_JSON=true".to_string());

    if let Some(ref url) = config.relay_url {
        lines.push(format!("BETCODE_RELAY_URL={url}"));
    }
    lines.push(format!("BETCODE_MACHINE_ID={}", config.machine_id));
    if let Some(ref user) = config.relay_username {
        lines.push(format!("BETCODE_RELAY_USERNAME={user}"));
    }
    if let Some(ref pass) = config.relay_password {
        lines.push(format!("BETCODE_RELAY_PASSWORD={pass}"));
    }
    if let Some(ref path) = config.relay_custom_ca_cert {
        lines.push(format!("BETCODE_RELAY_CUSTOM_CA_CERT={}", path.display()));
    }
    if let Some(ref dir) = config.worktree_dir {
        lines.push(format!("BETCODE_WORKTREE_DIR={}", dir.display()));
    }

    let mut result = lines.join("\n");
    result.push('\n');
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonMode;
    use std::path::PathBuf;

    use super::super::make_test_daemon_config;

    #[test]
    fn system_unit_contains_user() {
        let config = make_test_daemon_config(DaemonMode::System);
        let unit = systemd_unit_system(&config);
        assert!(unit.contains("User=betcode"));
        assert!(unit.contains("Group=betcode"));
    }

    #[test]
    fn system_unit_is_type_notify() {
        let config = make_test_daemon_config(DaemonMode::System);
        let unit = systemd_unit_system(&config);
        assert!(unit.contains("Type=notify"));
    }

    #[test]
    fn system_unit_has_hardening() {
        let config = make_test_daemon_config(DaemonMode::System);
        let unit = systemd_unit_system(&config);
        assert!(unit.contains("NoNewPrivileges=true"));
        assert!(unit.contains("ProtectSystem=strict"));
        assert!(unit.contains("ProtectHome=true"));
    }

    #[test]
    fn user_unit_is_type_notify() {
        let unit = systemd_unit_user(Path::new("/usr/local/bin/betcode-daemon"));
        assert!(unit.contains("Type=notify"));
    }

    #[test]
    fn user_unit_uses_home_specifier() {
        let unit = systemd_unit_user(Path::new("/usr/local/bin/betcode-daemon"));
        assert!(unit.contains("%h/.config/betcode/daemon.env"));
    }

    #[test]
    fn user_unit_has_no_user_directive() {
        let unit = systemd_unit_user(Path::new("/usr/local/bin/betcode-daemon"));
        assert!(!unit.contains("User="));
        assert!(!unit.contains("Group="));
    }

    #[test]
    fn user_unit_uses_absolute_binary_path() {
        let unit = systemd_unit_user(Path::new("/home/user/.cargo/bin/betcode-daemon"));
        assert!(unit.contains("ExecStart=/home/user/.cargo/bin/betcode-daemon"));
    }

    #[test]
    fn env_file_contains_required_keys() {
        let config = make_test_daemon_config(DaemonMode::System);
        let content = env_file(&config);
        assert!(content.contains("BETCODE_ADDR=127.0.0.1:50051"));
        assert!(content.contains("BETCODE_DB_PATH=/var/lib/betcode/daemon.db"));
        assert!(content.contains("BETCODE_MAX_PROCESSES=5"));
        assert!(content.contains("BETCODE_MAX_SESSIONS=10"));
        assert!(content.contains("BETCODE_LOG_JSON=true"));
        assert!(content.contains("BETCODE_MACHINE_ID=test-machine-id"));
    }

    #[test]
    fn env_file_omits_unset_relay_fields() {
        let config = make_test_daemon_config(DaemonMode::System);
        let content = env_file(&config);
        assert!(!content.contains("BETCODE_RELAY_URL"));
        assert!(!content.contains("BETCODE_RELAY_USERNAME"));
        assert!(!content.contains("BETCODE_RELAY_PASSWORD"));
    }

    #[test]
    fn env_file_includes_relay_fields_when_set() {
        let mut config = make_test_daemon_config(DaemonMode::System);
        config.relay_url = Some("https://relay.example.com".into());
        config.relay_username = Some("admin".into());
        config.relay_password = Some("secret123".into());
        config.relay_custom_ca_cert = Some(PathBuf::from("/etc/betcode/ca.pem"));
        let content = env_file(&config);
        assert!(content.contains("BETCODE_RELAY_URL=https://relay.example.com"));
        assert!(content.contains("BETCODE_RELAY_USERNAME=admin"));
        assert!(content.contains("BETCODE_RELAY_PASSWORD=secret123"));
        assert!(content.contains("BETCODE_RELAY_CUSTOM_CA_CERT=/etc/betcode/ca.pem"));
    }
}
