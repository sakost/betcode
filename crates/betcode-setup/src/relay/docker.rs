use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::RelaySetupConfig;

use super::templates;

/// Generate Docker deployment files in the current directory.
pub fn generate(config: &RelaySetupConfig, compose_cmd: &[String]) -> Result<()> {
    write_with_backup("Dockerfile", templates::dockerfile())?;
    write_with_backup("docker-compose.yml", &templates::docker_compose(config))?;
    write_with_backup(".env.example", &templates::env_example(config))?;

    let compose = compose_cmd.join(" ");

    #[allow(clippy::print_stdout)]
    {
        println!("\nDocker deployment files generated successfully!");
        println!();
        println!("Next steps:");
        println!("  1. Copy .env.example to .env and review the values:");
        println!("     cp .env.example .env");
        println!();
        println!("  2. Obtain TLS certificates (first time only):");
        println!("     {compose} --profile init up certbot-init");
        println!();
        println!("  3. Start the relay:");
        println!("     {compose} up -d relay");
        println!();
        println!("  4. View logs:");
        println!("     {compose} logs -f relay");
    }

    Ok(())
}

/// Write content to a file, backing up any existing file first.
fn write_with_backup(filename: &str, content: &str) -> Result<()> {
    let path = Path::new(filename);
    if path.exists() {
        let backup = format!("{filename}.bak");
        tracing::info!("backing up existing {filename} -> {backup}");
        fs::rename(path, &backup).with_context(|| format!("failed to backup {filename}"))?;
    }

    tracing::info!("writing {filename}");
    fs::write(path, content).with_context(|| format!("failed to write {filename}"))?;

    Ok(())
}
