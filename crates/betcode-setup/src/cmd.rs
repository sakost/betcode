use std::process::Command;

use anyhow::{Context, Result, bail};

/// Execute a command with logging. Logs the full command line at debug level
/// and a human-friendly description at info level.
pub fn run_cmd(description: &str, program: &str, args: &[&str]) -> Result<()> {
    let cmd_line = format!("{program} {}", args.join(" "));
    tracing::info!("{description}");
    tracing::debug!("exec: {cmd_line}");

    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to execute: {cmd_line}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("command failed: {cmd_line}\nstderr: {stderr}");
        bail!("{description} failed (exit {}): {stderr}", output.status);
    }
    Ok(())
}

/// Execute a command and return its stdout as a string.
pub fn run_cmd_output(program: &str, args: &[&str]) -> Result<String> {
    let cmd_line = format!("{program} {}", args.join(" "));
    tracing::debug!("exec (capture): {cmd_line}");

    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to execute: {cmd_line}"))?;

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Check whether a program exists on PATH.
pub fn command_exists(program: &str) -> bool {
    Command::new("which")
        .arg(program)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Check whether the port check should be skipped.
///
/// When performing an update and the service is already running, the port is
/// expected to be in use so we skip the availability check.
pub const fn should_skip_port_check(is_update: bool, service_is_active: bool) -> bool {
    is_update && service_is_active
}

/// Create the `betcode` system user if it does not already exist.
pub fn ensure_system_user() -> Result<()> {
    if command_exists("id")
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
