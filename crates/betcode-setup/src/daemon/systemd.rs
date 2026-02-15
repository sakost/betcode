use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::cmd::{create_setup_directories, ensure_system_user, run_cmd, write_env_if_fresh};
use crate::config::DaemonSetupConfig;

use super::templates;

/// Path to the system-level systemd unit file.
pub const SYSTEM_UNIT_PATH: &str = "/etc/systemd/system/betcode-daemon.service";

/// System-level environment file path.
const SYSTEM_ENV_PATH: &str = "/etc/betcode/daemon.env";

/// Returns the path to the user-level systemd unit file.
///
/// `~/.config/systemd/user/betcode-daemon.service`
pub(super) fn user_unit_path() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    Ok(home.join(".config/systemd/user/betcode-daemon.service"))
}

/// Returns the path to the user-level environment file.
///
/// `~/.config/betcode/daemon.env`
pub(super) fn user_env_path() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    Ok(home.join(".config/betcode/daemon.env"))
}

/// Deploy the daemon as a system-level systemd service.
///
/// Assumes we are running as root (enforced by escalate).
pub fn deploy_system(config: &DaemonSetupConfig, is_update: bool) -> Result<()> {
    ensure_user_exists(config)?;
    create_system_directories(config)?;
    write_env_file(config, is_update, SYSTEM_ENV_PATH, true)?;
    write_system_unit(config)?;

    // Stop the running service before overwriting the binary to avoid
    // "Text file busy" (ETXTBSY) on Linux.
    if is_update && is_daemon_active_system() {
        tracing::info!("stopping betcode-daemon before binary update");
        run_cmd(
            "stopping betcode-daemon",
            "systemctl",
            &["stop", "betcode-daemon"],
        )?;
    }

    install_daemon_binary(config)?;

    if config.enable_service {
        enable_and_start_system(is_update)?;
    } else {
        tracing::info!(
            "service not enabled — run `sudo systemctl enable --now betcode-daemon` to start it"
        );
    }

    Ok(())
}

/// Deploy the daemon as a user-level systemd service.
pub fn deploy_user(config: &DaemonSetupConfig, is_update: bool) -> Result<()> {
    let binary_path = resolve_user_binary(config)?;
    create_user_directories()?;

    let env_path = user_env_path()?;
    let env_path_str = env_path.to_string_lossy().to_string();
    write_env_file(config, is_update, &env_path_str, false)?;

    write_user_unit(&binary_path)?;

    // Stop the running service before updating
    if is_update && is_daemon_active_user() {
        tracing::info!("stopping betcode-daemon before update");
        run_cmd(
            "stopping betcode-daemon (user)",
            "systemctl",
            &["--user", "stop", "betcode-daemon"],
        )?;
    }

    if config.enable_service {
        enable_and_start_user(is_update)?;
    } else {
        tracing::info!(
            "service not enabled — run `systemctl --user enable --now betcode-daemon` to start it"
        );
    }

    if config.enable_linger {
        enable_linger()?;
    }

    Ok(())
}

/// Check whether the system-level betcode-daemon service is active.
pub(super) fn is_daemon_active_system() -> bool {
    std::process::Command::new("systemctl")
        .args(["is-active", "--quiet", "betcode-daemon"])
        .status()
        .is_ok_and(|s| s.success())
}

/// Check whether the user-level betcode-daemon service is active.
pub(super) fn is_daemon_active_user() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "betcode-daemon"])
        .status()
        .is_ok_and(|s| s.success())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Resolve the absolute path to the daemon binary for user-mode deployment.
///
/// Uses `--daemon-binary` if provided, otherwise looks up `betcode-daemon` on `PATH`.
fn resolve_user_binary(config: &DaemonSetupConfig) -> Result<PathBuf> {
    if let Some(ref path) = config.daemon_binary_path {
        tracing::info!("using provided daemon binary: {}", path.display());
        return Ok(path.clone());
    }

    // Try to find betcode-daemon on PATH via `which`
    let output = std::process::Command::new("which")
        .arg("betcode-daemon")
        .output()
        .context("failed to run `which betcode-daemon`")?;

    if output.status.success() {
        let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let path = PathBuf::from(&path_str);
        tracing::info!("found daemon binary on PATH: {}", path.display());
        return Ok(path);
    }

    anyhow::bail!(
        "betcode-daemon not found on PATH. Provide --daemon-binary with the absolute path \
         to the binary, or install betcode-daemon to a directory on your PATH."
    )
}

