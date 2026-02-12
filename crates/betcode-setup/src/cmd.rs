use std::process::Command;

use anyhow::{bail, Context, Result};

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
