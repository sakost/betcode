use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use betcode_setup::cli::CliArgs;
#[cfg(unix)]
use betcode_setup::daemon::DaemonArgs;
use betcode_setup::relay::RelayArgs;
#[cfg(feature = "releases")]
use betcode_setup::releases::ReleasesArgs;

/// `BetCode` deployment setup tool.
#[derive(Debug, Parser)]
#[command(name = "betcode-setup", version, about)]
struct Cli {
    /// Run without interactive prompts (use defaults or CLI flags)
    #[arg(long, global = true)]
    non_interactive: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Set up the betcode relay server
    Relay(RelayArgs),
    /// Set up the betcode daemon as a systemd service
    #[cfg(unix)]
    Daemon(DaemonArgs),
    /// Set up the betcode CLI for relay access
    Cli(CliArgs),
    /// Set up the betcode releases download server
    #[cfg(feature = "releases")]
    Releases(ReleasesArgs),
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Relay(args) => {
            betcode_setup::os::ensure_ubuntu()?;
            betcode_setup::relay::run(args, cli.non_interactive)?;
        }
        #[cfg(unix)]
        Commands::Daemon(args) => {
            betcode_setup::daemon::run(args, cli.non_interactive)?;
        }
        Commands::Cli(ref args) => betcode_setup::cli::run(args, cli.non_interactive)?,
        #[cfg(feature = "releases")]
        Commands::Releases(args) => {
            betcode_setup::os::ensure_ubuntu()?;
            betcode_setup::releases::run(args, cli.non_interactive)?;
        }
    }

    Ok(())
}
