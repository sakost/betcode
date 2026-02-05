//! Machine subcommands: list, switch, status.
//!
//! User-facing output uses writeln! to stdout (this is a CLI binary, not debug output).

use std::io::{self, Write};

use tonic::transport::Channel;
use tonic::Request;

use betcode_proto::v1::machine_service_client::MachineServiceClient;
use betcode_proto::v1::{GetMachineRequest, ListMachinesRequest, MachineStatus};

use crate::config::CliConfig;

/// Machine subcommand actions.
#[derive(clap::Subcommand, Debug)]
pub enum MachineAction {
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
    match action {
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
    Channel::from_shared(relay_url.clone())?
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to relay: {}", e))
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
    writeln!(out, "Active machine: {}", machine_id)?;
    Ok(())
}

async fn status(config: &CliConfig) -> anyhow::Result<()> {
    let mut out = io::stdout();
    match &config.active_machine {
        Some(mid) => {
            writeln!(out, "Active machine: {}", mid)?;
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
                            writeln!(out, "Status: {}", status)?;
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