/// Ensure the service user exists.
///
/// - If user is "betcode", create via `ensure_system_user()`.
/// - If a custom user name is given, verify it exists.
fn ensure_user_exists(config: &DaemonSetupConfig) -> Result<()> {
    if config.user == "betcode" {
        return ensure_system_user();
    }

    // Custom user: verify it exists
    let exists = std::process::Command::new("id")
        .arg(&config.user)
        .output()
        .is_ok_and(|o| o.status.success());

    if !exists {
        anyhow::bail!(
            "system user '{}' does not exist — create it first or use the default 'betcode' user",
            config.user
        );
    }

    tracing::debug!("using existing system user '{}'", config.user);
    Ok(())
}

/// Create system directories: `/var/lib/betcode` and `/etc/betcode`.
fn create_system_directories(config: &DaemonSetupConfig) -> Result<()> {
    create_setup_directories(&config.db_path, &config.user)
}

/// Write the environment file. Preserves existing content on update.
fn write_env_file(
    config: &DaemonSetupConfig,
    is_update: bool,
    path: &str,
    is_system: bool,
) -> Result<()> {
    let content = templates::env_file(config);
    if !write_env_if_fresh(path, &content, is_update)? {
        return Ok(());
    }

    // Set permissions (skip in tests — requires root / specific user)
    #[cfg(not(test))]
    {
        if is_system {
            // 0640 — root can read/write, daemon group can read
            fs::set_permissions(path, fs::Permissions::from_mode(0o640))
                .context("failed to set permissions on daemon.env")?;

            run_cmd(
                "setting ownership on daemon.env",
                "chown",
                &[&format!("root:{}", config.user), path],
            )?;
        } else {
            // 0600 — user can read/write only
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .context("failed to set permissions on daemon.env")?;
        }
    }

    // Suppress unused variable warning in test builds
    #[cfg(test)]
    let _ = is_system;

    Ok(())
}

/// Write the system-level systemd unit and reload the daemon.
fn write_system_unit(config: &DaemonSetupConfig) -> Result<()> {
    let path = SYSTEM_UNIT_PATH;
    tracing::info!("writing systemd unit: {path}");
    let content = templates::systemd_unit_system(config);
    fs::write(path, content).context("failed to write systemd unit")?;

    run_cmd("reloading systemd daemon", "systemctl", &["daemon-reload"])?;
    Ok(())
}

/// Write the user-level systemd unit and reload the user daemon.
fn write_user_unit(binary_path: &Path) -> Result<()> {
    let path = user_unit_path()?;

    tracing::info!("writing user systemd unit: {}", path.display());
    let content = templates::systemd_unit_user(binary_path);
    fs::write(&path, content).context("failed to write user systemd unit")?;

    run_cmd(
        "reloading user systemd daemon",
        "systemctl",
        &["--user", "daemon-reload"],
    )?;
    Ok(())
}

/// Install the daemon binary to `/usr/local/bin/betcode-daemon`.
fn install_daemon_binary(config: &DaemonSetupConfig) -> Result<()> {
    let dest = "/usr/local/bin/betcode-daemon";

    let Some(src) = config.daemon_binary_path.as_ref() else {
        // No --daemon-binary provided — use existing binary at dest
        if Path::new(dest).exists() {
            tracing::info!("using existing daemon binary at {dest}");
            return Ok(());
        }
        anyhow::bail!("daemon binary not found at {dest} and --daemon-binary not provided");
    };

    tracing::info!("installing daemon binary: {} -> {dest}", src.display());
    fs::copy(src, dest).with_context(|| format!("failed to copy {} to {dest}", src.display()))?;

    // Ensure executable
    fs::set_permissions(dest, fs::Permissions::from_mode(0o755))
        .context("failed to set permissions on daemon binary")?;

    Ok(())
}

