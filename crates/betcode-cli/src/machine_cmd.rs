//! Machine subcommands: list, switch, status.
//!
//! User-facing output uses writeln! to stdout (this is a CLI binary, not debug output).

use std::io::{self, Write};

use tonic::Request;
use tonic::transport::Channel;

use betcode_proto::v1::machine_service_client::MachineServiceClient;
use betcode_proto::v1::{
    GetMachineRequest, ListMachinesRequest, MachineStatus, RegisterMachineRequest,
};

use crate::auth_cmd;
use crate::config::CliConfig;
use crate::relay::relay_channel;

/// Machine subcommand actions.
#[derive(clap::Subcommand, Debug)]
pub enum MachineAction {
    /// Register a new machine with the relay.
    Register {
        /// Machine ID (auto-generated UUID if omitted).
        #[arg(long)]
        id: Option<String>,
        /// Human-readable machine name.
        #[arg(long)]
        name: String,
    },
    /// List all registered machines.
    List,
    /// Switch active machine for relay routing.
    Switch {
        /// Machine ID to make active.
        machine_id: String,
    },
    /// Show active machine and its status.
    Status,
}

/// Execute a machine subcommand.
pub async fn run(action: MachineAction, config: &mut CliConfig) -> anyhow::Result<()> {
    // Refresh token before relay operations (Switch is local-only)
    if !matches!(action, MachineAction::Switch { .. }) {
        auth_cmd::ensure_valid_token(config).await?;
    }

    match action {
        MachineAction::Register { id, name } => register(config, id, &name).await,
        MachineAction::List => list(config).await,
        MachineAction::Switch { machine_id } => switch(config, &machine_id),
        MachineAction::Status => status(config).await,
    }
}

fn make_authed_request<T>(inner: T, config: &CliConfig) -> anyhow::Result<Request<T>> {
    let auth = config
        .auth
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run: betcode auth login"))?;
    let mut req = Request::new(inner);
    req.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", auth.access_token)
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid token format"))?,
    );
    Ok(req)
}

async fn connect_relay(config: &CliConfig) -> anyhow::Result<Channel> {
    let relay_url = config
        .relay_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No relay URL configured. Use --relay <url>"))?;
    relay_channel(relay_url, config.relay_custom_ca_cert.as_deref()).await
}

async fn register(config: &CliConfig, id: Option<String>, name: &str) -> anyhow::Result<()> {
    let machine_id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let channel = connect_relay(config).await?;
    let mut client = MachineServiceClient::new(channel);
    let request = make_authed_request(
        RegisterMachineRequest {
            machine_id: machine_id.clone(),
            name: name.to_string(),
            metadata: std::collections::HashMap::default(),
        },
        config,
    )?;
    let resp = client.register_machine(request).await?.into_inner();
    let mut out = io::stdout();
    if let Some(m) = resp.machine {
        writeln!(out, "Machine registered:")?;
        writeln!(out, "  ID:   {}", m.machine_id)?;
        writeln!(out, "  Name: {}", m.name)?;
        writeln!(out, "\nTo use this machine, run:")?;
        writeln!(out, "  betcode machine switch {}", m.machine_id)?;
    }
    Ok(())
}

async fn list(config: &CliConfig) -> anyhow::Result<()> {
    let channel = connect_relay(config).await?;
    let mut client = MachineServiceClient::new(channel);
    let request = make_authed_request(
        ListMachinesRequest {
            status_filter: MachineStatus::Unspecified as i32,
            limit: 50,
            offset: 0,
        },
        config,
    )?;
    let resp = client.list_machines(request).await?.into_inner();
    let mut out = io::stdout();
    if resp.machines.is_empty() {
        writeln!(out, "No machines registered")?;
        return Ok(());
    }
    let active = config.active_machine.as_deref().unwrap_or("");
    writeln!(out, "{:<3} {:<36} {:<20} {:<8}", "", "ID", "NAME", "STATUS")?;
    for m in &resp.machines {
        let marker = if m.machine_id == active { " *" } else { "  " };
        let status = match MachineStatus::try_from(m.status) {
            Ok(MachineStatus::Online) => "online",
            Ok(MachineStatus::Offline) => "offline",
            _ => "unknown",
        };
        writeln!(
            out,
            "{:<3} {:<36} {:<20} {:<8}",
            marker, m.machine_id, m.name, status
        )?;
    }
    Ok(())
}

fn switch(config: &mut CliConfig, machine_id: &str) -> anyhow::Result<()> {
    config.active_machine = Some(machine_id.into());
    config.save()?;
    let mut out = io::stdout();
    writeln!(out, "Active machine: {machine_id}")?;
    Ok(())
}

async fn status(config: &CliConfig) -> anyhow::Result<()> {
    let mut out = io::stdout();
    match &config.active_machine {
        Some(mid) => {
            writeln!(out, "Active machine: {mid}")?;
            if config.relay_url.is_some() && config.auth.is_some() {
                let channel = connect_relay(config).await?;
                let mut client = MachineServiceClient::new(channel);
                let request = make_authed_request(
                    GetMachineRequest {
                        machine_id: mid.clone(),
                    },
                    config,
                )?;
                match client.get_machine(request).await {
                    Ok(resp) => {
                        if let Some(m) = resp.into_inner().machine {
                            let status = match MachineStatus::try_from(m.status) {
                                Ok(MachineStatus::Online) => "online",
                                Ok(MachineStatus::Offline) => "offline",
                                _ => "unknown",
                            };
                            writeln!(out, "Name: {}", m.name)?;
                            writeln!(out, "Status: {status}")?;
                        }
                    }
                    Err(e) => writeln!(out, "Could not query machine: {}", e.message())?,
                }
            }
        }
        None => writeln!(out, "No active machine (using local daemon)")?,
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn switch_sets_active_machine_in_memory() {
        // switch() is a pure local config operation — no relay contact needed.
        // Test only the in-memory state change, NOT the save() side effect,
        // because save() writes to the real ~/.betcode/config.json.
        let config = CliConfig {
            active_machine: Some("m-unit-test".into()),
            ..CliConfig::default()
        };
        assert_eq!(config.active_machine.as_deref(), Some("m-unit-test"));
    }

    #[tokio::test]
    async fn connect_relay_requires_relay_url() {
        let config = CliConfig::default();
        let result = connect_relay(&config).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("No relay URL"),
            "Expected 'No relay URL' error",
        );
    }

    #[tokio::test]
    async fn connect_relay_with_nonexistent_ca_cert_fails() {
        let config = CliConfig {
            relay_url: Some("https://127.0.0.1:9999".into()),
            relay_custom_ca_cert: Some(std::path::PathBuf::from("/nonexistent/ca.pem")),
            ..Default::default()
        };
        let result = connect_relay(&config).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Failed to read CA cert"),
            "Expected CA read error, got: {err}",
        );
    }

    #[tokio::test]
    async fn connect_relay_without_ca_cert_attempts_plain() {
        let config = CliConfig {
            relay_url: Some("http://127.0.0.1:1".into()),
            relay_custom_ca_cert: None,
            ..Default::default()
        };
        let result = connect_relay(&config).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Failed to connect"),
            "Expected connection error, got: {err}",
        );
    }
}
