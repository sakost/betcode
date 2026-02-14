use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cmd::{command_exists, ensure_system_user, run_cmd};
use crate::config::RelaySetupConfig;

use super::templates;

/// Path to the systemd service unit file.
pub const SERVICE_UNIT_PATH: &str = "/etc/systemd/system/betcode-relay.service";

/// Path to the environment file.
const ENV_FILE_PATH: &str = "/etc/betcode/relay.env";

/// Deploy the relay using systemd.
/// Assumes we are running as root (enforced by escalate).
pub fn deploy(config: &RelaySetupConfig, is_update: bool) -> Result<()> {
    ensure_system_user()?;
    create_directories(config)?;
    write_env_file(config, is_update)?;
    write_systemd_unit(config)?;

    // Stop the running service before overwriting the binary to avoid
    // "Text file busy" (ETXTBSY) on Linux.
    if is_update && super::validate::is_betcode_relay_active() {
        tracing::info!("stopping betcode-relay before binary update");
        run_cmd(
            "stopping betcode-relay",
            "systemctl",
            &["stop", "betcode-relay"],
        )?;
    }

    install_relay_binary(config)?;
    setup_certbot(config, is_update)?;
    enable_and_start(is_update)?;
    Ok(())
}

fn create_directories(config: &RelaySetupConfig) -> Result<()> {
    let db_dir = config
        .db_path
        .parent()
        .unwrap_or_else(|| Path::new("/var/lib/betcode"));

    for dir in &[db_dir, Path::new("/etc/betcode")] {
        tracing::info!("creating directory: {}", dir.display());
        fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    }

    run_cmd(
        "setting ownership on /var/lib/betcode",
        "chown",
        &["-R", "betcode:betcode", &db_dir.to_string_lossy()],
    )?;

    run_cmd(
        "setting ownership on /etc/betcode",
        "chown",
        &["root:betcode", "/etc/betcode"],
    )?;

    Ok(())
}

fn write_env_file(config: &RelaySetupConfig, is_update: bool) -> Result<()> {
    write_env_file_inner(config, is_update, ENV_FILE_PATH)
}

fn write_env_file_inner(config: &RelaySetupConfig, is_update: bool, path: &str) -> Result<()> {
    if is_update && Path::new(path).exists() {
        tracing::warn!("existing {path} preserved — to regenerate, delete it and re-run setup");
        return Ok(());
    }

    tracing::info!("writing environment file: {path}");
    let content = templates::env_file(config);
    fs::write(path, content).context("failed to write relay.env")?;

    // Skip chown/chmod in tests (requires root)
    #[cfg(not(test))]
    {
        // 0640 — root can read/write, betcode group can read
        fs::set_permissions(path, fs::Permissions::from_mode(0o640))
            .context("failed to set permissions on relay.env")?;

        run_cmd(
            "setting ownership on relay.env",
            "chown",
            &["root:betcode", path],
        )?;
    }
    Ok(())
}

fn write_systemd_unit(config: &RelaySetupConfig) -> Result<()> {
    let path = SERVICE_UNIT_PATH;
    tracing::info!("writing systemd unit: {path}");
    let content = templates::systemd_unit(config);
    fs::write(path, content).context("failed to write systemd unit")?;

    run_cmd("reloading systemd daemon", "systemctl", &["daemon-reload"])?;
    Ok(())
}

fn install_relay_binary(config: &RelaySetupConfig) -> Result<()> {
    let dest = "/usr/local/bin/betcode-relay";

    let Some(src) = config.relay_binary_path.as_ref() else {
        // No --relay-binary provided — use existing binary at dest
        if Path::new(dest).exists() {
            tracing::info!("using existing relay binary at {dest}");
            return Ok(());
        }
        anyhow::bail!("relay binary not found at {dest} and --relay-binary not provided");
    };

    tracing::info!("installing relay binary: {} -> {dest}", src.display());
    fs::copy(src, dest).with_context(|| format!("failed to copy {} to {dest}", src.display()))?;

    // Ensure executable
    fs::set_permissions(dest, fs::Permissions::from_mode(0o755))
        .context("failed to set permissions on relay binary")?;

    Ok(())
}

/// Check whether certbot setup should be skipped.
fn should_skip_certbot(is_update: bool, cert_path: &str) -> bool {
    is_update && Path::new(cert_path).exists()
}

