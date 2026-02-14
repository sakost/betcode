//! Shared tracing/logging initialization.
//!
//! Both betcode-daemon and betcode-relay use the same pattern for setting
//! up `tracing_subscriber` with an env-filter and optional JSON output.

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Initialise the global tracing subscriber.
///
/// * `default_filter` -- default `RUST_LOG` value when the env-var is not set
///   (e.g. `"betcode_daemon=info"`).
/// * `log_json` -- when `true`, emit structured JSON log lines instead of the
///   human-readable format.
pub fn init_tracing(default_filter: &str, log_json: bool) {
    let env_filter = tracing_subscriber::EnvFilter::new(
        std::env::var("RUST_LOG").unwrap_or_else(|_| default_filter.into()),
    );
    if log_json {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }
}
