mod systemd;
pub(crate) mod templates;

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use dialoguer::Confirm;

use crate::config::{DaemonMode, DaemonSetupConfig};

/// Try to load the machine ID from an existing daemon env file during updates.
fn load_existing_machine_id(is_update: bool, mode: DaemonMode) -> Option<String> {
    if !is_update {
        return None;
    }
    let path = match mode {
        DaemonMode::System => PathBuf::from(systemd::SYSTEM_ENV_PATH),
        DaemonMode::User => systemd::user_env_path().ok()?,
    };
    let content = std::fs::read_to_string(path).ok()?;
    parse_daemon_env_machine_id(&content)
}

/// Extract `BETCODE_MACHINE_ID` from env file content.
fn parse_daemon_env_machine_id(env_content: &str) -> Option<String> {
    for line in env_content.lines() {
        if let Some(val) = line.strip_prefix("BETCODE_MACHINE_ID=")
            && !val.is_empty()
        {
            return Some(val.to_string());
        }
    }
    None
}

/// Arguments for the `daemon` subcommand.
#[derive(Debug, Args)]
pub struct DaemonArgs {
    /// Deployment mode: "system" or "user"
    #[arg(long, default_value = "user", value_parser = parse_mode)]
    pub mode: DaemonMode,

    /// System user to run daemon as (system mode only)
    #[arg(long, default_value = "betcode")]
    pub user: String,

    /// gRPC listen address
    #[arg(long, default_value = "127.0.0.1:50051")]
    pub addr: SocketAddr,

    /// Database path
    #[arg(long)]
    pub db_path: Option<PathBuf>,

    /// Max concurrent Claude processes
    #[arg(long, default_value_t = 5)]
    pub max_processes: usize,

    /// Max concurrent sessions
    #[arg(long, default_value_t = 10)]
    pub max_sessions: usize,

    /// Relay URL for tunnel mode
    #[arg(long)]
    pub relay_url: Option<String>,

    /// Machine ID for relay
    #[arg(long)]
    pub machine_id: Option<String>,

    /// Machine name
    #[arg(long, default_value = "betcode-daemon")]
    pub machine_name: String,

    /// Relay username
    #[arg(long)]
    pub relay_username: Option<String>,

    /// Relay password
    #[arg(long)]
    pub relay_password: Option<String>,

    /// Custom CA cert for relay TLS verification
    #[arg(long)]
    pub relay_custom_ca_cert: Option<PathBuf>,

    /// Worktree base directory
    #[arg(long)]
    pub worktree_dir: Option<PathBuf>,

    /// Path to daemon binary (uses PATH lookup if omitted)
    #[arg(long)]
    pub daemon_binary: Option<PathBuf>,
}

fn parse_mode(s: &str) -> Result<DaemonMode, String> {
    match s {
        "system" => Ok(DaemonMode::System),
        "user" => Ok(DaemonMode::User),
        other => Err(format!(
            "unknown mode: {other} (expected 'system' or 'user')"
        )),
    }
}

fn default_db_path(mode: DaemonMode) -> Result<PathBuf> {
    match mode {
        DaemonMode::System => Ok(PathBuf::from("/var/lib/betcode/daemon.db")),
        DaemonMode::User => {
            let home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
            Ok(home.join(".local/share/betcode/daemon.db"))
        }
    }
}

/// Run the daemon setup flow.
pub fn run(args: DaemonArgs, non_interactive: bool) -> Result<()> {
    let is_update = match args.mode {
        DaemonMode::System => std::path::Path::new(systemd::SYSTEM_UNIT_PATH).exists(),
        DaemonMode::User => systemd::user_unit_path()
            .map(|p| p.exists())
            .unwrap_or(false),
    };

    if is_update {
        tracing::info!("existing daemon installation detected — running in update mode");
    }

    let db_path = match args.db_path {
        Some(p) => p,
        None => default_db_path(args.mode)?,
    };

    let daemon_binary_path = args
        .daemon_binary
        .map(|p| {
            std::fs::canonicalize(&p)
                .with_context(|| format!("daemon binary not found: {}", p.display()))
        })
        .transpose()?;

    let enable_linger = if args.mode == DaemonMode::User {
        prompt_linger(non_interactive)?
    } else {
        false
    };

    let enable_service = if is_update {
        true
    } else {
        prompt_enable_service(non_interactive)?
    };

    let machine_id = args
        .machine_id
        .or_else(|| load_existing_machine_id(is_update, args.mode))
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let config = DaemonSetupConfig {
        mode: args.mode,
        user: args.user,
        addr: args.addr,
        db_path,
        max_processes: args.max_processes,
        max_sessions: args.max_sessions,
        relay_url: args.relay_url,
        machine_id,
        machine_name: args.machine_name,
        relay_username: args.relay_username,
        relay_password: args.relay_password,
        relay_custom_ca_cert: args.relay_custom_ca_cert,
        worktree_dir: args.worktree_dir,
        claude_bin: None,
        daemon_binary_path,
        enable_linger,
        enable_service,
    };

    tracing::info!("daemon setup: mode={}, addr={}", config.mode, config.addr);

    // Provision mTLS client certificate when relay is configured
    if config.relay_url.is_some() {
        provision_mtls_certs(&config.machine_id, non_interactive)?;
    }

    match config.mode {
        DaemonMode::System => {
            crate::escalate::escalate_if_needed(non_interactive)?;
            validate_systemd_prereqs(config.addr, is_update, DaemonMode::System)?;
            systemd::deploy_system(&config, is_update)?;
        }
        DaemonMode::User => {
            validate_systemd_prereqs(config.addr, is_update, DaemonMode::User)?;
            systemd::deploy_user(&config, is_update)?;
        }
    }

    Ok(())
}