fn setup_certbot(config: &RelaySetupConfig, is_update: bool) -> Result<()> {
    let cert_path = format!("/etc/letsencrypt/live/{}/fullchain.pem", config.domain);
    if should_skip_certbot(is_update, &cert_path) {
        tracing::info!("TLS certificate already exists — skipping certbot setup");
        return Ok(());
    }
    if is_update {
        tracing::info!("TLS certificate not found — running certbot despite update mode");
    }

    if !command_exists("certbot") {
        run_cmd(
            "installing certbot",
            "apt-get",
            &["install", "-y", "certbot"],
        )?;
    }

    run_cmd(
        &format!("obtaining TLS certificate for {}", config.domain),
        "certbot",
        &[
            "certonly",
            "--standalone",
            "-d",
            &config.domain,
            "--agree-tos",
            "--non-interactive",
            "--register-unsafely-without-email",
        ],
    )?;

    // Grant betcode user read access to letsencrypt dirs via ACLs
    // (certbot defaults to 0700 root:root on /etc/letsencrypt/{live,archive})
    if !command_exists("setfacl") {
        run_cmd(
            "installing acl package",
            "apt-get",
            &["install", "-y", "acl"],
        )?;
    }
    for dir in &[
        "/etc/letsencrypt/live",
        "/etc/letsencrypt/archive",
        &format!("/etc/letsencrypt/live/{}", config.domain),
        &format!("/etc/letsencrypt/archive/{}", config.domain),
    ] {
        if Path::new(dir).exists() {
            run_cmd(
                &format!("setting ACL on {dir}"),
                "setfacl",
                &["-R", "-m", "u:betcode:rX", dir],
            )?;
            // Default ACL so renewed certs also get the right perms
            run_cmd(
                &format!("setting default ACL on {dir}"),
                "setfacl",
                &["-R", "-m", "d:u:betcode:rX", dir],
            )?;
        }
    }

    // Install renewal hooks
    let hooks_dir = "/etc/letsencrypt/renewal-hooks";
    for subdir in &["pre", "post"] {
        let dir = format!("{hooks_dir}/{subdir}");
        fs::create_dir_all(&dir).with_context(|| format!("failed to create {dir}"))?;
    }

    let pre_hook_path = format!("{hooks_dir}/pre/betcode-relay.sh");
    let post_hook_path = format!("{hooks_dir}/post/betcode-relay.sh");

    tracing::info!("installing certbot renewal hooks");
    fs::write(&pre_hook_path, templates::certbot_pre_hook())
        .context("failed to write certbot pre-hook")?;
    fs::write(&post_hook_path, templates::certbot_post_hook())
        .context("failed to write certbot post-hook")?;

    fs::set_permissions(&pre_hook_path, fs::Permissions::from_mode(0o755))?;
    fs::set_permissions(&post_hook_path, fs::Permissions::from_mode(0o755))?;

    Ok(())
}

/// Determine systemctl arguments for starting or restarting the service.
fn start_command_args(is_update: bool) -> (&'static str, Vec<&'static str>) {
    if is_update {
        ("restarting betcode-relay", vec!["restart", "betcode-relay"])
    } else {
        (
            "enabling and starting betcode-relay",
            vec!["enable", "--now", "betcode-relay"],
        )
    }
}

fn enable_and_start(is_update: bool) -> Result<()> {
    let (description, args) = start_command_args(is_update);
    run_cmd(description, "systemctl", &args)?;

    // Verify service is active
    let result = run_cmd(
        "verifying service status",
        "systemctl",
        &["is-active", "betcode-relay"],
    );
    if result.is_err() {
        tracing::warn!(
            "service may not have started correctly — check: journalctl -u betcode-relay"
        );
    }

    tracing::info!("betcode-relay is deployed and running");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DeploymentMode, RelaySetupConfig};
    use std::net::SocketAddr;
    use std::path::PathBuf;

    fn test_config() -> RelaySetupConfig {
        RelaySetupConfig {
            domain: "relay.example.com".into(),
            jwt_secret: "a".repeat(48),
            db_path: PathBuf::from("/var/lib/betcode/relay.db"),
            deployment_mode: DeploymentMode::Systemd,
            relay_binary_path: None,
            addr: "0.0.0.0:443".parse::<SocketAddr>().expect("valid addr"),
        }
    }

    // --- service_unit_exists ---

    #[test]
    fn service_unit_path_is_systemd_location() {
        assert_eq!(
            SERVICE_UNIT_PATH,
            "/etc/systemd/system/betcode-relay.service"
        );
    }

    // --- write_env_file_inner ---

    /// Write an env file via `write_env_file_inner` into a temp directory and
    /// return its final contents. Optionally seeds the file with `initial`
    /// content before calling the function under test.
    fn run_write_env_file(is_update: bool, initial: Option<&str>) -> String {
        let dir = tempfile::tempdir().expect("tempdir");
        let env_path = dir.path().join("relay.env");
        if let Some(data) = initial {
            std::fs::write(&env_path, data).expect("write");
        }

        let config = test_config();
        let result = write_env_file_inner(&config, is_update, env_path.to_str().expect("path"));
        assert!(result.is_ok());

        std::fs::read_to_string(&env_path).expect("read")
    }

    #[test]
    fn write_env_file_skips_existing_on_update() {
        let content = run_write_env_file(true, Some("BETCODE_JWT_SECRET=original-secret\n"));
        assert!(
            content.contains("original-secret"),
            "env file must be preserved on update"
        );
    }

    #[test]
    fn write_env_file_creates_on_fresh_install() {
        let content = run_write_env_file(false, None);
        assert!(content.contains("BETCODE_JWT_SECRET="));
        assert!(content.contains("BETCODE_DB_PATH="));
    }

    #[test]
    fn write_env_file_creates_on_update_when_missing() {
        let content = run_write_env_file(true, None);
        assert!(
            content.contains("BETCODE_JWT_SECRET="),
            "env file should be created even on update if it doesn't exist"
        );
    }

    // --- should_skip_certbot ---

    #[test]
    fn skip_certbot_when_cert_exists_on_update() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cert_path = dir.path().join("fullchain.pem");
        std::fs::write(&cert_path, "dummy cert").expect("write");

        assert!(should_skip_certbot(true, cert_path.to_str().expect("path")));
    }

    #[test]
    fn certbot_runs_on_fresh_install() {
        assert!(!should_skip_certbot(false, "/nonexistent/fullchain.pem"));
    }

    #[test]
    fn certbot_runs_on_update_when_cert_missing() {
        assert!(!should_skip_certbot(true, "/nonexistent/fullchain.pem"));
    }

    // --- start_command_args ---

    #[test]
    fn start_args_for_fresh_install() {
        let (desc, args) = start_command_args(false);
        assert!(desc.contains("enabling"));
        assert_eq!(args, vec!["enable", "--now", "betcode-relay"]);
    }

    #[test]
    fn start_args_for_update() {
        let (desc, args) = start_command_args(true);
        assert!(desc.contains("restarting"));
        assert_eq!(args, vec!["restart", "betcode-relay"]);
    }
}