/// Enable and start the system-level service.
fn enable_and_start_system(is_update: bool) -> Result<()> {
    let (description, args) = if is_update {
        (
            "restarting betcode-daemon",
            vec!["restart", "betcode-daemon"],
        )
    } else {
        (
            "enabling and starting betcode-daemon",
            vec!["enable", "--now", "betcode-daemon"],
        )
    };

    run_cmd(description, "systemctl", &args)?;

    // Verify service is active
    let result = run_cmd(
        "verifying service status",
        "systemctl",
        &["is-active", "betcode-daemon"],
    );
    if result.is_err() {
        tracing::warn!(
            "service may not have started correctly — check: journalctl -u betcode-daemon"
        );
    }

    tracing::info!("betcode-daemon is deployed and running (system)");
    Ok(())
}

/// Create user-level directories for the daemon.
fn create_user_directories() -> Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;

    let dirs = [
        home.join(".config/systemd/user"),
        home.join(".config/betcode"),
        home.join(".local/share/betcode"),
    ];

    for dir in &dirs {
        tracing::info!("creating directory: {}", dir.display());
        fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    }

    Ok(())
}

/// Enable and start the user-level service.
fn enable_and_start_user(is_update: bool) -> Result<()> {
    let (description, args) = if is_update {
        (
            "restarting betcode-daemon (user)",
            vec!["--user", "restart", "betcode-daemon"],
        )
    } else {
        (
            "enabling and starting betcode-daemon (user)",
            vec!["--user", "enable", "--now", "betcode-daemon"],
        )
    };

    run_cmd(description, "systemctl", &args)?;

    // Verify service is active
    let result = run_cmd(
        "verifying user service status",
        "systemctl",
        &["--user", "is-active", "betcode-daemon"],
    );
    if result.is_err() {
        tracing::warn!(
            "user service may not have started correctly — check: journalctl --user -u betcode-daemon"
        );
    }

    tracing::info!("betcode-daemon is deployed and running (user)");
    Ok(())
}

/// Enable lingering so the user service runs after logout.
fn enable_linger() -> Result<()> {
    run_cmd(
        "enabling lingering for current user",
        "loginctl",
        &["enable-linger"],
    )?;
    tracing::info!("lingering enabled — daemon will survive logout");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DaemonMode;

    use super::super::make_test_daemon_config;

    #[test]
    fn system_unit_path_is_correct() {
        assert_eq!(
            SYSTEM_UNIT_PATH,
            "/etc/systemd/system/betcode-daemon.service"
        );
    }

    /// Write an env file via `write_env_file` into a temp directory and
    /// return its final contents. Optionally seeds the file with `initial`
    /// content before calling the function under test.
    #[allow(clippy::expect_used)]
    fn run_write_env_file(is_update: bool, initial: Option<&str>) -> String {
        let dir = tempfile::tempdir().expect("tempdir");
        let env_path = dir.path().join("daemon.env");
        if let Some(data) = initial {
            std::fs::write(&env_path, data).expect("write");
        }

        let config = make_test_daemon_config(DaemonMode::System);
        let result = write_env_file(&config, is_update, env_path.to_str().expect("path"), true);
        assert!(result.is_ok(), "write_env_file should succeed");

        std::fs::read_to_string(&env_path).expect("read")
    }

    #[test]
    fn write_env_file_creates_on_fresh_install() {
        let content = run_write_env_file(false, None);
        assert!(content.contains("BETCODE_ADDR=127.0.0.1:50051"));
        assert!(content.contains("BETCODE_DB_PATH=/var/lib/betcode/daemon.db"));
    }

    #[test]
    fn write_env_file_preserves_on_update() {
        let content = run_write_env_file(true, Some("BETCODE_ADDR=original\n"));
        assert!(
            content.contains("original"),
            "env file must be preserved on update"
        );
    }

    #[test]
    fn write_env_file_creates_on_update_when_missing() {
        let content = run_write_env_file(true, None);
        assert!(
            content.contains("BETCODE_ADDR="),
            "env file should be created even on update if it doesn't exist"
        );
    }
}
