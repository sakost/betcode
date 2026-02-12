use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use betcode_setup::relay::RelayArgs;

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
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    betcode_setup::os::ensure_ubuntu()?;

    match cli.command {
        Commands::Relay(args) => betcode_setup::relay::run(args, cli.non_interactive)?,
    }

    Ok(())
}
