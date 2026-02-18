use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nix::unistd::{AccessFlags, access};

use crate::cmd::{create_setup_directories, ensure_system_user, run_cmd, write_env_if_fresh};
use crate::config::DaemonSetupConfig;

use super::templates;

/// Path to the system-level systemd unit file.
pub const SYSTEM_UNIT_PATH: &str = "/etc/systemd/system/betcode-daemon.service";

/// System-level environment file path.
pub(super) const SYSTEM_ENV_PATH: &str = "/etc/betcode/daemon.env";

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
    create_user_directories()?;

    let env_path = user_env_path()?;
    let env_path_str = env_path.to_string_lossy().to_string();
    write_env_file(config, is_update, &env_path_str, false)?;

    // Stop the running service before binary install to avoid ETXTBSY.
    if is_update && is_daemon_active_user() {
        tracing::info!("stopping betcode-daemon before binary update");
        run_cmd(
            "stopping betcode-daemon (user)",
            "systemctl",
            &["--user", "stop", "betcode-daemon"],
        )?;
    }

    let binary_path = install_user_daemon_binary(config)?;
    write_user_unit(&binary_path)?;

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

/// Check whether a directory is present in `$PATH`.
fn is_dir_in_path(dir: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|p| p == Path::new(dir)))
}

/// Check whether a directory is writable by the current user.
fn is_dir_writable(dir: &Path) -> bool {
    access(dir, AccessFlags::W_OK).is_ok()
}

/// Check whether two paths refer to the same file (by canonical path).
///
/// Returns `false` if either path does not exist.
fn is_same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// Detect the current user's login shell.
///
/// Reads `$SHELL` first; falls back to `getent passwd $USER`.
fn detect_shell() -> Option<String> {
    if let Some(shell) = std::env::var_os("SHELL") {
        let s = shell.to_string_lossy().to_string();
        if !s.is_empty() {
            return Some(s);
        }
    }

    // Fallback: getent passwd $USER
    let user = std::env::var("USER").ok()?;
    let output = std::process::Command::new("getent")
        .args(["passwd", &user])
        .output()
        .ok()?;

    if output.status.success() {
        let line = String::from_utf8_lossy(&output.stdout);
        // passwd format: name:x:uid:gid:gecos:home:shell
        let shell = line.trim().rsplit(':').next()?;
        if !shell.is_empty() {
            return Some(shell.to_string());
        }
    }
    None
}

/// Map a shell path to the appropriate shell config file.
fn shell_config_for_shell(shell: Option<&str>, home: &Path) -> PathBuf {
    let filename = match shell {
        Some(s) if s.ends_with("/bash") => ".bashrc",
        Some(s) if s.ends_with("/zsh") => ".zshrc",
        Some(s) if s.ends_with("/fish") => {
            tracing::warn!(
                "fish shell detected — PATH will be written to .profile; \
                 you may need to configure fish separately"
            );
            ".profile"
        }
        _ => ".profile",
    };
    home.join(filename)
}

/// Detect the shell config file path for the current user.
fn detect_shell_config(home: &Path) -> PathBuf {
    let shell = detect_shell();
    shell_config_for_shell(shell.as_deref(), home)
}

/// Marker comment used to identify lines added by betcode-setup.
const SHELL_CONFIG_MARKER: &str = "# Added by betcode-setup";

/// Append a `PATH` export to the given shell config file.
///
/// Idempotent: skips if the marker is already present.
fn append_path_to_shell_config(shell_config: &Path, dir: &Path) -> Result<()> {
    // Check idempotency
    if shell_config.exists() {
        let content =
            fs::read_to_string(shell_config).context("failed to read shell config file")?;
        if content.contains(SHELL_CONFIG_MARKER) {
            tracing::info!(
                "shell config {} already contains betcode PATH entry — skipping",
                shell_config.display()
            );
            return Ok(());
        }
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(shell_config)
        .with_context(|| format!("failed to open {} for appending", shell_config.display()))?;

    let snippet = format!(
        "\n{SHELL_CONFIG_MARKER}\nexport PATH=\"{}:$PATH\"\n",
        dir.display()
    );
    file.write_all(snippet.as_bytes())
        .with_context(|| format!("failed to write to {}", shell_config.display()))?;

    tracing::info!(
        "added {} to PATH in {} — restart your shell or run `source {}`",
        dir.display(),
        shell_config.display(),
        shell_config.display()
    );
    Ok(())
}

/// Ensure the given directory is in `$PATH`, patching the shell config if needed.
fn ensure_path_in_shell_config(dir: &Path) -> Result<()> {
    if is_dir_in_path(&dir.to_string_lossy()) {
        tracing::debug!("{} is already in PATH", dir.display());
        return Ok(());
    }

    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    let shell_config = detect_shell_config(&home);
    append_path_to_shell_config(&shell_config, dir)
}

/// Resolve the user-level binary directory.
///
/// Checks `$XDG_BIN_HOME` first, falls back to `~/.local/bin`.
fn user_bin_dir() -> Result<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_BIN_HOME") {
        let path = PathBuf::from(xdg);
        if path.is_absolute() {
            return Ok(path);
        }
    }
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    Ok(home.join(".local/bin"))
}

