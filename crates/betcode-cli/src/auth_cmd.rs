//! Auth subcommands: login, logout, status.
//!
//! User-facing output uses writeln! to stdout (this is a CLI binary, not debug output).

use std::io::{self, Write};

use std::path::Path;

use tonic::transport::{Certificate, Channel, ClientTlsConfig};

use betcode_proto::v1::auth_service_client::AuthServiceClient;
use betcode_proto::v1::{LoginRequest, RefreshTokenRequest, RegisterRequest, RevokeTokenRequest};

use crate::config::{AuthConfig, CliConfig};

/// Build a gRPC channel to the relay, with optional custom CA cert for TLS.
async fn relay_channel(url: &str, ca_cert: Option<&Path>) -> anyhow::Result<Channel> {
    let mut endpoint = Channel::from_shared(url.to_string())?;
    if let Some(ca_path) = ca_cert {
        let ca_pem = std::fs::read_to_string(ca_path)
            .map_err(|e| anyhow::anyhow!("Failed to read CA cert {}: {}", ca_path.display(), e))?;
        let tls_config = ClientTlsConfig::new().ca_certificate(Certificate::from_pem(ca_pem));
        endpoint = endpoint
            .tls_config(tls_config)
            .map_err(|e| anyhow::anyhow!("TLS config error: {e}"))?;
    }
    endpoint
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to relay: {e}"))
}

/// Auth subcommand actions.
#[derive(clap::Subcommand, Debug)]
pub enum AuthAction {
    /// Register a new account on the relay server.
    Register {
        /// Username.
        #[arg(short, long)]
        username: String,
        /// Password.
        #[arg(short, long)]
        password: String,
        /// Email address.
        #[arg(short, long, default_value = "")]
        email: String,
    },
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
        AuthAction::Register {
            username,
            password,
            email,
        } => register(config, &username, &password, &email).await,
        AuthAction::Login { username, password } => login(config, &username, &password).await,
        AuthAction::Logout => logout(config).await,
        AuthAction::Status => status(config).await,
    }
}

/// Refresh the access token via the relay's `RefreshToken` RPC.
///
/// Connects to the relay, exchanges the current refresh token for new
/// access + refresh tokens, and saves the updated config to disk.
///
/// # Panics
///
/// Panics if `config.auth` is `None` after the `is_none()` guard has already
/// verified it is `Some`. This is structurally unreachable.
#[allow(clippy::expect_used)]
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

    let channel = relay_channel(&relay_url, config.relay_ca_cert.as_deref())
        .await
        .map_err(|e| anyhow::anyhow!("Cannot reach relay: {e}"))?;

    let mut client = AuthServiceClient::new(channel);
    let resp = client
        .refresh_token(RefreshTokenRequest { refresh_token })
        .await
        .map_err(|e| anyhow::anyhow!("Token refresh failed (re-login required): {}", e.message()))?
        .into_inner();

    let auth_mut = config
        .auth
        .as_mut()
        .expect("auth was verified present above");
    auth_mut.access_token = resp.access_token;
    auth_mut.refresh_token = resp.refresh_token;
    config.save()?;
    Ok(())
}

async fn register(
    config: &mut CliConfig,
    username: &str,
    password: &str,
    email: &str,
) -> anyhow::Result<()> {
    let relay_url = config
        .relay_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No relay URL configured. Use --relay <url>"))?
        .clone();

    let channel = relay_channel(&relay_url, config.relay_ca_cert.as_deref()).await?;
    let mut client = AuthServiceClient::new(channel);
    let resp = client
        .register(RegisterRequest {
            username: username.into(),
            password: password.into(),
            email: email.into(),
        })
        .await
        .map_err(|e| anyhow::anyhow!("Registration failed: {}", e.message()))?
        .into_inner();

    config.auth = Some(AuthConfig {
        user_id: resp.user_id,
        username: username.into(),
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
    });
    config.save()?;

    let mut out = io::stdout();
    writeln!(out, "Registered and logged in as {username}")?;
    Ok(())
}

async fn login(config: &mut CliConfig, username: &str, password: &str) -> anyhow::Result<()> {
    let relay_url = config
        .relay_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No relay URL configured. Use --relay <url>"))?
        .clone();

    let channel = relay_channel(&relay_url, config.relay_ca_cert.as_deref()).await?;

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
    writeln!(out, "Logged in as {username}")?;
    Ok(())
}

