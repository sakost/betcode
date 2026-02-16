//! Shared tracing/logging initialization.
//!
//! Both betcode-daemon and betcode-relay use the same pattern for setting
//! up `tracing_subscriber` with an env-filter and optional JSON output.
//!
//! When the `metrics` feature is enabled, an `OpenTelemetry` tracing layer
//! can be added to export spans via OTLP.

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Initialise the global tracing subscriber.
///
/// * `default_filter` -- default `RUST_LOG` value when the env-var is not set
///   (e.g. `"betcode_daemon=info"`).
/// * `log_json` -- when `true`, emit structured JSON log lines instead of the
///   human-readable format.
pub fn init_tracing(default_filter: &str, log_json: bool) {
    init_tracing_with_metrics(default_filter, log_json, None);
}

/// Initialise the global tracing subscriber with optional `OpenTelemetry`
/// integration.
///
/// * `default_filter` -- default `RUST_LOG` value when the env-var is not set.
/// * `log_json` -- emit structured JSON log lines when `true`.
/// * `metrics_endpoint` -- when `Some`, adds an `OpenTelemetry` tracing layer
///   that exports spans to the given OTLP endpoint. Requires the `metrics`
///   Cargo feature; ignored when the feature is disabled.
///
/// # Returns
///
/// An optional [`MetricsGuardHandle`] that must be held alive for
/// the lifetime of the application when the `metrics` feature is active.
/// Without the feature the return value is always `None`.
pub fn init_tracing_with_metrics(
    default_filter: &str,
    log_json: bool,
    #[allow(unused_variables)] metrics_endpoint: Option<&str>,
) -> Option<MetricsGuardHandle> {
    let env_filter = tracing_subscriber::EnvFilter::new(
        std::env::var("RUST_LOG").unwrap_or_else(|_| default_filter.into()),
    );

    #[cfg(feature = "metrics")]
    {
        init_tracing_inner(env_filter, log_json, metrics_endpoint)
    }

    #[cfg(not(feature = "metrics"))]
    {
        init_tracing_inner(env_filter, log_json);
        None
    }
}

// ---- Feature-gated implementation details ----

/// When the `metrics` feature is **disabled**, the guard is a simple unit
/// wrapper so call-sites can hold `Option<MetricsGuardHandle>` uniformly.
#[cfg(not(feature = "metrics"))]
pub type MetricsGuardHandle = ();

/// When the `metrics` feature is **enabled**, the guard wraps the real
/// [`crate::metrics::MetricsGuard`].
#[cfg(feature = "metrics")]
pub type MetricsGuardHandle = crate::metrics::MetricsGuard;

/// Build the subscriber *without* `OpenTelemetry`.
#[cfg(not(feature = "metrics"))]
fn init_tracing_inner(env_filter: tracing_subscriber::EnvFilter, log_json: bool) {
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

/// Build the subscriber *with* an optional `OpenTelemetry` layer.
#[cfg(feature = "metrics")]
#[allow(clippy::print_stderr)]
fn init_tracing_inner(
    env_filter: tracing_subscriber::EnvFilter,
    log_json: bool,
    metrics_endpoint: Option<&str>,
) -> Option<MetricsGuardHandle> {
    // Attempt to initialise the OTel pipeline if an endpoint was supplied.
    let guard = metrics_endpoint.and_then(|ep| match crate::metrics::init_metrics(ep) {
        Ok(g) => Some(g),
        Err(err) => {
            // Cannot log via tracing yet (subscriber not initialised), so use
            // stderr directly for this one-time bootstrap warning.
            eprintln!("Warning: failed to initialise OpenTelemetry pipeline: {err}");
            None
        }
    });

    // Build the OpenTelemetry layer only when the pipeline initialised
    // successfully.  Using `Option<L>` is handled by tracing-subscriber
    // as a no-op when `None`.
    if log_json {
        let otel_layer = guard.as_ref().map(|_| {
            let tracer = opentelemetry::global::tracer("betcode");
            tracing_opentelemetry::OpenTelemetryLayer::new(tracer)
        });
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer().json())
            .with(otel_layer)
            .init();
    } else {
        let otel_layer = guard.as_ref().map(|_| {
            let tracer = opentelemetry::global::tracer("betcode");
            tracing_opentelemetry::OpenTelemetryLayer::new(tracer)
        });
        tracing_subscriber::registry()
            .with(env_filter)
            .with(tracing_subscriber::fmt::layer())
            .with(otel_layer)
            .init();
    }

    guard
}