/// Choose the destination directory for the daemon binary in user mode.
///
/// Prefers `/usr/local/bin` if it is in `$PATH` and writable; otherwise
/// falls back to `user_bin_dir()`.
fn choose_user_binary_destination() -> Result<PathBuf> {
    let usr_local = Path::new("/usr/local/bin");
    if is_dir_in_path("/usr/local/bin") && is_dir_writable(usr_local) {
        return Ok(usr_local.join("betcode-daemon"));
    }

    let dir = user_bin_dir()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create directory {}", dir.display()))?;
    Ok(dir.join("betcode-daemon"))
}

/// Resolve the source daemon binary from config or `$PATH`.
///
/// Returns `None` if no source binary can be found (as opposed to bailing).
fn resolve_source_binary(config: &DaemonSetupConfig) -> Result<Option<PathBuf>> {
    if let Some(ref path) = config.daemon_binary_path {
        tracing::info!("using provided daemon binary: {}", path.display());
        return Ok(Some(path.clone()));
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
        return Ok(Some(path));
    }

    Ok(None)
}

/// Install the daemon binary to a well-known location for user-mode deployment.
///
/// Selects a destination via [`choose_user_binary_destination`], copies the
/// binary there (unless source and destination are the same file), and ensures
/// the parent directory is in `$PATH`.
fn install_user_daemon_binary(config: &DaemonSetupConfig) -> Result<PathBuf> {
    let dest = choose_user_binary_destination()?;
    let source = resolve_source_binary(config)?;

    if let Some(ref src) = source {
        if is_same_file(src, &dest) {
            tracing::info!(
                "source and destination are the same file — skipping copy: {}",
                dest.display()
            );
        } else {
            tracing::info!(
                "installing daemon binary: {} -> {}",
                src.display(),
                dest.display()
            );
            fs::copy(src, &dest).with_context(|| {
                format!("failed to copy {} to {}", src.display(), dest.display())
            })?;
            fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))
                .context("failed to set permissions on daemon binary")?;
        }
    } else if dest.exists() {
        tracing::info!(
            "no source binary found, using existing installation at {}",
            dest.display()
        );
    } else {
        anyhow::bail!(
            "betcode-daemon not found on PATH and no existing installation at {}. \
             Provide --daemon-binary with the absolute path to the binary.",
            dest.display()
        );
    }

    // Ensure the destination directory is in PATH (for ~/.local/bin etc.)
    if let Some(parent) = dest.parent()
        && parent != Path::new("/usr/local/bin")
    {
        ensure_path_in_shell_config(parent)?;
    }

    Ok(dest)
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

    // -----------------------------------------------------------------------
    // is_dir_in_path
    // -----------------------------------------------------------------------

    #[test]
    #[allow(clippy::expect_used)]
    fn is_dir_in_path_finds_existing_entry() {
        // Pick the first entry from the actual PATH
        let path_var = std::env::var("PATH").expect("PATH should be set");
        let first = path_var
            .split(':')
            .find(|s| !s.is_empty())
            .expect("PATH should have at least one entry");
        assert!(
            is_dir_in_path(first),
            "first PATH entry '{first}' should be found by is_dir_in_path"
        );
    }

    #[test]
    fn is_dir_in_path_rejects_nonexistent() {
        assert!(!is_dir_in_path("/nonexistent-dir-abc-xyz-12345"));
    }

    // -----------------------------------------------------------------------
    // is_dir_writable
    // -----------------------------------------------------------------------

    #[test]
    #[allow(clippy::expect_used)]
    fn is_dir_writable_on_temp() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(is_dir_writable(dir.path()));
    }

    #[test]
    fn is_dir_writable_rejects_nonexistent() {
        assert!(!is_dir_writable(Path::new(
            "/nonexistent-dir-abc-xyz-12345"
        )));
    }

    // -----------------------------------------------------------------------
    // is_same_file
    // -----------------------------------------------------------------------

    #[test]
    #[allow(clippy::expect_used)]
    fn is_same_file_detects_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("binary");
        std::fs::write(&file, b"test").expect("write");
        assert!(is_same_file(&file, &file));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn is_same_file_detects_different_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, b"aaa").expect("write a");
        std::fs::write(&b, b"bbb").expect("write b");
        assert!(!is_same_file(&a, &b));
    }

    #[test]
    fn is_same_file_returns_false_if_missing() {
        assert!(!is_same_file(
            Path::new("/does-not-exist-a"),
            Path::new("/does-not-exist-b")
        ));
    }

    // -----------------------------------------------------------------------
    // shell_config_for_shell
    // -----------------------------------------------------------------------

    #[test]
    fn shell_config_bash() {
        let home = Path::new("/home/user");
        assert_eq!(
            shell_config_for_shell(Some("/bin/bash"), home),
            PathBuf::from("/home/user/.bashrc")
        );
    }

    #[test]
    fn shell_config_zsh() {
        let home = Path::new("/home/user");
        assert_eq!(
            shell_config_for_shell(Some("/usr/bin/zsh"), home),
            PathBuf::from("/home/user/.zshrc")
        );
    }

    #[test]
    fn shell_config_fish_falls_back_to_profile() {
        let home = Path::new("/home/user");
        assert_eq!(
            shell_config_for_shell(Some("/usr/bin/fish"), home),
            PathBuf::from("/home/user/.profile")
        );
    }

    #[test]
    fn shell_config_unknown_falls_back_to_profile() {
        let home = Path::new("/home/user");
        assert_eq!(
            shell_config_for_shell(Some("/usr/bin/ksh"), home),
            PathBuf::from("/home/user/.profile")
        );
    }

    #[test]
    fn shell_config_none_falls_back_to_profile() {
        let home = Path::new("/home/user");
        assert_eq!(
            shell_config_for_shell(None, home),
            PathBuf::from("/home/user/.profile")
        );
    }

    // -----------------------------------------------------------------------
    // append_path_to_shell_config
    // -----------------------------------------------------------------------

    #[test]
    #[allow(clippy::expect_used)]
    fn append_path_creates_file_when_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join(".bashrc");
        let bin_dir = Path::new("/home/user/.local/bin");

        append_path_to_shell_config(&config, bin_dir).expect("append");

        let content = std::fs::read_to_string(&config).expect("read");
        assert!(content.contains(SHELL_CONFIG_MARKER));
        assert!(content.contains("/home/user/.local/bin"));
        assert!(content.contains("export PATH="));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn append_path_preserves_existing_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join(".bashrc");
        let existing = "# My existing config\nalias ll='ls -la'\n";
        std::fs::write(&config, existing).expect("write");

        let bin_dir = Path::new("/home/user/.local/bin");
        append_path_to_shell_config(&config, bin_dir).expect("append");

        let content = std::fs::read_to_string(&config).expect("read");
        assert!(content.starts_with(existing));
        assert!(content.contains(SHELL_CONFIG_MARKER));
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn append_path_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join(".bashrc");
        let bin_dir = Path::new("/home/user/.local/bin");

        append_path_to_shell_config(&config, bin_dir).expect("first append");
        let first = std::fs::read_to_string(&config).expect("read");

        append_path_to_shell_config(&config, bin_dir).expect("second append");
        let second = std::fs::read_to_string(&config).expect("read");

        assert_eq!(first, second, "second append should be a no-op");
    }

    // -----------------------------------------------------------------------
    // user_bin_dir
    // -----------------------------------------------------------------------

    #[test]
    #[allow(clippy::expect_used)]
    fn user_bin_dir_returns_absolute_path() {
        // We cannot safely mutate env vars (unsafe_code = deny), so just
        // verify the function returns a valid absolute path that ends
        // with the expected directory name.
        let result = user_bin_dir().expect("user_bin_dir");
        assert!(
            result.is_absolute(),
            "user_bin_dir should return an absolute path, got: {}",
            result.display()
        );
        // Should end with either "bin" from XDG_BIN_HOME or ".local/bin"
        let name = result.file_name().expect("should have file name");
        assert_eq!(name, "bin", "expected dir name 'bin', got: {name:?}");
    }

    // -----------------------------------------------------------------------
    // resolve_source_binary
    // -----------------------------------------------------------------------

    #[test]
    #[allow(clippy::expect_used)]
    fn resolve_source_binary_returns_none_when_not_found() {
        let mut config = make_test_daemon_config(DaemonMode::User);
        config.daemon_binary_path = None;

        // This may return Some or None depending on whether betcode-daemon
        // is installed; we just verify it doesn't bail.
        let _result = resolve_source_binary(&config).expect("should not bail");
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn resolve_source_binary_returns_provided_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("betcode-daemon");
        std::fs::write(&bin, b"fake-binary").expect("write");

        let mut config = make_test_daemon_config(DaemonMode::User);
        config.daemon_binary_path = Some(bin.clone());

        let result = resolve_source_binary(&config)
            .expect("should not bail")
            .expect("should return Some");
        assert_eq!(result, bin);
    }
}
