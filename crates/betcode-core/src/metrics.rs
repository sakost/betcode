//! Shared `OpenTelemetry` metrics initialisation.
//!
//! This module is only compiled when the `metrics` Cargo feature is enabled.
//! It sets up the OTLP exporter for both traces and metrics, sending
//! telemetry to a configurable endpoint (e.g. an `OpenTelemetry` Collector).

use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

/// Errors that can occur during metrics / tracing pipeline initialisation.
#[derive(Debug, thiserror::Error)]
pub enum MetricsError {
    /// Failed to build an OTLP exporter.
    #[error("failed to build OTLP exporter: {0}")]
    ExporterBuild(#[from] opentelemetry_otlp::ExporterBuildError),

    /// Failed during `OTel` SDK shutdown or flush.
    #[error("OpenTelemetry SDK error: {0}")]
    Sdk(#[from] opentelemetry_sdk::error::OTelSdkError),
}

/// Opaque handle that keeps the `OpenTelemetry` providers alive.
///
/// When dropped, the providers are **not** shut down automatically -- call
/// [`MetricsGuard::shutdown`] for a graceful flush before exiting.
pub struct MetricsGuard {
    tracer_provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
}

impl MetricsGuard {
    /// Gracefully shut down both providers, flushing any buffered telemetry.
    ///
    /// # Errors
    ///
    /// Returns an error if either provider fails to shut down cleanly.
    pub fn shutdown(self) -> Result<(), MetricsError> {
        self.tracer_provider.shutdown()?;
        self.meter_provider.shutdown()?;
        Ok(())
    }
}

/// Initialise the `OpenTelemetry` OTLP pipeline for traces **and** metrics.
///
/// * `endpoint` -- OTLP receiver URL, e.g. `"http://localhost:4317"` (gRPC).
///
/// Returns a [`MetricsGuard`] that **must** be kept alive for the lifetime of
/// the application.  Call [`MetricsGuard::shutdown`] before process exit to
/// flush buffered telemetry.
///
/// # Errors
///
/// Returns [`MetricsError`] if the OTLP exporters cannot be constructed.
pub fn init_metrics(endpoint: &str) -> Result<MetricsGuard, MetricsError> {
    // --- Traces ---
    let trace_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(trace_exporter)
        .build();

    global::set_tracer_provider(tracer_provider.clone());

    // --- Metrics ---
    let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;

    let meter_provider = SdkMeterProvider::builder()
        .with_periodic_exporter(metric_exporter)
        .build();

    global::set_meter_provider(meter_provider.clone());

    Ok(MetricsGuard {
        tracer_provider,
        meter_provider,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[allow(clippy::unwrap_used)]
    async fn init_metrics_does_not_panic() {
        // Use a dummy endpoint -- we just verify the pipeline constructs
        // without panicking. The exporter will fail at send-time, which is
        // expected in tests without a collector.
        let guard = init_metrics("http://localhost:4317").unwrap();
        // Shutdown should not panic either.
        guard.shutdown().unwrap();
    }
}
