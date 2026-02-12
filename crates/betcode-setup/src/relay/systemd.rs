use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cmd::{command_exists, run_cmd};
use crate::config::RelaySetupConfig;

use super::templates;

/// Deploy the relay using systemd.
/// Assumes we are running as root (enforced by escalate).
pub fn deploy(config: &RelaySetupConfig) -> Result<()> {
    create_system_user()?;
    create_directories(config)?;
    write_env_file(config)?;
    write_systemd_unit(config)?;
    install_relay_binary(config)?;
    setup_certbot(config)?;
    enable_and_start()?;
    Ok(())
}

fn create_system_user() -> Result<()> {
    // Check if user already exists before attempting to create
    if crate::cmd::command_exists("id")
        && std::process::Command::new("id")
            .arg("betcode")
            .output()
            .is_ok_and(|o| o.status.success())
    {
        tracing::debug!("system user 'betcode' already exists");
        return Ok(());
    }

    run_cmd(
        "creating system user 'betcode'",
        "useradd",
        &["--system", "--shell", "/usr/sbin/nologin", "betcode"],
    )
}

fn create_directories(config: &RelaySetupConfig) -> Result<()> {
    let db_dir = config
        .db_path
        .parent()
        .unwrap_or_else(|| Path::new("/var/lib/betcode"));

    for dir in &[db_dir, Path::new("/etc/betcode")] {
        tracing::info!("creating directory: {}", dir.display());
        fs::create_dir_all(dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
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

fn write_env_file(config: &RelaySetupConfig) -> Result<()> {
    let path = "/etc/betcode/relay.env";
    tracing::info!("writing environment file: {path}");
    let content = templates::env_file(config);
    fs::write(path, content).context("failed to write relay.env")?;

    // 0640 — root can read/write, betcode group can read
    fs::set_permissions(path, fs::Permissions::from_mode(0o640))
        .context("failed to set permissions on relay.env")?;

    run_cmd("setting ownership on relay.env", "chown", &["root:betcode", path])?;
    Ok(())
}

fn write_systemd_unit(config: &RelaySetupConfig) -> Result<()> {
    let path = "/etc/systemd/system/betcode-relay.service";
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
        anyhow::bail!(
            "relay binary not found at {dest} and --relay-binary not provided"
        );
    };

    tracing::info!("installing relay binary: {} -> {dest}", src.display());
    fs::copy(src, dest).with_context(|| {
        format!("failed to copy {} to {dest}", src.display())
    })?;

    // Ensure executable
    fs::set_permissions(dest, fs::Permissions::from_mode(0o755))
        .context("failed to set permissions on relay binary")?;

    Ok(())
}

fn setup_certbot(config: &RelaySetupConfig) -> Result<()> {
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
        &["certonly", "--standalone", "-d", &config.domain, "--agree-tos", "--non-interactive", "--register-unsafely-without-email"],
    )?;

    // Install renewal hooks
    let hooks_dir = "/etc/letsencrypt/renewal-hooks";
    for subdir in &["pre", "post"] {
        let dir = format!("{hooks_dir}/{subdir}");
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create {dir}"))?;
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

fn enable_and_start() -> Result<()> {
    run_cmd(
        "enabling and starting betcode-relay",
        "systemctl",
        &["enable", "--now", "betcode-relay"],
    )?;

    // Verify service is active
    let result = run_cmd("verifying service status", "systemctl", &["is-active", "betcode-relay"]);
    if result.is_err() {
        tracing::warn!("service may not have started correctly — check: journalctl -u betcode-relay");
    }

    tracing::info!("betcode-relay is deployed and running");
    Ok(())
}
