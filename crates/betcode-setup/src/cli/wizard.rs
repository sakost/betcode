use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::cmd::command_exists;
use crate::prompt;

use super::CliArgs;

/// Locate the betcode CLI binary.
pub fn find_betcode_binary(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        let resolved = std::fs::canonicalize(path)
            .with_context(|| format!("betcode binary not found: {}", path.display()))?;
        return Ok(resolved);
    }
    if command_exists("betcode") {
        return Ok(PathBuf::from("betcode"));
    }
    bail!("betcode CLI not found on PATH. Install it first, or use --cli-binary <path>");
}

/// Run the interactive CLI setup wizard.
pub fn run_wizard(betcode: &Path, args: &CliArgs, non_interactive: bool) -> Result<()> {
    // Step 1: Relay URL
    let relay_url = match &args.relay {
        Some(url) => url.clone(),
        None => prompt::prompt_relay_url(non_interactive, "https://relay.example.com:443")?,
    };
    tracing::info!("relay URL: {relay_url}");

    // Step 2: Auth — register or login
    let username = match &args.username {
        Some(u) => u.clone(),
        None => prompt::prompt_username(non_interactive)?,
    };
    let password = prompt::prompt_password(non_interactive)?;

    let is_login = if args.login {
        true
    } else if args.register {
        false
    } else {
        prompt::prompt_auth_action(non_interactive)?
    };

    if is_login {
        run_betcode_auth(betcode, &relay_url, "login", &username, &password)?;
    } else {
        run_betcode_auth(betcode, &relay_url, "register", &username, &password)?;
    }

    // Step 3: Machine setup
    let machine_name = if let Some(n) = &args.machine_name {
        n.clone()
    } else {
        let hostname = gethostname();
        prompt::prompt_machine_name(non_interactive, &hostname)?
    };

    let machine_id = run_betcode_machine_register(betcode, &relay_url, &machine_name)?;
    run_betcode_machine_switch(betcode, &machine_id)?;

    #[allow(clippy::print_stdout)]
    {
        println!();
        println!("CLI setup complete!");
        println!();
        println!("  Relay:   {relay_url}");
        println!("  User:    {username}");
        println!("  Machine: {machine_name} ({machine_id})");
        println!();
        println!("You can now use betcode in relay mode. Try:");
        println!("  betcode auth status");
    }

    Ok(())
}

/// Run `betcode auth login` or `betcode auth register`.
fn run_betcode_auth(
    betcode: &Path,
    relay_url: &str,
    action: &str,
    username: &str,
    password: &str,
) -> Result<()> {
    tracing::info!("{action}ing as {username}");

    let output = Command::new(betcode)
        .args(["--relay", relay_url, "auth", action, "-u", username])
        .env("BETCODE_PASSWORD", password)
        .output()
        .with_context(|| format!("failed to run betcode auth {action}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!("auth {action} failed: {stdout}{stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    tracing::info!("{}", stdout.trim());
    Ok(())
}

/// Run `betcode machine register` and extract the machine ID from output.
fn run_betcode_machine_register(betcode: &Path, relay_url: &str, name: &str) -> Result<String> {
    tracing::info!("registering machine: {name}");

    let output = Command::new(betcode)
        .args(["--relay", relay_url, "machine", "register", "--name", name])
        .output()
        .context("failed to run betcode machine register")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!("machine register failed: {stdout}{stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Parse "  ID:   <uuid>" from output
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(id) = trimmed.strip_prefix("ID:") {
            return Ok(id.trim().to_string());
        }
    }

    bail!("could not parse machine ID from output:\n{stdout}");
}

/// Run `betcode machine switch <id>`.
fn run_betcode_machine_switch(betcode: &Path, machine_id: &str) -> Result<()> {
    tracing::info!("switching to machine: {machine_id}");

    let output = Command::new(betcode)
        .args(["machine", "switch", machine_id])
        .output()
        .context("failed to run betcode machine switch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("machine switch failed: {stderr}");
    }
    Ok(())
}

/// Get the system hostname for the default machine name.
fn gethostname() -> String {
    crate::cmd::run_cmd_output("hostname", &[]).unwrap_or_else(|_| "my-machine".to_string())
}
