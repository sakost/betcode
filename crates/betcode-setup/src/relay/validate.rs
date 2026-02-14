use std::net::{SocketAddr, TcpListener};

use anyhow::{bail, Result};

use crate::cmd::{command_exists, should_skip_port_check};

/// Check whether the betcode-relay systemd service is currently active.
pub fn is_betcode_relay_active() -> bool {
    std::process::Command::new("systemctl")
        .args(["is-active", "--quiet", "betcode-relay"])
        .status()
        .is_ok_and(|s| s.success())
}

/// Check systemd prerequisites: systemctl present and configured port is free.
pub fn check_systemd_prereqs(addr: SocketAddr, is_update: bool) -> Result<()> {
    if !command_exists("systemctl") {
        bail!("systemctl not found — systemd mode requires a systemd-based system");
    }

    if should_skip_port_check(is_update, is_betcode_relay_active()) {
        tracing::info!("betcode-relay is already active — skipping port check");
        return Ok(());
    }

    match TcpListener::bind(addr) {
        Ok(_listener) => {
            tracing::debug!("port {} is available", addr.port());
        }
        Err(e) => {
            bail!(
                "port {} is already in use ({e}). \
                 Stop the service occupying it before running setup.",
                addr.port()
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
