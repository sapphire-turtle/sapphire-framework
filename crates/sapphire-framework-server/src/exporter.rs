//! Prometheus metrics at `/metrics` on the HTTP listener (feature `metrics`).
//!
//! The one route not behind an API key's token, and not behind `allowed_hosts`
//! either: it carries nothing worth hiding — counts and states, never an identifier — and
//! Prometheus scrapers expect an open endpoint. The rmcp `Host` check that `allowed_hosts`
//! drives protects `/mcp` from DNS rebinding; a read-only page of numbers needs no such guard.
//!
//! The recorder is process-wide, so it is installed once and every server in the process
//! (a test runs several) renders from the same handle. A failure here never stops the
//! server: with another recorder already installed, `/metrics` is simply not served. Two
//! servers with metrics on in one process therefore share every series: the state gauges
//! show whichever server was scraped last, and the counters are the process's totals.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};

use crate::current::Current;
use crate::update;

/// The content type of Prometheus's text exposition format.
const CONTENT_TYPE_TEXT: &str = "text/plain; version=0.0.4";

/// The re-index duration histogram's bucket bounds, in seconds.
///
/// Without bounds the exporter renders a histogram as a summary, whose quantiles cannot be
/// summed across servers or windows. These span a small workspace's sub-second run to a
/// large one's first full index.
const REINDEX_BUCKETS: &[f64] = &[0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 300.0];

/// How long a scrape waits for the server's state before it renders without it.
///
/// Reading sync state can wait on locks a busy runtime holds. A scraper's own timeout is
/// commonly ten seconds, so two keeps the page answering well within it.
const STATE_TIMEOUT: Duration = Duration::from_secs(2);

/// The recorder's builder, with the re-index histogram's buckets set.
///
/// Setting buckets only fails on an empty list, which `REINDEX_BUCKETS` is not; should it
/// fail anyway, the plain builder still serves every metric, with the histogram as a summary.
fn builder() -> PrometheusBuilder {
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full(crate::instrument::REINDEX_DURATION_SECONDS.into()),
            REINDEX_BUCKETS,
        )
        .unwrap_or_else(|err| {
            tracing::error!("re-index durations render as a summary: {err}");
            PrometheusBuilder::new()
        })
}

/// Install the Prometheus recorder for this process, once.
///
/// `None` when another recorder was installed first — the application installed its own,
/// say. That is logged once, and the server carries on without `/metrics`.
pub(crate) fn install() -> Option<&'static PrometheusHandle> {
    static HANDLE: OnceLock<Option<PrometheusHandle>> = OnceLock::new();
    HANDLE
        .get_or_init(|| match builder().install_recorder() {
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
    /// The application's own collectors, run in the order it added them.
    pub(crate) collectors: Vec<Arc<dyn Fn() + Send + Sync>>,
}

impl Scrape {
    /// Refresh every value the scrape owns.
    ///
    /// The state gauges get [`STATE_TIMEOUT`] to refresh. Past it they keep the values the
    /// last scrape set, rather than values made up for the occasion, and the page renders
    /// anyway: `/metrics` answers whatever the server's state.
    async fn refresh(&self) {
        let state = crate::instrument::refresh_state(&self.current, &self.update);
        if tokio::time::timeout(STATE_TIMEOUT, state).await.is_err() {
            tracing::debug!(
                "metrics: the server's state took over {STATE_TIMEOUT:?}; serving the last values"
            );
        }
        self.process.collect();
        for collect in &self.collectors {
            collect();
        }
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