fn validate_systemd_prereqs(addr: SocketAddr, is_update: bool, mode: DaemonMode) -> Result<()> {
    if !crate::cmd::command_exists("systemctl") {
        anyhow::bail!("systemctl not found — daemon setup requires a systemd-based system");
    }

    let is_active = match mode {
        DaemonMode::System => systemd::is_daemon_active_system(),
        DaemonMode::User => systemd::is_daemon_active_user(),
    };

    if crate::cmd::should_skip_port_check(is_update, is_active) {
        tracing::info!("betcode-daemon is already active — skipping port check");
        return Ok(());
    }

    match std::net::TcpListener::bind(addr) {
        Ok(_) => tracing::debug!("port {} is available", addr.port()),
        Err(e) => anyhow::bail!(
            "port {} is already in use ({e}). Stop the service occupying it before running setup.",
            addr.port()
        ),
    }

    Ok(())
}

fn prompt_enable_service(non_interactive: bool) -> Result<bool> {
    if non_interactive {
        return Ok(true);
    }
    let confirmed = Confirm::new()
        .with_prompt(
            "Enable and start the daemon service now? \
             (you can do this later with `systemctl enable --now betcode-daemon`)",
        )
        .default(true)
        .interact()?;
    Ok(confirmed)
}

/// Provision mTLS client certificates for relay connections.
///
/// If certificates already exist, prompts the user (or skips in non-interactive
/// mode) before overwriting.
fn provision_mtls_certs(machine_id: &str, non_interactive: bool) -> Result<()> {
    let certs_dir = crate::cert_provisioning::default_certs_dir()?;

    if crate::cert_provisioning::certs_exist(&certs_dir) {
        let should_overwrite = if non_interactive {
            false
        } else {
            Confirm::new()
                .with_prompt(
                    "mTLS client certificates already exist. Regenerate them? \
                     (existing certificates will be overwritten)",
                )
                .default(false)
                .interact()?
        };

        if !should_overwrite {
            tracing::info!("Keeping existing mTLS certificates");
            return Ok(());
        }
    }

    let paths = crate::cert_provisioning::provision_client_cert(&certs_dir, machine_id)?;

    #[allow(clippy::print_stdout)]
    {
        println!();
        println!("  mTLS certificates provisioned:");
        println!("    Client cert: {}", paths.client_cert.display());
        println!("    Client key:  {}", paths.client_key.display());
        println!("    CA cert:     {}", paths.ca_cert.display());
        println!();
    }

    Ok(())
}

fn prompt_linger(non_interactive: bool) -> Result<bool> {
    if non_interactive {
        return Ok(false);
    }
    let confirmed = Confirm::new()
        .with_prompt(
            "Enable lingering? This keeps the daemon running after you log out \
             (recommended for headless/CI machines, optional for workstations)",
        )
        .default(false)
        .interact()?;
    Ok(confirmed)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) fn make_test_daemon_config(mode: DaemonMode) -> DaemonSetupConfig {
    DaemonSetupConfig {
        mode,
        user: "betcode".into(),
        addr: "127.0.0.1:50051".parse::<SocketAddr>().expect("valid addr"),
        db_path: PathBuf::from("/var/lib/betcode/daemon.db"),
        max_processes: 5,
        max_sessions: 10,
        relay_url: None,
        machine_id: "test-machine-id".into(),
        machine_name: "betcode-daemon".into(),
        relay_username: None,
        relay_password: None,
        relay_custom_ca_cert: None,
        worktree_dir: None,
        claude_bin: None,
        daemon_binary_path: None,
        enable_linger: false,
        enable_service: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_system() {
        assert_eq!(parse_mode("system"), Ok(DaemonMode::System));
    }

    #[test]
    fn parse_mode_user() {
        assert_eq!(parse_mode("user"), Ok(DaemonMode::User));
    }

    #[test]
    fn parse_mode_invalid() {
        assert!(parse_mode("docker").is_err());
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn default_db_path_system() {
        let path = default_db_path(DaemonMode::System).expect("should resolve");
        assert_eq!(path, PathBuf::from("/var/lib/betcode/daemon.db"));
    }

    #[test]
    fn parse_daemon_env_machine_id_extracts_value() {
        let env =
            "BETCODE_ADDR=127.0.0.1:50051\nBETCODE_MACHINE_ID=abc-123\nBETCODE_LOG_JSON=true\n";
        assert_eq!(
            parse_daemon_env_machine_id(env),
            Some("abc-123".to_string())
        );
    }

    #[test]
    fn parse_daemon_env_machine_id_returns_none_when_missing() {
        let env = "BETCODE_ADDR=127.0.0.1:50051\nBETCODE_LOG_JSON=true\n";
        assert_eq!(parse_daemon_env_machine_id(env), None);
    }

    #[test]
    fn parse_daemon_env_machine_id_ignores_empty_value() {
        let env = "BETCODE_MACHINE_ID=\nBETCODE_LOG_JSON=true\n";
        assert_eq!(parse_daemon_env_machine_id(env), None);
    }

    #[test]
    #[allow(clippy::expect_used)]
    fn default_db_path_user_is_under_home() {
        let path = default_db_path(DaemonMode::User).expect("should resolve");
        assert!(
            path.to_string_lossy().contains(".local/share/betcode"),
            "user db path should be under ~/.local/share/betcode, got: {}",
            path.display()
        );
    }
}
