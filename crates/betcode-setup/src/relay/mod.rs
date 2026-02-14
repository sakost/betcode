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

/// Existing relay configuration loaded from installed files.
#[derive(Debug, Default)]
struct ExistingConfig {
    domain: Option<String>,
    jwt_secret: Option<String>,
    db_path: Option<PathBuf>,
}

/// Extract the domain from a systemd unit file's `--tls-cert` path.
fn parse_domain_from_unit(unit_content: &str) -> Option<String> {
    let prefix = "/etc/letsencrypt/live/";
    for line in unit_content.lines() {
        if let Some(pos) = line.find(prefix) {
            let after = &line[pos + prefix.len()..];
            if let Some(end) = after.find('/') {
                return Some(after[..end].to_string());
            }
        }
    }
    None
}

/// Parse JWT secret and DB path from a relay env file.
fn parse_env_file(env_content: &str) -> (Option<String>, Option<PathBuf>) {
    let mut jwt = None;
    let mut db = None;
    for line in env_content.lines() {
        if let Some(val) = line.strip_prefix("BETCODE_JWT_SECRET=") {
            jwt = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("BETCODE_DB_PATH=") {
            db = Some(PathBuf::from(val));
        }
    }
    (jwt, db)
}

/// Try to load configuration from an existing systemd installation.
fn load_existing_config() -> ExistingConfig {
    let mut config = ExistingConfig::default();

    if let Ok(unit) = std::fs::read_to_string(systemd::SERVICE_UNIT_PATH) {
        config.domain = parse_domain_from_unit(&unit);
    }

    if let Ok(env) = std::fs::read_to_string("/etc/betcode/relay.env") {
        let (jwt, db) = parse_env_file(&env);
        config.jwt_secret = jwt;
        config.db_path = db;
    }

    config
}

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

    /// Path to the betcode-releases binary (for deploying the download server).
    #[arg(long)]
    pub releases_binary: Option<PathBuf>,

    /// Domain for the releases download server (e.g. get.betcode.dev).
    /// Required when --releases-binary is provided.
    #[arg(long)]
    pub releases_domain: Option<String>,

    /// GitHub repository for release downloads (used with --releases-binary).
    #[arg(long, default_value = "sakost/betcode")]
    pub github_repo: String,
}

fn parse_mode(s: &str) -> Result<DeploymentMode, String> {
    match s {
        "systemd" => Ok(DeploymentMode::Systemd),
        "docker" => Ok(DeploymentMode::Docker),
        other => Err(format!(
            "unknown mode: {other} (expected 'systemd' or 'docker')"
        )),
    }
}

/// Run the relay setup flow.
pub fn run(args: RelayArgs, non_interactive: bool) -> Result<()> {
    // Detect update mode early so we can reuse existing config.
    let is_update = std::path::Path::new(systemd::SERVICE_UNIT_PATH).exists();
    let existing = if is_update {
        let cfg = load_existing_config();
        tracing::info!("existing installation detected — running in update mode");
        if let Some(ref d) = cfg.domain {
            tracing::info!("detected existing domain: {d}");
        }
        cfg
    } else {
        ExistingConfig::default()
    };

    // CLI args override existing config; existing config skips prompts.
    let domain = match args.domain {
        Some(d) => d,
        None => match existing.domain {
            Some(d) => d,
            None => prompt::prompt_domain(non_interactive, "localhost")?,
        },
    };

    let jwt_secret = match args.jwt_secret {
        Some(s) => s,
        None => match existing.jwt_secret {
            Some(s) => s,
            None => prompt::prompt_jwt_secret(non_interactive)?,
        },
    };

    let db_path = match args.db_path {
        Some(p) => PathBuf::from(p),
        None => match existing.db_path {
            Some(p) => p,
            None => prompt::prompt_db_path(non_interactive, "/var/lib/betcode/relay.db")?,
        },
    };

    let deployment_mode = match args.mode {
        Some(m) => m,
        None if is_update => DeploymentMode::Systemd,
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

    tracing::info!(
        "relay setup: mode={}, domain={}",
        config.deployment_mode,
        config.domain
    );

    match config.deployment_mode {
        DeploymentMode::Systemd => {
            crate::escalate::escalate_if_needed(non_interactive)?;
            validate::check_systemd_prereqs(config.addr, is_update)?;
            systemd::deploy(&config, is_update)?;
        }
        DeploymentMode::Docker => {
            let compose_cmd = validate::detect_compose_command()?;
            docker::generate(&config, &compose_cmd)?;
        }
    }

    // Optionally deploy the releases download server
    if let Some(releases_binary) = args.releases_binary {
        let releases_domain = args
            .releases_domain
            .unwrap_or_else(|| format!("get.{}", config.domain.trim_start_matches("relay.")));
        let releases_binary = std::fs::canonicalize(&releases_binary)
            .with_context(|| format!("releases binary not found: {}", releases_binary.display()))?;
        systemd::deploy_releases(&releases_binary, &releases_domain, &args.github_repo)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_domain_from_unit_extracts_domain() {
        let unit = r"[Service]
ExecStart=/usr/local/bin/betcode-relay \
  --tls-cert /etc/letsencrypt/live/relay.example.com/fullchain.pem \
  --tls-key /etc/letsencrypt/live/relay.example.com/privkey.pem";
        assert_eq!(
            parse_domain_from_unit(unit),
            Some("relay.example.com".to_string())
        );
    }

    #[test]
    fn parse_domain_from_unit_returns_none_for_no_match() {
        let unit = "[Service]\nExecStart=/usr/local/bin/betcode-relay";
        assert_eq!(parse_domain_from_unit(unit), None);
    }

    #[test]
    fn parse_env_file_extracts_values() {
        let env = "BETCODE_JWT_SECRET=mysecret123\nBETCODE_DB_PATH=/var/lib/betcode/relay.db\n";
        let (jwt, db) = parse_env_file(env);
        assert_eq!(jwt, Some("mysecret123".to_string()));
        assert_eq!(db, Some(PathBuf::from("/var/lib/betcode/relay.db")));
    }

    #[test]
    fn parse_env_file_handles_empty() {
        let (jwt, db) = parse_env_file("");
        assert_eq!(jwt, None);
        assert_eq!(db, None);
    }
}
