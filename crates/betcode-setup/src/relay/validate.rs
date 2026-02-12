use std::net::TcpListener;

use anyhow::{bail, Result};

use crate::cmd::command_exists;

/// Check systemd prerequisites: systemctl present and port 443 is free.
pub fn check_systemd_prereqs() -> Result<()> {
    if !command_exists("systemctl") {
        bail!("systemctl not found — systemd mode requires a systemd-based system");
    }

    // Check if port 443 is available
    match TcpListener::bind("0.0.0.0:443") {
        Ok(_listener) => {
            // Port is free — listener drops and releases it
            tracing::debug!("port 443 is available");
        }
        Err(e) => {
            bail!(
                "port 443 is already in use ({e}). \
                 Stop the service occupying it before running systemd setup."
            );
        }
    }

    Ok(())
}

/// Detect a container compose command. Checks in order:
/// `docker compose`, `docker-compose`, `podman compose`.
/// Returns the full command as a vector of strings.
pub fn detect_compose_command() -> Result<Vec<String>> {
    // Try `docker compose` (v2 plugin)
    if command_exists("docker") {
        let output = std::process::Command::new("docker")
            .args(["compose", "version"])
            .output();
        if output.is_ok_and(|o| o.status.success()) {
            tracing::info!("detected: docker compose (v2 plugin)");
            return Ok(vec!["docker".into(), "compose".into()]);
        }
    }

    // Try `docker-compose` (standalone v1/v2)
    if command_exists("docker-compose") {
        tracing::info!("detected: docker-compose (standalone)");
        return Ok(vec!["docker-compose".into()]);
    }

    // Try `podman compose`
    if command_exists("podman") {
        let output = std::process::Command::new("podman")
            .args(["compose", "version"])
            .output();
        if output.is_ok_and(|o| o.status.success()) {
            tracing::info!("detected: podman compose");
            return Ok(vec!["podman".into(), "compose".into()]);
        }
    }

    bail!(
        "no container compose command found. \
         Install Docker (with compose plugin) or Podman (with podman-compose)."
    );
}
