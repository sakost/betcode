use std::net::{SocketAddr, TcpListener};

use anyhow::{bail, Result};

use crate::cmd::command_exists;

/// Check whether the port check should be skipped.
const fn should_skip_port_check(is_update: bool, service_is_active: bool) -> bool {
    is_update && service_is_active
}

/// Check whether the betcode-releases systemd service is currently active.
pub fn is_releases_active() -> bool {
    std::process::Command::new("systemctl")
        .args(["is-active", "--quiet", "betcode-releases"])
        .status()
        .is_ok_and(|s| s.success())
}

/// Check prerequisites: systemctl present and port 8090 is free.
pub fn check_prereqs(is_update: bool) -> Result<()> {
    if !command_exists("systemctl") {
        bail!("systemctl not found — releases setup requires a systemd-based system");
    }

    let addr: SocketAddr = SocketAddr::from(([0, 0, 0, 0], 8090));

    if should_skip_port_check(is_update, is_releases_active()) {
        tracing::info!("betcode-releases is already active — skipping port check");
        return Ok(());
    }

    match TcpListener::bind(addr) {
        Ok(_listener) => {
            tracing::debug!("port 8090 is available");
        }
        Err(e) => {
            bail!(
                "port 8090 is already in use ({e}). \
                 Stop the service occupying it before running setup."
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_port_check_when_update_and_service_active() {
        assert!(should_skip_port_check(true, true));
    }

    #[test]
    fn no_skip_port_check_on_fresh_install() {
        assert!(!should_skip_port_check(false, false));
        assert!(!should_skip_port_check(false, true));
    }

    #[test]
    fn no_skip_port_check_when_update_but_service_inactive() {
        assert!(!should_skip_port_check(true, false));
    }
}
