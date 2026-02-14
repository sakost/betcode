use std::path::PathBuf;

use anyhow::Result;
use dialoguer::{Confirm, Input, Password, Select};
use rand::RngExt;

use crate::config::DeploymentMode;

/// Generate a random alphanumeric string of the given length.
fn generate_secret(len: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..len)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

/// Prompt for the relay domain.
pub fn prompt_domain(non_interactive: bool, default: &str) -> Result<String> {
    if non_interactive {
        return Ok(default.to_string());
    }
    let domain: String = Input::new()
        .with_prompt("Domain name for the relay")
        .default(default.to_string())
        .interact_text()?;
    Ok(domain)
}

/// Prompt for the JWT secret — auto-generate or manual entry.
pub fn prompt_jwt_secret(non_interactive: bool) -> Result<String> {
    if non_interactive {
        return Ok(generate_secret(48));
    }

    let auto = Confirm::new()
        .with_prompt("Auto-generate JWT secret? (recommended)")
        .default(true)
        .interact()?;

    if auto {
        let secret = generate_secret(48);
        tracing::info!("generated 48-character JWT secret");
        Ok(secret)
    } else {
        let secret: String = Password::new()
            .with_prompt("Enter JWT secret (min 32 characters)")
            .interact()?;
        if secret.len() < 32 {
            anyhow::bail!("JWT secret must be at least 32 characters");
        }
        Ok(secret)
    }
}

/// Prompt for the database file path.
pub fn prompt_db_path(non_interactive: bool, default: &str) -> Result<PathBuf> {
    if non_interactive {
        return Ok(PathBuf::from(default));
    }
    let path: String = Input::new()
        .with_prompt("Database file path")
        .default(default.to_string())
        .interact_text()?;
    Ok(PathBuf::from(path))
}

/// Prompt for the deployment mode.
pub fn prompt_deployment_mode(non_interactive: bool) -> Result<DeploymentMode> {
    if non_interactive {
        return Ok(DeploymentMode::Docker);
    }

    let items = &["systemd (recommended)", "docker"];
    let selection = Select::new()
        .with_prompt("Deployment mode")
        .items(items)
        .default(0)
        .interact()?;

    Ok(match selection {
        0 => DeploymentMode::Systemd,
        _ => DeploymentMode::Docker,
    })
}

/// Prompt for a relay URL.
pub fn prompt_relay_url(non_interactive: bool, default: &str) -> Result<String> {
    if non_interactive {
        return Ok(default.to_string());
    }
    let url: String = Input::new()
        .with_prompt("Relay URL")
        .default(default.to_string())
        .interact_text()?;
    Ok(url)
}

/// Prompt for a username.
pub fn prompt_username(non_interactive: bool) -> Result<String> {
    if non_interactive {
        anyhow::bail!("--username is required in non-interactive mode");
    }
    let username: String = Input::new().with_prompt("Username").interact_text()?;
    Ok(username)
}

/// Prompt for a password (hidden input).
pub fn prompt_password(non_interactive: bool) -> Result<String> {
    if non_interactive {
        std::env::var("BETCODE_PASSWORD").map_err(|_| {
            anyhow::anyhow!("BETCODE_PASSWORD env var is required in non-interactive mode")
        })
    } else {
        let password: String = Password::new().with_prompt("Password").interact()?;
        Ok(password)
    }
}

/// Prompt for register vs login. Returns `true` for login, `false` for register.
pub fn prompt_auth_action(non_interactive: bool) -> Result<bool> {
    if non_interactive {
        return Ok(true);
    }
    let items = &["Login (existing account)", "Register (new account)"];
    let selection = Select::new()
        .with_prompt("Do you have an account on this relay?")
        .items(items)
        .default(0)
        .interact()?;
    Ok(selection == 0)
}

/// Prompt for a machine name.
pub fn prompt_machine_name(non_interactive: bool, default: &str) -> Result<String> {
    if non_interactive {
        return Ok(default.to_string());
    }
    let name: String = Input::new()
        .with_prompt("Machine name for this computer")
        .default(default.to_string())
        .interact_text()?;
    Ok(name)
}

/// Prompt user to select from a list of items. Returns the selected index.
pub fn prompt_select(prompt_text: &str, items: &[String]) -> Result<usize> {
    let selection = Select::new()
        .with_prompt(prompt_text)
        .items(items)
        .default(0)
        .interact()?;
    Ok(selection)
}
