//! Auth subcommands: login, logout, status.
//!
//! User-facing output uses writeln! to stdout (this is a CLI binary, not debug output).

use std::io::{self, Write};

use tonic::transport::Channel;

use betcode_proto::v1::auth_service_client::AuthServiceClient;
use betcode_proto::v1::{LoginRequest, RevokeTokenRequest};

use crate::config::{AuthConfig, CliConfig};

/// Auth subcommand actions.
#[derive(clap::Subcommand, Debug)]
pub enum AuthAction {
    /// Log in to a relay server.
    Login {
        /// Username.
        #[arg(short, long)]
        username: String,
        /// Password.
        #[arg(short, long)]
        password: String,
    },
    /// Log out and revoke tokens.
    Logout,
    /// Show current auth status.
    Status,
}

/// Execute an auth subcommand.
pub async fn run(action: AuthAction, config: &mut CliConfig) -> anyhow::Result<()> {
    match action {
        AuthAction::Login { username, password } => login(config, &username, &password).await,
        AuthAction::Logout => logout(config).await,
        AuthAction::Status => {
            status(config);
            Ok(())
        }
    }
}

async fn login(config: &mut CliConfig, username: &str, password: &str) -> anyhow::Result<()> {
    let relay_url = config
        .relay_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No relay URL configured. Use --relay <url>"))?
        .clone();

    let channel = Channel::from_shared(relay_url)?
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to relay: {}", e))?;

    let mut client = AuthServiceClient::new(channel);
    let resp = client
        .login(LoginRequest {
            username: username.into(),
            password: password.into(),
        })
        .await
        .map_err(|e| anyhow::anyhow!("Login failed: {}", e.message()))?
        .into_inner();

    config.auth = Some(AuthConfig {
        user_id: resp.user_id,
        username: username.into(),
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
    });
    config.save()?;

    let mut out = io::stdout();
    writeln!(out, "Logged in as {}", username)?;
    Ok(())
}

async fn logout(config: &mut CliConfig) -> anyhow::Result<()> {
    if let (Some(auth), Some(relay_url)) = (&config.auth, &config.relay_url) {
        if let Ok(channel) = Channel::from_shared(relay_url.clone())
            .ok()
            .unwrap()
            .connect()
            .await
        {
            let mut client = AuthServiceClient::new(channel);
            let _ = client
                .revoke_token(RevokeTokenRequest {
                    refresh_token: auth.refresh_token.clone(),
                })
                .await;
        }
    }
    config.clear_auth();
    config.save()?;
    let mut out = io::stdout();
    writeln!(out, "Logged out")?;
    Ok(())
}

fn status(config: &CliConfig) {
    let mut out = io::stdout();
    match &config.auth {
        Some(auth) => {
            let _ = writeln!(out, "Logged in as: {}", auth.username);
            let _ = writeln!(out, "User ID: {}", auth.user_id);
            if let Some(url) = &config.relay_url {
                let _ = writeln!(out, "Relay: {}", url);
            }
        }
        None => {
            let _ = writeln!(out, "Not logged in");
        }
    }
}
