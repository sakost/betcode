mod docker;
mod systemd;
mod templates;
mod validate;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;

use crate::config::{DeploymentMode, RelaySetupConfig};
use crate::prompt;

/// Arguments for the `relay` subcommand.
#[derive(Debug, Args)]
pub struct RelayArgs {
    /// Domain name for the relay (e.g. relay.example.com)
    #[arg(long)]
    pub domain: Option<String>,

    /// JWT secret for authentication (min 32 chars, auto-generated if omitted)
    #[arg(long)]
    pub jwt_secret: Option<String>,

    /// Path to the database file
    #[arg(long)]
    pub db_path: Option<String>,

    /// Deployment mode
    #[arg(long, value_parser = parse_mode)]
    pub mode: Option<DeploymentMode>,

    /// Path to the betcode-relay binary (for systemd mode).
    /// If omitted, uses the existing binary at /usr/local/bin/betcode-relay.
    #[arg(long)]
    pub relay_binary: Option<PathBuf>,

    /// Listen address for the relay server.
    #[arg(long, default_value = "0.0.0.0:443")]
    pub addr: SocketAddr,
}

fn parse_mode(s: &str) -> Result<DeploymentMode, String> {
    match s {
        "systemd" => Ok(DeploymentMode::Systemd),
        "docker" => Ok(DeploymentMode::Docker),
        other => Err(format!("unknown mode: {other} (expected 'systemd' or 'docker')")),
    }
}

/// Run the relay setup flow.
pub fn run(args: RelayArgs, non_interactive: bool) -> Result<()> {
    let domain = match args.domain {
        Some(d) => d,
        None => prompt::prompt_domain(non_interactive, "localhost")?,
    };

    let jwt_secret = match args.jwt_secret {
        Some(s) => s,
        None => prompt::prompt_jwt_secret(non_interactive)?,
    };

    let db_path = match args.db_path {
        Some(p) => PathBuf::from(p),
        None => prompt::prompt_db_path(non_interactive, "/var/lib/betcode/relay.db")?,
    };

    let deployment_mode = match args.mode {
        Some(m) => m,
        None => prompt::prompt_deployment_mode(non_interactive)?,
    };

    let config = RelaySetupConfig {
        domain,
        jwt_secret,
        db_path,
        deployment_mode,
        relay_binary_path: args
            .relay_binary
            .map(|p| {
                std::fs::canonicalize(&p)
                    .with_context(|| format!("relay binary not found: {}", p.display()))
            })
            .transpose()?,
        addr: args.addr,
    };

    config.validate()?;

    tracing::info!("relay setup: mode={}, domain={}", config.deployment_mode, config.domain);

    match config.deployment_mode {
        DeploymentMode::Systemd => {
            crate::escalate::escalate_if_needed(non_interactive)?;
            let is_update = std::path::Path::new(systemd::SERVICE_UNIT_PATH).exists();
            if is_update {
                tracing::info!("existing installation detected — running in update mode");
            }
            validate::check_systemd_prereqs(config.addr, is_update)?;
            systemd::deploy(&config, is_update)?;
        }
        DeploymentMode::Docker => {
            let compose_cmd = validate::detect_compose_command()?;
            docker::generate(&config, &compose_cmd)?;
        }
    }

    Ok(())
}
