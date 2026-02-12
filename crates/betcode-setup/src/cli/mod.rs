mod wizard;

use std::path::PathBuf;

use anyhow::Result;
use clap::Args;

/// Arguments for the `cli` subcommand.
#[derive(Debug, Args)]
pub struct CliArgs {
    /// Relay URL (e.g. `https://relay.example.com:443`)
    #[arg(long)]
    pub relay: Option<String>,

    /// Path to the betcode CLI binary (default: search PATH)
    #[arg(long)]
    pub cli_binary: Option<PathBuf>,

    /// Username for authentication
    #[arg(long)]
    pub username: Option<String>,

    /// Machine name for this computer
    #[arg(long)]
    pub machine_name: Option<String>,

    /// Register a new account (instead of login)
    #[arg(long, conflicts_with = "login")]
    pub register: bool,

    /// Login to existing account (instead of register)
    #[arg(long, conflicts_with = "register")]
    pub login: bool,
}

/// Run the CLI setup wizard.
pub fn run(args: &CliArgs, non_interactive: bool) -> Result<()> {
    let betcode = wizard::find_betcode_binary(args.cli_binary.as_deref())?;
    wizard::run_wizard(&betcode, args, non_interactive)
}
