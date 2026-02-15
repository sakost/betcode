use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cmd::{command_exists, run_cmd};

use super::templates;

const MAIN_CADDYFILE: &str = "/etc/caddy/Caddyfile";
const CONF_DIR: &str = "/etc/caddy/conf.d";
const SITE_CONFIG: &str = "/etc/caddy/conf.d/betcode-releases.caddy";
const LOG_DIR: &str = "/var/log/caddy";
const LOG_FILE: &str = "/var/log/caddy/betcode-releases.log";

/// Set up Caddy as a reverse proxy for betcode-releases.
pub fn setup(domain: &str, acme_email: Option<&str>) -> Result<()> {
    install_caddy()?;
    ensure_conf_dir()?;
    write_main_caddyfile()?;
    write_site_config(domain, acme_email)?;
    create_log_dir()?;
    reload_caddy()?;

    tracing::info!("caddy reverse proxy configured for {domain}");
    Ok(())
}

/// Install Caddy via the official apt repository if not already present.
fn install_caddy() -> Result<()> {
    if command_exists("caddy") {
        tracing::info!("caddy is already installed");
        return Ok(());
    }

    tracing::info!("installing caddy via official apt repository");

    run_cmd(
        "installing prerequisites",
        "apt-get",
        &[
            "install",
            "-y",
            "debian-keyring",
            "debian-archive-keyring",
            "apt-transport-https",
            "curl",
        ],
    )?;

    // Download and install the GPG key
    let gpg_output = std::process::Command::new("curl")
        .args([
            "-1sLf",
            "https://dl.cloudsmith.io/public/caddy/stable/gpg.key",
        ])
        .output()
        .context("failed to download caddy GPG key")?;

    if !gpg_output.status.success() {
        anyhow::bail!("failed to download caddy GPG key");
    }

    let dearmor = std::process::Command::new("gpg")
        .args([
            "--dearmor",
            "-o",
            "/usr/share/keyrings/caddy-stable-archive-keyring.gpg",
        ])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .context("failed to spawn gpg")?;

    if let Some(mut stdin) = dearmor.stdin {
        stdin.write_all(&gpg_output.stdout)?;
    }

    // Add the apt source
    let source_line = "deb [signed-by=/usr/share/keyrings/caddy-stable-archive-keyring.gpg] \
                        https://dl.cloudsmith.io/public/caddy/stable/deb/debian any-version main";
    let source_path = "/etc/apt/sources.list.d/caddy-stable.list";
    fs::write(source_path, format!("{source_line}\n"))
        .context("failed to write caddy apt source")?;

    run_cmd("updating apt", "apt-get", &["update"])?;
    run_cmd("installing caddy", "apt-get", &["install", "-y", "caddy"])?;

    Ok(())
}

/// Ensure `/etc/caddy/conf.d/` exists.
fn ensure_conf_dir() -> Result<()> {
    fs::create_dir_all(CONF_DIR).with_context(|| format!("failed to create {CONF_DIR}"))?;
    Ok(())
}

/// Append `import /etc/caddy/conf.d/*` to the main Caddyfile if not already present.
fn write_main_caddyfile() -> Result<()> {
    let import_line = "import /etc/caddy/conf.d/*";

    let content = if Path::new(MAIN_CADDYFILE).exists() {
        fs::read_to_string(MAIN_CADDYFILE).context("failed to read Caddyfile")?
    } else {
        String::new()
    };

    if content.contains(import_line) {
        tracing::debug!("Caddyfile already contains import directive");
        return Ok(());
    }

    tracing::info!("appending import directive to {MAIN_CADDYFILE}");
    let mut new_content = content;
    if !new_content.ends_with('\n') && !new_content.is_empty() {
        new_content.push('\n');
    }
    new_content.push_str(import_line);
    new_content.push('\n');

    fs::write(MAIN_CADDYFILE, new_content).context("failed to write Caddyfile")?;
    Ok(())
}

/// Write the site-specific Caddy config for betcode-releases.
fn write_site_config(domain: &str, acme_email: Option<&str>) -> Result<()> {
    tracing::info!("writing caddy site config: {SITE_CONFIG}");
    let content = templates::caddyfile_site(domain, acme_email);
    fs::write(SITE_CONFIG, content).context("failed to write caddy site config")?;
    Ok(())
}

/// Create `/var/log/caddy/` and the log file with caddy ownership.
fn create_log_dir() -> Result<()> {
    fs::create_dir_all(LOG_DIR).with_context(|| format!("failed to create {LOG_DIR}"))?;
    fs::set_permissions(LOG_DIR, fs::Permissions::from_mode(0o755))
        .context("failed to set permissions on log dir")?;

    // Pre-create the log file so caddy doesn't need to create it at reload time
    if !Path::new(LOG_FILE).exists() {
        fs::File::create(LOG_FILE).with_context(|| format!("failed to create {LOG_FILE}"))?;
    }

    run_cmd(
        "setting ownership on caddy log dir",
        "chown",
        &["-R", "caddy:caddy", LOG_DIR],
    )?;

    Ok(())
}

/// Validate Caddy config and reload.
fn reload_caddy() -> Result<()> {
    run_cmd(
        "validating caddy config",
        "caddy",
        &["validate", "--config", MAIN_CADDYFILE],
    )?;

    run_cmd(
        "reloading caddy",
        "systemctl",
        &["reload-or-restart", "caddy"],
    )?;

    Ok(())
}
