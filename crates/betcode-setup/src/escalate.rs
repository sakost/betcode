use std::env;
use std::process::Command;

use anyhow::{Result, bail};
use dialoguer::Confirm;
use nix::unistd::geteuid;

/// Check if the current process is running as root.
pub fn is_root() -> bool {
    geteuid().is_root()
}

/// If not root, prompt the user and re-exec via sudo.
/// This replaces the current process — it does not return on success.
pub fn escalate_if_needed(non_interactive: bool) -> Result<()> {
    if is_root() {
        return Ok(());
    }

    if non_interactive {
        bail!(
            "systemd setup requires root privileges. \
             Re-run with sudo or use --mode docker for unprivileged file generation."
        );
    }

    let confirmed = Confirm::new()
        .with_prompt("Systemd setup requires root privileges. Re-run with sudo?")
        .default(true)
        .interact()?;

    if !confirmed {
        bail!(
            "root privileges declined. \
             Use --mode docker for unprivileged file generation, \
             or re-run manually with sudo."
        );
    }

    let exe = env::current_exe()?;
    let args: Vec<String> = env::args().collect();

    tracing::info!("re-executing with sudo");
    tracing::debug!("exec: sudo {} {}", exe.display(), args[1..].join(" "));

    let status = Command::new("sudo").arg(exe).args(&args[1..]).status()?;

    // sudo process completed — exit with its code
    std::process::exit(status.code().unwrap_or(1));
}
