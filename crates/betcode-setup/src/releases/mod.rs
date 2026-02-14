mod caddy;
mod systemd;
mod templates;
mod validate;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;

use crate::prompt;

/// Arguments for the `releases` subcommand.
#[derive(Debug, Args)]
pub struct ReleasesArgs {
    /// Domain name for the releases server (e.g. get.betcode.dev)
    #[arg(long)]
    pub domain: Option<String>,

    /// Path to the betcode-releases binary.
    /// If omitted, uses the existing binary at /usr/local/bin/betcode-releases.
    #[arg(long)]
    pub releases_binary: Option<PathBuf>,

    /// GitHub repository for release downloads.
    #[arg(long, default_value = "sakost/betcode")]
    pub github_repo: String,

    /// Skip Caddy reverse proxy setup (expose on port 8090 directly).
    #[arg(long)]
    pub no_caddy: bool,

    /// Email address for ACME/Let's Encrypt (used by Caddy).
    #[arg(long)]
    pub acme_email: Option<String>,
}

/// Run the releases setup flow.
pub fn run(args: ReleasesArgs, non_interactive: bool) -> Result<()> {
    let is_update = std::path::Path::new(systemd::SERVICE_UNIT_PATH).exists();
    if is_update {
        tracing::info!("existing releases installation detected — running in update mode");
    }

    let domain = match args.domain {
        Some(d) => d,
        None => prompt::prompt_domain(non_interactive, "get.betcode.dev")?,
    };

    let binary = args
        .releases_binary
        .map(|p| {
            std::fs::canonicalize(&p)
                .with_context(|| format!("releases binary not found: {}", p.display()))
        })
        .transpose()?;

    crate::escalate::escalate_if_needed(non_interactive)?;

    validate::check_prereqs(is_update)?;

    let use_caddy = !args.no_caddy;
    systemd::deploy(
        binary.as_deref(),
        &domain,
        &args.github_repo,
        is_update,
        use_caddy,
    )?;

    if use_caddy {
        caddy::setup(&domain, args.acme_email.as_deref())?;
    }

    Ok(())
}
