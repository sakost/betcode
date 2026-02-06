//! Auth subcommands: login, logout, status.
//!
//! User-facing output uses writeln! to stdout (this is a CLI binary, not debug output).

use std::io::{self, Write};

use tonic::transport::Channel;

use betcode_proto::v1::auth_service_client::AuthServiceClient;
use betcode_proto::v1::{LoginRequest, RefreshTokenRequest, RevokeTokenRequest};

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
        AuthAction::Status => status(config).await,
    }
}

/// Refresh the access token via the relay's RefreshToken RPC.
///
/// Connects to the relay, exchanges the current refresh token for new
/// access + refresh tokens, and saves the updated config to disk.
pub async fn ensure_valid_token(config: &mut CliConfig) -> anyhow::Result<()> {
    let relay_url = config
        .relay_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No relay URL configured"))?
        .clone();

    let auth = config
        .auth
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not logged in"))?;

    let refresh_token = auth.refresh_token.clone();

    let channel = Channel::from_shared(relay_url)?
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("Cannot reach relay: {}", e))?;

    let mut client = AuthServiceClient::new(channel);
    let resp = client
        .refresh_token(RefreshTokenRequest { refresh_token })
        .await
        .map_err(|e| anyhow::anyhow!("Token refresh failed (re-login required): {}", e.message()))?
        .into_inner();

    let auth_mut = config.auth.as_mut().unwrap();
    auth_mut.access_token = resp.access_token;
    auth_mut.refresh_token = resp.refresh_token;
    config.save()?;
    Ok(())
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

async fn status(config: &mut CliConfig) -> anyhow::Result<()> {
    let mut out = io::stdout();
    if config.auth.is_none() {
        writeln!(out, "Not logged in")?;
        return Ok(());
    }

    // Print local credentials first (before mutable borrow for token refresh)
    let auth = config.auth.as_ref().unwrap();
    writeln!(out, "Logged in as: {}", auth.username)?;
    writeln!(out, "User ID: {}", auth.user_id)?;
    if let Some(url) = &config.relay_url {
        writeln!(out, "Relay: {}", url)?;
    }

    // Check relay connectivity by refreshing token
    if config.relay_url.is_some() {
        match ensure_valid_token(config).await {
            Ok(()) => writeln!(out, "Token: valid (refreshed)")?,
            Err(e) => writeln!(out, "Token: invalid ({})", e)?,
        }
    }
    Ok(())
}

/// Write auth status to the given writer (used by tests).
#[cfg(test)]
fn status_to_writer(config: &CliConfig, out: &mut dyn Write) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;

    fn make_auth() -> AuthConfig {
        AuthConfig {
            user_id: "u1".into(),
            username: "alice".into(),
            access_token: "old_at".into(),
            refresh_token: "old_rt".into(),
        }
    }

    #[tokio::test]
    async fn ensure_valid_token_requires_relay_url() {
        let mut config = CliConfig {
            relay_url: None,
            auth: Some(make_auth()),
            ..Default::default()
        };
        let err = ensure_valid_token(&mut config).await.unwrap_err();
        assert!(
            err.to_string().contains("No relay URL"),
            "Expected 'No relay URL' error, got: {}",
            err,
        );
    }

    #[tokio::test]
    async fn ensure_valid_token_requires_auth() {
        let mut config = CliConfig {
            relay_url: Some("http://127.0.0.1:50052".into()),
            auth: None,
            ..Default::default()
        };
        let err = ensure_valid_token(&mut config).await.unwrap_err();
        assert!(
            err.to_string().contains("Not logged in"),
            "Expected 'Not logged in' error, got: {}",
            err,
        );
    }

    #[tokio::test]
    async fn ensure_valid_token_fails_when_relay_unreachable() {
        // Uses a port that nothing is listening on — connection should fail.
        let mut config = CliConfig {
            relay_url: Some("http://127.0.0.1:1".into()),
            auth: Some(make_auth()),
            ..Default::default()
        };
        let err = ensure_valid_token(&mut config).await.unwrap_err();
        assert!(
            err.to_string().contains("Cannot reach relay"),
            "Expected connection error, got: {}",
            err,
        );
    }

    #[test]
    fn status_shows_not_logged_in() {
        let config = CliConfig::default();
        let mut buf = Vec::new();
        status_to_writer(&config, &mut buf);
        let output = String::from_utf8(buf).unwrap();
        assert!(
            output.contains("Not logged in"),
            "Expected 'Not logged in', got: {}",
            output,
        );
    }

    #[test]
    fn status_shows_credentials_and_relay() {
        let config = CliConfig {
            relay_url: Some("http://relay.test:443".into()),
            auth: Some(make_auth()),
            ..Default::default()
        };
        let mut buf = Vec::new();
        status_to_writer(&config, &mut buf);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("alice"), "Missing username in: {}", output);
        assert!(output.contains("u1"), "Missing user_id in: {}", output);
        assert!(
            output.contains("relay.test"),
            "Missing relay URL in: {}",
            output,
        );
    }
}