async fn logout(config: &mut CliConfig) -> anyhow::Result<()> {
    if let (Some(auth), Some(relay_url)) = (&config.auth, &config.relay_url) {
        if let Ok(channel) = relay_channel(relay_url, config.relay_ca_cert.as_deref()).await {
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

#[allow(clippy::expect_used)]
async fn status(config: &mut CliConfig) -> anyhow::Result<()> {
    let mut out = io::stdout();
    if config.auth.is_none() {
        writeln!(out, "Not logged in")?;
        return Ok(());
    }

    // Print local credentials first (before mutable borrow for token refresh)
    let auth = config
        .auth
        .as_ref()
        .expect("auth checked via is_none() guard above");
    writeln!(out, "Logged in as: {}", auth.username)?;
    writeln!(out, "User ID: {}", auth.user_id)?;
    if let Some(url) = &config.relay_url {
        writeln!(out, "Relay: {url}")?;
    }

    // Check relay connectivity by refreshing token
    if config.relay_url.is_some() {
        match ensure_valid_token(config).await {
            Ok(()) => writeln!(out, "Token: valid (refreshed)")?,
            Err(e) => writeln!(out, "Token: invalid ({e})")?,
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
                let _ = writeln!(out, "Relay: {url}");
            }
        }
        None => {
            let _ = writeln!(out, "Not logged in");
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::default_trait_access
)]
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
            "Expected 'No relay URL' error, got: {err}",
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
            "Expected 'Not logged in' error, got: {err}",
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
            "Expected connection error, got: {err}",
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
            "Expected 'Not logged in', got: {output}",
        );
    }

    #[tokio::test]
    async fn ensure_valid_token_preserves_auth_on_failure() {
        // When refresh fails (relay unreachable), auth must NOT be cleared.
        // This is the bug that caused "not logged in" after token expiry.
        let mut config = CliConfig {
            relay_url: Some("http://127.0.0.1:1".into()),
            auth: Some(make_auth()),
            active_machine: Some("m1".into()),
            ..Default::default()
        };
        let original_token = config.auth.as_ref().unwrap().access_token.clone();

        let err = ensure_valid_token(&mut config).await;
        assert!(err.is_err(), "Should fail when relay is unreachable");

        // Auth must still be present with original values
        assert!(
            config.auth.is_some(),
            "Auth should NOT be cleared on refresh failure"
        );
        assert_eq!(
            config.auth.as_ref().unwrap().access_token,
            original_token,
            "Access token should be unchanged on refresh failure"
        );
        assert!(
            config.is_relay_mode(),
            "is_relay_mode() should still return true after failed refresh"
        );
    }

    #[tokio::test]
    async fn ensure_valid_token_preserves_auth_fields() {
        // Verify all auth fields survive a failed refresh attempt
        let mut config = CliConfig {
            relay_url: Some("http://127.0.0.1:1".into()),
            auth: Some(AuthConfig {
                user_id: "user-123".into(),
                username: "testuser".into(),
                access_token: "expired_at".into(),
                refresh_token: "expired_rt".into(),
            }),
            ..Default::default()
        };

        let _ = ensure_valid_token(&mut config).await;

        let auth = config
            .auth
            .as_ref()
            .expect("Auth must survive failed refresh");
        assert_eq!(auth.user_id, "user-123");
        assert_eq!(auth.username, "testuser");
        assert_eq!(auth.access_token, "expired_at");
        assert_eq!(auth.refresh_token, "expired_rt");
    }

    #[test]
    fn relay_mode_with_expired_token_still_has_auth() {
        // Simulates the scenario: user was logged in, token expired,
        // config still has auth → is_relay_mode should be true
        let config = CliConfig {
            relay_url: Some("http://relay.test:443".into()),
            auth: Some(make_auth()),
            active_machine: Some("m1".into()),
            ..Default::default()
        };
        assert!(config.is_relay_mode());
        assert!(config.auth.is_some());
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
        assert!(output.contains("alice"), "Missing username in: {output}");
        assert!(output.contains("u1"), "Missing user_id in: {output}");
        assert!(
            output.contains("relay.test"),
            "Missing relay URL in: {output}",
        );
    }

    // =========================================================================
    // relay_channel TLS tests
    // =========================================================================

    #[tokio::test]
    async fn relay_channel_without_ca_cert_connects_plain() {
        // No CA cert → plain connection attempt (fails because nothing listens)
        let result = relay_channel("http://127.0.0.1:1", None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Failed to connect"),
            "Expected connection error, got: {err}",
        );
    }

    #[tokio::test]
    async fn relay_channel_with_nonexistent_ca_cert_fails() {
        let bad_path = std::path::Path::new("/nonexistent/ca.pem");
        let result = relay_channel("https://127.0.0.1:9999", Some(bad_path)).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Failed to read CA cert"),
            "Expected CA read error, got: {err}",
        );
    }

    #[tokio::test]
    async fn relay_channel_with_ca_cert_configures_tls() {
        let dir = std::env::temp_dir().join(format!("betcode-auth-tls-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ca_path = dir.join("ca.pem");
        std::fs::write(
            &ca_path,
            "-----BEGIN CERTIFICATE-----\nMIIBkTCB+wIUEjRVnJ1234=\n-----END CERTIFICATE-----\n",
        )
        .unwrap();

        let result = relay_channel("https://127.0.0.1:9999", Some(&ca_path)).await;
        // Should fail with connection error, not CA read error
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("Failed to read CA cert"),
            "Should not be a CA read error, got: {err}",
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
