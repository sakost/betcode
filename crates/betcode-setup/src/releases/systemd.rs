use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cmd::{ensure_system_user, run_cmd};

use super::templates;

/// Path to the systemd service unit file.
pub const SERVICE_UNIT_PATH: &str = "/etc/systemd/system/betcode-releases.service";

/// Deploy the releases server using systemd.
/// Assumes we are running as root (enforced by escalate).
pub fn deploy(
    binary: Option<&Path>,
    domain: &str,
    repo: &str,
    is_update: bool,
    use_caddy: bool,
) -> Result<()> {
    ensure_system_user()?;

    // Stop existing service before overwriting the binary to avoid ETXTBSY.
    if is_update && super::validate::is_releases_active() {
        tracing::info!("stopping betcode-releases before update");
        run_cmd(
            "stopping betcode-releases",
            "systemctl",
            &["stop", "betcode-releases"],
        )?;
    }

    install_binary(binary)?;

    // When Caddy is fronting, bind only to localhost.
    let localhost_only = use_caddy;
    let unit_content = templates::systemd_unit(domain, repo, localhost_only);
    tracing::info!("writing systemd unit: {SERVICE_UNIT_PATH}");
    fs::write(SERVICE_UNIT_PATH, unit_content).context("failed to write releases systemd unit")?;

    run_cmd("reloading systemd daemon", "systemctl", &["daemon-reload"])?;

    if is_update {
        run_cmd(
            "restarting betcode-releases",
            "systemctl",
            &["restart", "betcode-releases"],
        )?;
    } else {
        run_cmd(
            "enabling and starting betcode-releases",
            "systemctl",
            &["enable", "--now", "betcode-releases"],
        )?;
    }

    tracing::info!("betcode-releases is deployed and running on port 8090");
    Ok(())
}

/// Install the betcode-releases binary to `/usr/local/bin/`.
fn install_binary(src: Option<&Path>) -> Result<()> {
    let dest = "/usr/local/bin/betcode-releases";

    let Some(src) = src else {
        if Path::new(dest).exists() {
            tracing::info!("using existing releases binary at {dest}");
            return Ok(());
        }
        anyhow::bail!("releases binary not found at {dest} and --releases-binary not provided");
    };

    tracing::info!("installing releases binary: {} -> {dest}", src.display());
    fs::copy(src, dest).with_context(|| format!("failed to copy {} to {dest}", src.display()))?;

    fs::set_permissions(dest, fs::Permissions::from_mode(0o755))
        .context("failed to set permissions on releases binary")?;

    Ok(())
}
