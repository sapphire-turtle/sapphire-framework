//! The framework's own metrics: their names, their help texts, and the values the server
//! sets as they change or as a scrape asks.
//!
//! Only the `metrics` facade is used here, so recording compiles in every build and costs a
//! no-op call when no recorder is installed. Naming, and the rule that no metric or label
//! carries an identifier, are in the metrics design spec.

/// Always 1, labelled with the application and its version.
#[cfg(feature = "metrics")]
pub(crate) const BUILD_INFO: &str = "sapphire_build_info";

#[cfg(feature = "metrics")]
const WORKSPACE_SELECTED: &str = "sapphire_workspace_selected";
#[cfg(feature = "metrics")]
const SYNC_ENABLED: &str = "sapphire_sync_enabled";
#[cfg(feature = "metrics")]
const SYNC_BRIDGE_AVAILABLE: &str = "sapphire_sync_bridge_available";
#[cfg(feature = "metrics")]
const SYNC_PAUSED: &str = "sapphire_sync_paused";
#[cfg(feature = "metrics")]
const SYNC_ERROR: &str = "sapphire_sync_error";
#[cfg(feature = "metrics")]
const SYNC_PEERS: &str = "sapphire_sync_peers";
#[cfg(feature = "metrics")]
const SYNC_LIVE_SESSIONS: &str = "sapphire_sync_live_sessions";
#[cfg(feature = "metrics")]
const EMBEDDING_VECTORS: &str = "sapphire_embedding_vectors";
#[cfg(feature = "metrics")]
const EMBEDDING_PENDING: &str = "sapphire_embedding_pending";
#[cfg(feature = "metrics")]
const EMBEDDING_RUNNING: &str = "sapphire_embedding_running";
#[cfg(feature = "metrics")]
const UPDATE_AVAILABLE: &str = "sapphire_update_available";

// The event metrics are recorded where the event happens, in builds without the exporter
// too; only `describe` needs them under the feature, hence the allowance.
#[cfg_attr(not(feature = "metrics"), allow(dead_code))]
pub(crate) const IPC_CONNECTIONS: &str = "sapphire_ipc_connections";
#[cfg_attr(not(feature = "metrics"), allow(dead_code))]
pub(crate) const IPC_CONNECTIONS_TOTAL: &str = "sapphire_ipc_connections_total";
#[cfg_attr(not(feature = "metrics"), allow(dead_code))]
pub(crate) const REINDEX_TOTAL: &str = "sapphire_reindex_total";
#[cfg_attr(not(feature = "metrics"), allow(dead_code))]
pub(crate) const REINDEX_DURATION_SECONDS: &str = "sapphire_reindex_duration_seconds";
#[cfg_attr(not(feature = "metrics"), allow(dead_code))]
pub(crate) const SYNC_SESSIONS_TOTAL: &str = "sapphire_sync_sessions_total";

/// Register a help text for every framework metric.
///
/// Help texts are registered with the recorder, so this runs once it is installed. A gauge
/// that has not been set is not rendered, whatever its description.
#[cfg(feature = "metrics")]
pub(crate) fn describe() {
    use metrics::{Unit, describe_counter, describe_gauge, describe_histogram};

    describe_gauge!(
        BUILD_INFO,
        "Always 1; labelled with the application and its version."
    );
    describe_gauge!(WORKSPACE_SELECTED, "1 if a workspace is selected.");
    describe_gauge!(SYNC_ENABLED, "1 if the workspace is synced.");
    describe_gauge!(SYNC_BRIDGE_AVAILABLE, "1 if the bridge is reachable.");
    describe_gauge!(SYNC_PAUSED, "1 if replication is paused.");
    describe_gauge!(SYNC_ERROR, "1 if the last sync attempt failed.");
    describe_gauge!(SYNC_PEERS, "The workgroup's other devices.");
    describe_gauge!(SYNC_LIVE_SESSIONS, "Live sessions with peers now.");
    describe_gauge!(EMBEDDING_VECTORS, "Documents with a vector.");
    describe_gauge!(EMBEDDING_PENDING, "Documents still without a vector.");
    describe_gauge!(EMBEDDING_RUNNING, "1 while an embedding pass runs.");
    describe_gauge!(
        UPDATE_AVAILABLE,
        "1 if a newer release is known; absent when the check is off or has no answer yet."
    );
    describe_gauge!(IPC_CONNECTIONS, "Open IPC connections.");
    describe_counter!(IPC_CONNECTIONS_TOTAL, "IPC connections accepted.");
    describe_counter!(REINDEX_TOTAL, "Re-index runs, by result.");
    describe_histogram!(
        REINDEX_DURATION_SECONDS,
        Unit::Seconds,
        "How long a re-index run took."
    );
    describe_counter!(
        SYNC_SESSIONS_TOTAL,
        "Live sessions with a peer whose opening exchange finished, by result."
    );
}

/// A flag as a gauge value.
#[cfg(feature = "metrics")]
fn flag(on: bool) -> f64 {
    if on { 1.0 } else { 0.0 }
}

/// Set the state gauges from what the server holds now.
///
/// Sync state is what `sync.status` answers for the current workspace and update state is
/// the update check's shared status. Free-text states (`paused`, `last_error`) become flags,
/// so no text reaches the page. Embedding and update gauges are left unset when there is
/// nothing to say, which a scraper sees as absent.
#[cfg(feature = "metrics")]
pub(crate) async fn refresh_state(
    current: &crate::current::Current,
    update: &crate::update::SharedStatus,
) {
    use metrics::gauge;

    let root = current.root();
    let status = current.status().await;
    gauge!(WORKSPACE_SELECTED).set(flag(root.is_some()));
    gauge!(SYNC_ENABLED).set(flag(status.enabled));
    gauge!(SYNC_BRIDGE_AVAILABLE).set(flag(status.bridge_available));
    gauge!(SYNC_PAUSED).set(flag(status.paused.is_some()));
    gauge!(SYNC_ERROR).set(flag(status.last_error.is_some()));
    gauge!(SYNC_PEERS).set(status.peers as f64);
    let live = match (current.sync(), &root) {
        (Some(runtime), Some(root)) => runtime.live_session_devices(root).await.len(),
        _ => 0,
    };
    gauge!(SYNC_LIVE_SESSIONS).set(live as f64);
    if let Some(embedding) = &status.embedding {
        gauge!(EMBEDDING_VECTORS).set(embedding.vectors as f64);
        gauge!(EMBEDDING_PENDING).set(embedding.pending as f64);
        gauge!(EMBEDDING_RUNNING).set(flag(embedding.running));
    }
    // Copied out so the lock is not held across an await, or past this line.
    let (enabled, known, available) = {
        let update = update.lock().expect("update status lock");
        (
            update.enabled,
            update.latest.is_some(),
            update.update_available,
        )
    };
    if enabled && known {
        gauge!(UPDATE_AVAILABLE).set(flag(available));
    }
}
