//! Prometheus metrics at `/metrics` on the HTTP listener (feature `metrics`).
//!
//! The one route not behind an external device's token, and not behind `allowed_hosts`
//! either: it carries nothing worth hiding — counts and states, never an identifier — and
//! Prometheus scrapers expect an open endpoint. The rmcp `Host` check that `allowed_hosts`
//! drives protects `/mcp` from DNS rebinding; a read-only page of numbers needs no such guard.
//!
//! The recorder is process-wide, so it is installed once and every server in the process
//! (a test runs several) renders from the same handle. A failure here never stops the
//! server: with another recorder already installed, `/metrics` is simply not served.

use std::sync::{Arc, OnceLock};

use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

use crate::current::Current;
use crate::update;

/// The content type of Prometheus's text exposition format.
const CONTENT_TYPE_TEXT: &str = "text/plain; version=0.0.4";

/// Install the Prometheus recorder for this process, once.
///
/// `None` when another recorder was installed first — the application installed its own,
/// say. That is logged once, and the server carries on without `/metrics`.
pub(crate) fn install() -> Option<&'static PrometheusHandle> {
    static HANDLE: OnceLock<Option<PrometheusHandle>> = OnceLock::new();
    HANDLE
        .get_or_init(|| match PrometheusBuilder::new().install_recorder() {
            Ok(handle) => {
                // Help texts are registered with the recorder, so they wait until it is in.
                metrics_process::Collector::default().describe();
                crate::instrument::describe();
                Some(handle)
            }
            Err(err) => {
                tracing::error!(
                    "metrics are off: another metrics recorder is installed in this process: {err}"
                );
                None
            }
        })
        .as_ref()
}

/// What a scrape refreshes before it renders: the values nothing updates as it happens.
pub(crate) struct Scrape {
    /// The process's CPU, memory and file descriptors, read from the operating system.
    pub(crate) process: metrics_process::Collector,
    /// The workspace and sync state that the state gauges report.
    pub(crate) current: Arc<Current>,
    /// What the update check has found so far.
    pub(crate) update: update::SharedStatus,
}

impl Scrape {
    /// Refresh every value the scrape owns.
    async fn refresh(&self) {
        crate::instrument::refresh_state(&self.current, &self.update).await;
        self.process.collect();
    }
}

/// `GET /metrics`, rendering from `handle` after `scrape` has refreshed its values.
///
/// It answers 200 whatever the server's state: anything worth knowing is a value on the
/// page, not a failed scrape.
pub(crate) fn routes(handle: &'static PrometheusHandle, scrape: Arc<Scrape>) -> axum::Router {
    axum::Router::new().route(
        "/metrics",
        get(move || async move {
            scrape.refresh().await;
            // Without the exporter's own HTTP server nothing else runs upkeep, which drains
            // the histograms' buckets; once per scrape is as often as anyone reads them.
            handle.run_upkeep();
            ([(CONTENT_TYPE, CONTENT_TYPE_TEXT)], handle.render()).into_response()
        }),
    )
}
