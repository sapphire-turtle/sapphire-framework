//! Replication, from the app server's side.
//!
//! One `Replica` per synced workspace, registered with the bridge, driven by announcements
//! from it and by local edits.
//!
//! See `docs/superpowers/specs/2026-09-16-process-architecture-design.md` §4.2.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use grain_id::GrainId;
use sapphire_backend::protocol as proto;
use sapphire_bridge_api::{
    IncomingParams, RegisterParams, WorkspaceRegistration, WorkspacesResult,
};
use sapphire_bridge_client::BridgeClient;
use sapphire_ipc::ManagedBy;
use sapphire_sync::{PathUpdate, PauseReason, Replica, ReplicaConfig, ScanOutcome, SystemClock};
use sapphire_workspace::{AppContext, Workspace};
use tokio::sync::{Mutex, OnceCell, broadcast, mpsc};

pub mod id;
mod live;
mod methods;
#[cfg(any(test, feature = "test-util"))]
pub mod testing;
mod topology;
mod watch;

pub use id::{SYNC_ID_FILE, WORKSPACE_MAP_FILE, sync_id, sync_id_path};
pub use live::{DIAL_BACKOFF_MAX, OPEN_TIMEOUT};
pub use methods::sync_router;
/// What [`SyncRuntime::peers`] answers with.
pub use sapphire_bridge_api::{PeerInfo, PeersResult};
pub use watch::{DEBOUNCE, Watcher};

use live::{LivePeers, dial_backoff};

use crate::error::{Error, Result};
use crate::settings::Settings;

use crate::host::WorkspaceHost;

/// What `sync.status` reports.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct SyncStatus {
    /// Whether this workspace is synced at all.
    pub enabled: bool,
    /// Its identity across devices, when it is.
    pub workspace_id: Option<GrainId>,
    /// How many devices the workgroup has, this host excluded.
    pub peers: usize,
    /// Why replication is paused, if it is — a missing root or marker directory.
    pub paused: Option<String>,
    /// The last failure, if any.
    pub last_error: Option<String>,
    /// Whether the bridge is reachable. `false` is not an outage: the app server works.
    pub bridge_available: bool,
    /// How the workspace is wired to its peers, by the roles the bridge last reported.
    pub topology: proto::Topology,
    /// How far embedding has got, when this host embeds (#188).
    pub embedding: Option<proto::EmbeddingProgress>,
}

/// One workspace this runtime syncs.
struct Synced {
    /// Its identity across devices, shared with every peer.
    workspace_id: GrainId,
    /// Its replica store. Held open because the store is a redb database.
    replica: Arc<Mutex<Replica>>,
    /// Why replication is paused, from the last scan.
    paused: Option<PauseReason>,
    /// The last failure, if any.
    last_error: Option<String>,
}

/// How old a vector file no live content needs must be before the primary device removes
/// it: a vector can arrive before its file, and a day is far longer than that window.
const STALE_VECTOR_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Texts per embedding request in a server pass: small, so a search query waiting on the
/// bridge's one worker is never stuck behind a long backfill (#188).
const PASS_BATCH: usize = 4;

/// How the primary device paces its backfill (#188).
#[derive(Clone, Copy, Debug)]
pub struct BackfillTiming {
    /// How old another device's version must be before the primary embeds it: its author
    /// has embedded it by then, or will not.
    pub grace: std::time::Duration,
    /// How often the primary device runs a pass with nothing prompting it.
    pub interval: std::time::Duration,
}

impl Default for BackfillTiming {
    fn default() -> Self {
        BackfillTiming {
            grace: std::time::Duration::from_secs(10 * 60),
            interval: std::time::Duration::from_secs(10 * 60),
        }
    }
}

/// How long [`SyncRuntime::keep_connected`] waits before its first retry after the bridge
/// could not be reached. Each failure doubles it, up to [`RECONNECT_BACKOFF_MAX`].
pub const RECONNECT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(500);

/// The longest [`SyncRuntime::keep_connected`] waits between two attempts to reach the
/// bridge: a bridge that comes up is found within this long.
pub const RECONNECT_BACKOFF_MAX: std::time::Duration = std::time::Duration::from_secs(30);

/// The wait before reconnection attempt `attempt` (0-based), backing off from `first`.
fn reconnect_backoff(first: std::time::Duration, attempt: u32) -> std::time::Duration {
    first
        .saturating_mul(1u32 << attempt.min(16))
        .min(RECONNECT_BACKOFF_MAX.max(first))
}

/// What a connection attempt to the bridge yields.
type Connecting = std::pin::Pin<
    Box<dyn std::future::Future<Output = sapphire_ipc::Result<Option<BridgeClient>>> + Send>,
>;

/// How a [`SyncRuntime`] reaches the bridge, on start and whenever it has lost it (#244).
///
/// An attempt answers `Ok(None)` when no bridge is running — the runtime then tries again
/// later — and an error for anything else, such as a bridge of another API version.
#[derive(Clone)]
pub struct BridgeConnector(Arc<dyn Fn() -> Connecting + Send + Sync>);

impl BridgeConnector {
    /// Connect with `connect`.
    pub fn new<F, Fut>(connect: F) -> BridgeConnector
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = sapphire_ipc::Result<Option<BridgeClient>>>
            + Send
            + 'static,
    {
        BridgeConnector(Arc::new(move || Box::pin(connect())))
    }

    /// The host's own bridge, at its standard endpoint, as a client of `kind` at
    /// `version`.
    pub fn standard(kind: &'static str, version: &'static str) -> BridgeConnector {
        BridgeConnector::new(move || async move {
            match BridgeClient::connect(kind, version).await {
                Ok(client) => Ok(Some(client)),
                Err(sapphire_ipc::Error::NotRunning(_)) => Ok(None),
                Err(err) => Err(err),
            }
        })
    }

    /// The bridge listening at `endpoint`: for a fixture that runs its own.
    pub fn at(
        endpoint: sapphire_ipc::Endpoint,
        kind: &'static str,
        version: &'static str,
    ) -> BridgeConnector {
        BridgeConnector::new(move || {
            let endpoint = endpoint.clone();
            async move { BridgeClient::connect_at(&endpoint, kind, version).await }
        })
    }

    /// One attempt: `None` when no bridge is running.
    pub(crate) async fn connect(&self) -> sapphire_ipc::Result<Option<BridgeClient>> {
        (self.0)().await
    }
}

impl std::fmt::Debug for BridgeConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BridgeConnector")
    }
}

/// What [`SyncRuntime::reclaim`] found of a replica `disable` let go of.
enum Reclaimed {
    /// Still held: here it is.
    Open(Arc<Mutex<Replica>>),
    /// Let go of since, so its store may be closing still.
    Closing,
    /// Nothing to take back.
    Closed,
}

/// How often, and how far apart, `enable` tries to open a store that may still be closing.
const CLOSING_RETRIES: u32 = 50;
const CLOSING_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(20);

/// Replication for one application's workspaces.
pub struct SyncRuntime {
    ctx: &'static AppContext,
    /// The bridge connection, while there is one. Swapped when the runtime reconnects
    /// (#244); [`run`](SyncRuntime::run) follows it.
    bridge: tokio::sync::watch::Sender<Option<Arc<BridgeClient>>>,
    /// How to reach the bridge again; `None` for a runtime given one connection to use.
    connector: Option<BridgeConnector>,
    /// Woken by [`reconnect_now`](SyncRuntime::reconnect_now): someone wants the bridge
    /// now, so [`keep_connected`](SyncRuntime::keep_connected) cuts its backoff short.
    reconnect_now: tokio::sync::Notify,
    /// Workspaces asked to sync while the bridge was away, with their ids. Enabled once
    /// it is back.
    ///
    /// Also the lock that orders an `enable` against a reconnection: whoever holds it sees
    /// the bridge slot as the other left it, so a workspace is either enabled now or left
    /// here for the reconnection to enable — never neither.
    pending: Mutex<HashMap<PathBuf, GrainId>>,
    /// The announcements of a connection [`attach`](SyncRuntime::attach) just made,
    /// subscribed to before it registered: the bridge announces the streams that waited for
    /// this server as soon as it registers (#244), and a subscription made afterwards by
    /// [`run`](SyncRuntime::run) would miss them.
    subscribed: std::sync::Mutex<Option<(Arc<BridgeClient>, broadcast::Receiver<IncomingParams>)>>,
    exe_path: PathBuf,
    managed_by: ManagedBy,
    synced: Mutex<HashMap<PathBuf, Synced>>,
    /// The replicas [`disable`](SyncRuntime::disable) let go of, by canonical root, for as
    /// long as something else still holds one: a scan, a session or the first dial that
    /// began before. The store is a redb database, open until its last holder drops it, so
    /// a re-enable meanwhile could not open it again; [`enable`](SyncRuntime::enable)
    /// takes the open one back instead. [`release`](SyncRuntime::release) puts nothing
    /// here: its store is about to be moved or deleted.
    stopped: std::sync::Mutex<HashMap<PathBuf, std::sync::Weak<Mutex<Replica>>>>,
    /// This host's device id. Asked of the bridge once, on the first [`enable`], and never
    /// again: a second registration with an empty workspace list would briefly clear this
    /// application's routes.
    ///
    /// [`enable`]: SyncRuntime::enable
    device_id: OnceCell<GrainId>,
    /// The file watcher, once [`watch_changes`] has started it.
    ///
    /// The runtime owns it because the spec puts it here: the sync runtime is a watcher and
    /// a replica per registered root, and the two must agree on which roots are synced.
    /// [`enable`] and [`disable`] keep them in step.
    ///
    /// [`watch_changes`]: SyncRuntime::watch_changes
    /// [`enable`]: SyncRuntime::enable
    /// [`disable`]: SyncRuntime::disable
    watcher: OnceCell<Arc<Watcher>>,
    /// The app server that owns these workspaces' caches, once it has told us.
    ///
    /// A replication session writes files, and the search index is not part of a session: the
    /// index belongs to the app server, which holds the workspace open. Reaching it through
    /// the host is what keeps a received file and its index moving together, and it is the
    /// *same* [`WorkspaceState`](sapphire_workspace::WorkspaceState) the server serves from —
    /// opening a second one would collide on the retrieve store's exclusive lock.
    ///
    /// Absent for a runtime nobody has wired to an app server (a test stub driving the
    /// replica directly). Nothing then indexes, which is correct: nothing owns an index.
    host: OnceCell<Arc<WorkspaceHost>>,
    /// The app server's settings, whose workspace scope is a file in the selected
    /// workspace's marker: read again when a session or a local edit changes the files.
    settings: std::sync::Mutex<Option<Settings>>,
    /// Serialises re-indexing, so two sessions finishing at once do not both sweep the
    /// workspace through the same store.
    reindexing: Mutex<()>,
    /// Roots with an embedding pass waiting to run (#187). A request for a root already
    /// waiting is folded into that pass.
    embed_wanted: Mutex<std::collections::HashSet<PathBuf>>,
    /// Serialises embedding passes: one model, one queue.
    embedding: Mutex<()>,
    /// Roots whose embedding pass is running now, for `sync.status`.
    embed_running: Mutex<std::collections::HashSet<PathBuf>>,
    /// The primary device's backfill pacing (#188).
    backfill: std::sync::Mutex<BackfillTiming>,
    /// The bridge an embedding pass asks for its model; `None` is the host's standard one.
    bridge_endpoint: Option<sapphire_ipc::Endpoint>,
    /// The live session table of every synced workspace, keyed by canonical root.
    ///
    /// One table per workspace rather than one for the runtime: a session is about one
    /// workspace, and the two storm rules are per-workspace rules. A root that is not
    /// synced has none, so a dial or a push for it is a no-op rather than an error.
    live: Mutex<HashMap<PathBuf, Arc<LivePeers>>>,
    /// The bridge's last answer to `peers`, roles included. The inbound guard and
    /// [`is_primary`](SyncRuntime::is_primary) read it rather than asking again.
    last_peers: Mutex<Option<PeersResult>>,
    /// How long a dial waits for the bridge to open a stream; [`OPEN_TIMEOUT`] by default.
    open_timeout: std::time::Duration,
    /// The first wait of [`keep_connected`](SyncRuntime::keep_connected)'s backoff;
    /// [`RECONNECT_BACKOFF`] but for tests.
    reconnect_first: std::time::Duration,
    /// The first dials [`enable`](SyncRuntime::enable) leaves running. Each pins the
    /// runtime, so [`stop_dials`](SyncRuntime::stop_dials) ends them with the server.
    first_dials: std::sync::Mutex<tokio::task::JoinSet<()>>,
    /// The rest of the runtime's background work: inbound sessions, the readers of live
    /// sessions, and embedding passes. Each pins the runtime — and through it the replica
    /// stores, which are redb databases locked until the last pin drops — so
    /// [`stop_background`](SyncRuntime::stop_background) ends them with the server (#235).
    background: std::sync::Mutex<tokio::task::JoinSet<()>>,
}

impl SyncRuntime {
    /// A runtime for `ctx`'s application, syncing through `bridge`.
    ///
    /// It has no way to reach the bridge again: when the connection closes, sync stops until
    /// the runtime is given a [`connector`](Self::with_connector).
    pub fn new(
        ctx: &'static AppContext,
        bridge: Arc<BridgeClient>,
        exe_path: PathBuf,
        managed_by: ManagedBy,
    ) -> SyncRuntime {
        SyncRuntime::build(ctx, Some(bridge), None, exe_path, managed_by)
    }

    /// A runtime for `ctx`'s application that reaches the bridge with `connector`, and again
    /// whenever it loses it (#244).
    ///
    /// Not connected yet: [`connect`](Self::connect) makes the first attempt, and
    /// [`keep_connected`](Self::keep_connected) the rest. [`AppServer::run`] does both.
    ///
    /// [`AppServer::run`]: crate::AppServer::run
    pub fn connecting(
        ctx: &'static AppContext,
        connector: BridgeConnector,
        exe_path: PathBuf,
        managed_by: ManagedBy,
    ) -> SyncRuntime {
        SyncRuntime::build(ctx, None, Some(connector), exe_path, managed_by)
    }

    /// Reach the bridge again with `connector` once the connection closes.
    pub fn with_connector(mut self, connector: BridgeConnector) -> SyncRuntime {
        self.connector = Some(connector);
        self
    }

    fn build(
        ctx: &'static AppContext,
        bridge: Option<Arc<BridgeClient>>,
        connector: Option<BridgeConnector>,
        exe_path: PathBuf,
        managed_by: ManagedBy,
    ) -> SyncRuntime {
        SyncRuntime {
            ctx,
            bridge: tokio::sync::watch::Sender::new(bridge),
            connector,
            reconnect_now: tokio::sync::Notify::new(),
            pending: Mutex::new(HashMap::new()),
            subscribed: std::sync::Mutex::new(None),
            exe_path,
            managed_by,
            synced: Mutex::new(HashMap::new()),
            stopped: std::sync::Mutex::new(HashMap::new()),
            device_id: OnceCell::new(),
            watcher: OnceCell::new(),
            host: OnceCell::new(),
            settings: std::sync::Mutex::new(None),
            reindexing: Mutex::new(()),
            embed_wanted: Mutex::new(std::collections::HashSet::new()),
            embedding: Mutex::new(()),
            embed_running: Mutex::new(std::collections::HashSet::new()),
            backfill: std::sync::Mutex::new(BackfillTiming::default()),
            bridge_endpoint: None,
            live: Mutex::new(HashMap::new()),
            last_peers: Mutex::new(None),
            open_timeout: OPEN_TIMEOUT,
            reconnect_first: RECONNECT_BACKOFF,
            first_dials: std::sync::Mutex::new(tokio::task::JoinSet::new()),
            background: std::sync::Mutex::new(tokio::task::JoinSet::new()),
        }
    }

    /// The bridge, while connected.
    fn bridge(&self) -> Result<Arc<BridgeClient>> {
        match &*self.bridge.borrow() {
            Some(bridge) if !bridge.is_closed() => Ok(Arc::clone(bridge)),
            _ => Err(Error::BridgeDisconnected),
        }
    }

    /// Whether the runtime has a live connection to the bridge.
    pub fn is_connected(&self) -> bool {
        self.bridge().is_ok()
    }

    /// The bridge connection, while there is an open one: what the relay forwards over.
    pub(crate) fn connected_bridge(&self) -> Option<Arc<BridgeClient>> {
        self.bridge().ok()
    }

    /// Have [`keep_connected`](Self::keep_connected) try the bridge now rather than when its
    /// backoff next ends, and start its backoff over: for the relay, whose client may just
    /// have started the bridge. A no-op while connected, bar one early attempt after the
    /// next loss, which makes one at once anyway.
    pub(crate) fn reconnect_now(&self) {
        self.reconnect_now.notify_one();
    }

    /// The bridge slot, followed: changes each time the runtime connects or loses the
    /// connection.
    pub(crate) fn bridge_changes(&self) -> tokio::sync::watch::Receiver<Option<Arc<BridgeClient>>> {
        self.bridge.subscribe()
    }

    /// Run `task` in the background, tracked so the server's shutdown ends it.
    fn spawn_background(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        let mut background = self.background.lock().expect("background tasks");
        // Reap the finished ones, so the set holds only what is running.
        while background.try_join_next().is_some() {}
        background.spawn(task);
    }

    /// Give up on opening a stream to a peer after `timeout` rather than [`OPEN_TIMEOUT`]:
    /// for tests, which cannot wait that long for a peer that never answers.
    pub fn with_open_timeout(mut self, timeout: std::time::Duration) -> SyncRuntime {
        self.open_timeout = timeout;
        self
    }

    /// Back off from `first` rather than [`RECONNECT_BACKOFF`] while the bridge cannot be
    /// reached: for tests of a backoff that has grown long, which cannot wait for it to.
    #[cfg(test)]
    pub(crate) fn with_reconnect_backoff(mut self, first: std::time::Duration) -> SyncRuntime {
        self.reconnect_first = first;
        self
    }

    /// Tell the runtime which app server owns these workspaces' caches.
    ///
    /// Called by [`AppServer::sync`](crate::AppServer::sync). First writer wins: a runtime
    /// belongs to the server that wired it.
    /// Embed through the bridge at `endpoint` rather than the host's standard one: for a
    /// fixture running several hosts in one process.
    pub fn with_bridge_endpoint(mut self, endpoint: sapphire_ipc::Endpoint) -> SyncRuntime {
        self.bridge_endpoint = Some(endpoint);
        self
    }

    /// Pace the primary device's backfill differently: for tests, which cannot wait ten
    /// minutes.
    pub fn set_backfill_timing(&self, timing: BackfillTiming) {
        *self.backfill.lock().expect("backfill timing") = timing;
    }

    fn backfill_timing(&self) -> BackfillTiming {
        *self.backfill.lock().expect("backfill timing")
    }

    pub(crate) fn set_host(&self, host: Arc<WorkspaceHost>) {
        let _ = self.host.set(host);
    }

    /// Read `settings` again whenever a workspace's files change under this runtime — a
    /// session wrote them, or someone edited them — so workspace keys follow the marker's
    /// `config.toml`. Replaced, not kept, by a later call (a restarted server's).
    pub(crate) fn set_settings(&self, settings: Option<Settings>) {
        *self.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
    }

    /// Read the settings again after the files under a workspace changed. Cheap — two
    /// small files — and holding no lock past the read, so it runs on every change.
    fn refresh_settings(&self) {
        let settings = self
            .settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(settings) = settings
            && settings.refresh().is_err()
        {
            // The error may quote the file; the change itself went through.
            tracing::warn!("could not read the settings again after a change to the files");
        }
    }

    /// Start syncing `root`, returning its identity across devices.
    ///
    /// Idempotent: enabling an already-synced workspace returns the same id and changes
    /// nothing.
    pub async fn enable(self: &Arc<Self>, root: &Path) -> Result<GrainId> {
        let key = root.canonicalize().map_err(Error::Io)?;
        let watch_key = key.clone();
        let workspace_id = sync_id(self.ctx.app_name, &key)?;

        // Without a bridge there is no device id to open the replica with, and nobody to
        // register with: keep the workspace for the reconnection to enable (#244).
        {
            let mut pending = self.pending.lock().await;
            if self.bridge().is_err() {
                pending.insert(key, workspace_id);
                return Err(Error::BridgeDisconnected);
            }
            pending.remove(&key);
        }

        // The map stays locked across opening the replica, so two simultaneous `enable`s for
        // one root cannot both open it — the store is a redb database, and it is opened once.
        {
            let mut synced = self.synced.lock().await;
            if let Some(existing) = synced.get(&key) {
                return Ok(existing.workspace_id);
            }
            // A replica disabled a moment ago may still be open; it is this root's, with this
            // runtime's one device id, so it is taken back rather than opened a second time.
            let replica = match self.reclaim(&key) {
                Reclaimed::Open(replica) => replica,
                reclaimed => {
                    let device_id = self.device_id().await?;
                    let state_dir = self.state_dir(&key)?;
                    std::fs::create_dir_all(&state_dir).map_err(Error::Io)?;
                    let config =
                        ReplicaConfig::new(self.ctx.app_name, key.clone(), device_id, &state_dir);
                    // The last holder let go between the two: its store closes as the replica
                    // drops, just after it can no longer be taken back. Give it that moment.
                    let mut tries = match reclaimed {
                        Reclaimed::Closing => CLOSING_RETRIES,
                        _ => 0,
                    };
                    let replica = loop {
                        match Replica::open(config.clone(), Arc::new(SystemClock)) {
                            Ok(replica) => break replica,
                            Err(_) if tries > 0 => {
                                tries -= 1;
                                tokio::time::sleep(CLOSING_RETRY_DELAY).await;
                            }
                            Err(err) => return Err(Error::Sync(err.to_string())),
                        }
                    };
                    Arc::new(Mutex::new(replica))
                }
            };
            synced.insert(
                key.clone(),
                Synced {
                    workspace_id,
                    replica,
                    paused: None,
                    last_error: None,
                },
            );
        }

        // Watch the new root too, so an edit made outside the server from now on is seen.
        // Not fatal if it fails: the app server's own writes take the exact path in
        // `handlers.rs`, and the next `watch_changes` re-reads the root set anyway.
        if let Some(watcher) = self.watcher.get()
            && let Err(err) = watcher.watch(&watch_key)
        {
            tracing::warn!(root = %watch_key.display(), "could not watch: {err}");
        }

        // Register only once the replica is open, so a registration never names a workspace
        // that cannot serve a session.
        self.reregister().await?;

        // Dial now, so enabling a workspace converges it rather than waiting for the next
        // local edit to trigger a session. This is what catches a host up when it returns:
        // the peer that has been running is the one the returning host has to ask, and no
        // watcher on either side fires for a file that was never written locally.
        //
        // Best-effort: a workspace is enabled whether or not any peer is up, and a failure
        // here is not a failure to enable. A session is symmetric, so dialing is enough —
        // this side sends its own view, and the peer answers with what this side lacks.
        //
        // In the background, so the answer does not wait on it: a peer that is offline
        // holds a dial up to `open_timeout`, longer than a client waits for `sync.enable`
        // (#228).
        //
        // Scan first: a file already on disk when the workspace is enabled — a folder that
        // existed before sync was turned on, or an edit made while the server was down —
        // fires no watcher, so without this it is never recorded and the session has
        // nothing to send. Logged, not fatal: the session still fetches what peers have.
        let runtime = Arc::clone(self);
        let mut dials = self.first_dials.lock().expect("first dials");
        // Reap the finished ones, so the set holds only what is running.
        while dials.try_join_next().is_some() {}
        dials.spawn(async move {
            if let Err(err) = runtime.scan(&key).await {
                tracing::warn!(root = %key.display(), "the first scan after enabling failed: {err}");
            }
            if let Err(err) = runtime.sync_now(&key).await {
                tracing::warn!(root = %key.display(), "the first session after enabling failed: {err}");
            }
        });
        Ok(workspace_id)
    }

    /// Stop syncing `root`. Files and the sync id stay, so re-enabling rejoins the same
    /// workspace rather than creating a second one.
    ///
    /// Something that began before — a scan, a session, the first dial — may still hold the
    /// replica open; a re-enable meanwhile takes it back (see [`reclaim`](Self::reclaim)).
    pub async fn disable(&self, root: &Path) -> Result<()> {
        let (replica, unregistered) = self.stop(root).await;
        if let Some(replica) = replica {
            let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
            let mut stopped = self.stopped.lock().expect("stopped replicas");
            // Forget the ones nothing holds any more, so the map stays small.
            stopped.retain(|_, replica| replica.strong_count() > 0);
            stopped.insert(key, Arc::downgrade(&replica));
        }
        unregistered
    }

    /// Stop syncing `root`, like [`disable`](Self::disable), then wait up to `limit` until
    /// nothing holds its replica store open any more — a session, a scan or an embedding
    /// pass that started before may still have it. A store about to be moved or deleted
    /// must be closed first.
    ///
    /// Only a store still in use at `limit` is an error. The bridge not hearing of it is
    /// logged, not returned: the store is closed all the same, and the next registration
    /// replaces the bridge's view.
    pub async fn release(&self, root: &Path, limit: std::time::Duration) -> Result<()> {
        let (replica, unregistered) = self.stop(root).await;
        if let Err(err) = unregistered {
            tracing::warn!(root = %root.display(), "stopping sync: {err}");
        }
        let Some(replica) = replica else {
            return Ok(());
        };
        let deadline = tokio::time::Instant::now() + limit;
        while Arc::strong_count(&replica) > 1 {
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Sync(format!(
                    "the replica of {} is still in use; try again",
                    root.display()
                )));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Ok(())
    }

    /// Stop syncing `root`, handing back its replica — `None` when it was not synced — and
    /// how telling the bridge went.
    async fn stop(&self, root: &Path) -> (Option<Arc<Mutex<Replica>>>, Result<()>) {
        // A root that no longer exists cannot be canonicalized, but the keys of `synced` are
        // canonical already, so the path as given (a registry root) still names the entry.
        let key = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        self.pending.lock().await.remove(&key);
        let removed = self.synced.lock().await.remove(&key);
        if removed.is_some()
            && let Some(watcher) = self.watcher.get()
        {
            watcher.unwatch(&key);
        }
        // Any live session for a workspace this server no longer holds is closed: it would
        // otherwise keep pushing files for a workspace nobody asked about any more.
        if let Some(peers) = self.live.lock().await.remove(&key) {
            peers.drop_connections().await;
        }
        let Some(removed) = removed else {
            return (None, Ok(()));
        };
        let unregistered = async {
            // A bridge that is gone has forgotten this server's registrations already, and
            // the reconnection registers only what is still synced.
            let Ok(bridge) = self.bridge() else {
                return Ok(());
            };
            bridge
                .unregister(removed.workspace_id)
                .await
                .map_err(|e| Error::Bridge(e.to_string()))?;
            self.reregister().await
        }
        .await;
        (Some(removed.replica), unregistered)
    }

    /// The workspaces the workgroup knows about.
    ///
    /// Read straight from the bridge, so it reflects every device's registrations. The
    /// bridge being down is not an empty answer — that would read as "nothing to map" — so
    /// it is an error, and `sync.map` reports it.
    pub async fn workspaces(&self) -> Result<WorkspacesResult> {
        self.bridge()?
            .workspaces()
            .await
            .map_err(|e| Error::Bridge(e.to_string()))
    }

    /// Place a workgroup workspace at a directory of this host: write its identity and
    /// map there, returning the canonical root. [`Current::map`](crate::Current::map)
    /// then selects it and syncs it.
    ///
    /// `selector` is the workspace's name or id, as [`workspaces`] lists it; `dir` must
    /// exist. Its marker is made when it has none, and nothing else: the marker's
    /// `config.toml` is the application's, and arrives with the first sync — a local one
    /// written first would meet it as a conflict (#253).
    ///
    /// [`workspaces`]: SyncRuntime::workspaces
    pub async fn place(&self, selector: &str, dir: &Path) -> Result<PathBuf> {
        let workgroup = self.workspaces().await?;
        let wanted = workgroup
            .workspaces
            .iter()
            // A name is the user-facing handle; the id is the exact one. Two workspaces
            // may not share a name, so a name names at most one.
            .find(|w| w.name == selector || w.workspace_id.to_string() == selector)
            .ok_or_else(|| Error::UnknownWorkspaceName(selector.to_owned()))?;

        // Refuse another application's workspace: this server syncs its own application's
        // workspaces, and `Workspace::from_root` would later refuse the directory anyway.
        if wanted.app_name != self.ctx.app_name {
            return Err(Error::WrongApp {
                name: wanted.name.clone(),
                app_name: wanted.app_name.clone(),
            });
        }

        let root = dir.canonicalize().map_err(Error::Io)?;
        let marker = root.join(format!(".{}", self.ctx.app_name));
        if !marker.is_dir() {
            std::fs::create_dir(&marker).map_err(Error::Io)?;
        }
        let workspace = Workspace::from_root(self.ctx, &root)?;
        if workspace.root != root {
            return Err(Error::UnknownWorkspace(root, self.ctx.app_name));
        }

        // A directory already syncing under another identity is not remapped: the sync id
        // is a device's word that it is the same workspace, and rewriting it silently
        // would make two hosts disagree about what they share.
        {
            let synced = self.synced.lock().await;
            if let Some(entry) = synced.get(&root)
                && entry.workspace_id != wanted.workspace_id
            {
                return Err(Error::SyncId(format!(
                    "{} already syncs as {}, not {}",
                    root.display(),
                    entry.workspace_id,
                    wanted.workspace_id
                )));
            }
        }

        // The identity arrives from the workgroup, not from this host: two hosts that map
        // the same workspace must sync as one, so the id is the one the workgroup lists,
        // written before `enable` reads it. A different id already on disk is refused
        // above; an absent one is written here.
        let marker = workspace.marker_dir();
        let id_path = marker.join(crate::sync::id::SYNC_ID_FILE);
        match std::fs::read_to_string(&id_path) {
            Ok(existing) if existing.trim() != wanted.workspace_id.to_string() => {
                return Err(Error::SyncId(format!(
                    "{} already holds identity {}, not {}",
                    root.display(),
                    existing.trim(),
                    wanted.workspace_id
                )));
            }
            Err(_) => {
                std::fs::write(&id_path, format!("{}\n", wanted.workspace_id))
                    .map_err(Error::Io)?;
            }
            Ok(_) => {}
        }

        // The map is what tells this application, on a later start, which workgroup
        // workspace a directory is.
        let map_path = root
            .join(format!(".{}", self.ctx.app_name))
            .join(WORKSPACE_MAP_FILE);
        let workspace_id = wanted.workspace_id.to_string();
        std::fs::write(&map_path, format!("{workspace_id}\n")).map_err(Error::Io)?;
        Ok(root)
    }

    /// What `sync.status` answers with.
    pub async fn status(&self, root: &Path) -> SyncStatus {
        let bridge = self.bridge().ok();
        let bridge_available = match &bridge {
            Some(bridge) => bridge.status().await.is_ok(),
            None => false,
        };
        let answer = match &bridge {
            Some(_) => self.peers().await.ok(),
            None => None,
        };
        let peers = answer
            .as_ref()
            .map_or(0, |p| p.peers.len().saturating_sub(1));
        let Ok(key) = root.canonicalize() else {
            return SyncStatus {
                enabled: false,
                workspace_id: None,
                peers,
                paused: None,
                last_error: None,
                bridge_available,
                topology: proto::Topology::Mesh,
                embedding: None,
            };
        };
        let synced = self.synced.lock().await;
        match synced.get(&key) {
            Some(entry) => SyncStatus {
                enabled: true,
                workspace_id: Some(entry.workspace_id),
                peers,
                paused: entry.paused.map(|r| format!("{r:?}")),
                last_error: entry.last_error.clone(),
                bridge_available,
                topology: topology::topology(
                    answer
                        .as_ref()
                        .and_then(|p| p.roles_for(entry.workspace_id)),
                ),
                embedding: self.embedding_progress(&key).await,
            },
            None => {
                drop(synced);
                // Asked to sync while the bridge was away: it is enabled, waiting for the
                // bridge, and `bridge_available` says why nothing moves.
                let waiting = self.pending.lock().await.get(&key).copied();
                SyncStatus {
                    enabled: waiting.is_some(),
                    workspace_id: waiting,
                    peers,
                    paused: None,
                    last_error: None,
                    bridge_available,
                    topology: proto::Topology::Mesh,
                    embedding: None,
                }
            }
        }
    }

    /// How far `key`'s embedding has got, when this host embeds it. Reads the open
    /// workspace's index; never opens one, so a status call stays cheap.
    async fn embedding_progress(&self, key: &Path) -> Option<proto::EmbeddingProgress> {
        let host = self.host.get()?;
        let backend = host.open_backend(key)?;
        let state = backend.state();
        state.embedder()?;
        let info = state.db_info().ok()?;
        Some(proto::EmbeddingProgress {
            vectors: info.vector_count,
            pending: info.pending_count,
            running: self.embed_running.lock().await.contains(key),
        })
    }

    /// Bring the replica's view of the files up to date.
    ///
    /// Called straight after the app server's own writes, and by the watcher for everything
    /// else. A scan that finds nothing is cheap; a scan that is skipped loses an edit until
    /// the next one.
    pub async fn scan(&self, root: &Path) -> Result<()> {
        let Ok(key) = root.canonicalize() else {
            return Ok(());
        };
        let replica = {
            let synced = self.synced.lock().await;
            match synced.get(&key) {
                Some(entry) => Arc::clone(&entry.replica),
                None => return Ok(()),
            }
        };
        let (outcome, updates) = {
            let mut replica = replica.lock().await;
            let outcome = replica.scan().map_err(|e| Error::Sync(e.to_string()))?;
            // What the scan committed, as path updates, so it can go out on the sessions
            // that are already open. This is the whole point of the exact-path scan in
            // `handlers.rs`: a write through the server reaches a peer live, not at the next
            // dial.
            let updates = match &outcome {
                ScanOutcome::Scanned(report) => {
                    // The unit of replication is the path state (design doc section 2.4):
                    // the `seen` a peer prunes against is the *path's* version vector - the
                    // context the recorded entry was built on plus its own dot - never the
                    // whole replica vv. Sending the global vv here made receivers prune
                    // causally-related versions they still held: a conflict copy on the
                    // first remote edit, and tombstones pruned so deletes stopped
                    // propagating (issue #157).
                    report
                        .recorded
                        .iter()
                        .map(|entry| PathUpdate {
                            path: entry.path.clone(),
                            versions: vec![entry.clone()],
                            seen: {
                                let mut seen = entry.context.clone();
                                seen.add_dot(&entry.dot);
                                seen
                            },
                        })
                        .collect()
                }
                ScanOutcome::Paused(_) => Vec::new(),
            };
            (outcome, updates)
        };
        {
            let mut synced = self.synced.lock().await;
            if let Some(entry) = synced.get_mut(&key) {
                entry.paused = match outcome {
                    ScanOutcome::Paused(reason) => Some(reason),
                    ScanOutcome::Scanned(_) => None,
                };
            }
        }
        // After the map is unlocked: a push awaits every peer, and the table must not be
        // held behind a slow one.
        self.after_commit(&key, updates).await;
        Ok(())
    }

    /// Bring the search index up to date with files a session has written.
    ///
    /// A replication session moves files, not index rows: the index belongs to the app
    /// server, and the sync core is deliberately free of it. So the side that *applied* a
    /// change re-indexes, which is what makes a file that arrived from a peer searchable
    /// without a manual reindex — the claim the whole architecture rests on.
    ///
    /// Incremental, not a full rebuild: the workspace's mtime/size stamps mean only what
    /// actually changed is read back. A file the session wrote has a new stamp, so it is
    /// picked up; one that was already indexed and untouched is skipped.
    ///
    /// Best-effort by contract: the caller's session has already succeeded, and a failure
    /// here must not be reported as the session's. It is logged and the next session, local
    /// edit or explicit `workspace.reindex` tries again.
    async fn reindex(&self, root: &Path) {
        // The marker's `config.toml` may be among what arrived.
        self.refresh_settings();
        let Some(host) = self.host.get() else {
            return;
        };
        let host = Arc::clone(host);
        let root = root.to_owned();
        // One sweep at a time: two sessions that finish together would otherwise both walk
        // the workspace through the same retrieve store.
        let _serialised = self.reindexing.lock().await;
        // Timed from here, so a run waiting for the one before it is not charged its wait.
        let started = std::time::Instant::now();
        // Opening the workspace is itself blocking work, and it is where the exclusive
        // retrieve-store lock is taken, so it happens here rather than inside the sweep.
        let backend = match host.backend_at(&root).await {
            Ok(backend) => backend,
            Err(err) => {
                crate::instrument::reindexed(started, false);
                tracing::warn!(root = %root.display(), "re-indexing after a session failed: {err}");
                return;
            }
        };
        let state = Arc::clone(backend.state());
        let result = tokio::task::spawn_blocking(move || state.sync_retrieve()).await;
        crate::instrument::reindexed(started, matches!(result, Ok(Ok(_))));
        match result {
            Ok(Ok(report)) => {
                tracing::debug!(root = %root.display(), ?report, "re-indexed after a session");
            }
            Ok(Err(err)) => {
                tracing::warn!(root = %root.display(), "re-indexing after a session failed: {err}");
            }
            Err(err) => {
                tracing::warn!(root = %root.display(), "re-indexing after a session failed: {err}");
            }
        }
    }

    /// Run an embedding pass over `root` in the background, after the pass already
    /// waiting for it if there is one (#187).
    ///
    /// The pass reads the vector files that arrived, and computes the missing ones this
    /// host is responsible for: the files whose winning version this replica wrote, and —
    /// on the primary device — every other one. It never holds up the caller.
    pub fn request_embed(self: &Arc<Self>, root: &Path) {
        let Ok(key) = root.canonicalize() else {
            return;
        };
        let runtime = Arc::clone(self);
        self.spawn_background(async move {
            if !runtime.embed_wanted.lock().await.insert(key.clone()) {
                return;
            }
            let _one_at_a_time = runtime.embedding.lock().await;
            // Taken off the list as the pass starts: a change during the pass asks again.
            runtime.embed_wanted.lock().await.remove(&key);
            runtime.embed_running.lock().await.insert(key.clone());
            runtime.embed_pass(&key).await;
            runtime.embed_running.lock().await.remove(&key);
        });
    }

    /// One embedding pass over `key`. Best-effort, like the re-index: logged, never fatal.
    async fn embed_pass(&self, key: &Path) {
        let Some(host) = self.host.get() else {
            return;
        };
        let (workspace_id, replica) = {
            let synced = self.synced.lock().await;
            match synced.get(key) {
                Some(entry) => (entry.workspace_id, Arc::clone(&entry.replica)),
                None => return,
            }
        };
        let backend = match host.backend_at(key).await {
            Ok(backend) => backend,
            Err(err) => {
                tracing::warn!(root = %key.display(), "embedding pass: {err}");
                return;
            }
        };
        let state = Arc::clone(backend.state());
        if let Err(err) = state
            .refresh_embedder_at(self.bridge_endpoint.clone())
            .await
        {
            tracing::warn!(root = %key.display(), "embedding pass: {err}");
            return;
        }
        if state.embedder().is_none() {
            return;
        }
        let primary = self.is_primary(workspace_id).await;
        // The replica's view, read once: which content each path has, and which of those
        // versions this replica wrote.
        let grace_ms = self.backfill_timing().grace.as_millis() as u64;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        let (mine, settled, live) = {
            let replica = replica.lock().await;
            let me = replica.replica_id();
            let states = match replica.states() {
                Ok(states) => states,
                Err(err) => {
                    tracing::warn!(root = %key.display(), "embedding pass: {err}");
                    return;
                }
            };
            let mut mine = HashMap::new();
            // Other devices' versions old enough for the primary to fill in.
            let mut settled = HashMap::new();
            let mut live = std::collections::HashSet::new();
            for (path, path_state) in states {
                let winner = path_state.winner();
                let Some(hash) = winner.content.hash() else {
                    continue;
                };
                let hash = hash.to_hex();
                if winner.dot.replica == me {
                    mine.insert(path, hash.clone());
                } else if now_ms.saturating_sub(winner.hlc.wall_ms) >= grace_ms {
                    settled.insert(path, hash.clone());
                }
                live.insert(hash);
            }
            (mine, settled, live)
        };
        let root = key.to_owned();
        let result = tokio::task::spawn_blocking(move || {
            // The author embeds its own versions at once; the primary device fills in other
            // devices' versions once they have settled, racing nobody.
            let policy = |rel: &str, hash: &str| {
                mine.get(rel).is_some_and(|h| h.as_str() == hash)
                    || (primary && settled.get(rel).is_some_and(|h| h.as_str() == hash))
            };
            let embedded = state.embed_pending_with(&policy, PASS_BATCH)?;
            let removed = if primary {
                state.remove_stale_vectors(&live, STALE_VECTOR_AGE)
            } else {
                0
            };
            Ok::<_, sapphire_workspace::Error>((embedded, removed))
        })
        .await;
        match result {
            Ok(Ok((embedded, removed))) if embedded > 0 || removed > 0 => tracing::info!(
                root = %root.display(),
                embedded,
                removed,
                "embedding pass"
            ),
            Ok(Ok(_)) => {}
            Ok(Err(err)) => tracing::warn!(root = %root.display(), "embedding pass: {err}"),
            Err(err) => tracing::warn!(root = %root.display(), "embedding pass: {err}"),
        }
    }

    /// Open a session with every peer that will take one.
    pub async fn sync_now(self: &Arc<Self>, root: &Path) -> Result<()> {
        let Ok(key) = root.canonicalize() else {
            return Ok(());
        };
        let workspace_id = {
            let synced = self.synced.lock().await;
            match synced.get(&key) {
                Some(entry) => entry.workspace_id,
                None => return Ok(()),
            }
        };
        let peers = self.peers().await?;
        let me = self.device_id().await?;

        // The same direction rule the dial loop follows: a host only dials peers with a
        // greater device id. Dialling the other way from here would reintroduce the
        // dial-both-ways deadlock the loop was cured of, just on a different trigger.
        // Inside a star, a pair with neither hub in it does not link at all.
        let roles = peers.roles_for(workspace_id).cloned();
        // Every peer at once, so one that is offline costs the others nothing (#228). On
        // this task, not spawned: cancelling the caller — the server's shutdown aborting the
        // watcher — must drop every dial with it, since each one pins the runtime.
        let mut dials: Vec<Dial<'_>> = Vec::new();
        for peer in peers
            .peers
            .iter()
            .filter(|p| topology::link(me, p.device_id, roles.as_ref()) == topology::Link::Dial)
            .cloned()
            .collect::<Vec<_>>()
        {
            // A peer that already has a live session is caught up by it: a second exchange
            // on the same workspace would be redundant, and a second stream between the same
            // pair is exactly what the tie-break cannot settle.
            if let Some(live) = self.live_peers(&key).await
                && live.has(&peer.device_id).await
            {
                continue;
            }
            // One dial per peer at a time. `sync_now` runs beside the dial loop, and two
            // overlapping outbound dials to one peer are the one case the tie-break rule
            // cannot resolve — each end would see its own stream as `outbound`. Whoever
            // arrives second simply leaves it to the next pass. The table is made here if
            // the dial loop has not yet: the first dial after enabling must not depend on
            // a tick that has not fired.
            let Some((_, table)) = self.live_peers_or_create(&key).await else {
                continue;
            };
            if !table.begin_dial(peer.device_id).await {
                continue;
            }
            let key = &key;
            dials.push(Box::pin(async move {
                // A peer that does not host this workspace refuses, which is normal and cheap.
                let result = self
                    .sync_one(key, &peer.device_id, peer.name.clone(), workspace_id)
                    .await;
                table.end_dial(peer.device_id).await;
                if let Err(err) = result {
                    tracing::debug!(peer = %peer.name, "no session: {err}");
                }
            }));
        }
        join_all(dials).await;
        Ok(())
    }

    /// Dial one peer and adopt the stream as a live session.
    ///
    /// The one-shot path (`run_session`) would close the stream the moment the exchange
    /// finished — while the accepting peer keeps its half open and pushes into it, a session
    /// that dies with nothing reading it. So the dialled side is adopted live too, and both
    /// ends of a pair agree that a session stays open.
    async fn sync_one(
        self: &Arc<Self>,
        key: &Path,
        device: &GrainId,
        name: String,
        workspace_id: GrainId,
    ) -> Result<()> {
        let stream = self.open_stream(workspace_id, *device).await?;
        self.adopt_live_session(stream, key, *device, workspace_id)
            .await
            .map_err(|err| {
                tracing::warn!(peer = %name, "a live session could not be started: {err}");
                err
            })
    }

    /// Ask the bridge for a stream to `device`, giving up after `open_timeout`.
    async fn open_stream(
        &self,
        workspace_id: GrainId,
        device: GrainId,
    ) -> Result<sapphire_ipc::RawStream> {
        match tokio::time::timeout(
            self.open_timeout,
            self.bridge()?.open_stream(workspace_id, device),
        )
        .await
        {
            Ok(opened) => Ok(opened?),
            Err(_) => Err(Error::Bridge(format!(
                "no stream to {device} within {} s; it may be offline",
                self.open_timeout.as_secs_f32()
            ))),
        }
    }

    /// Answer the bridge's announcements, following the connection across reconnections
    /// (#244). Runs until the task is aborted.
    pub async fn run(self: Arc<Self>) -> Result<()> {
        let mut slot = self.bridge.subscribe();
        loop {
            let bridge = slot.borrow_and_update().clone();
            match bridge {
                Some(bridge) if !bridge.is_closed() => self.answer(&bridge, &mut slot).await,
                // Wait for a connection. The runtime holds the sender, so this never ends.
                _ => {
                    if slot.changed().await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Answer `bridge`'s announcements until it closes or another connection replaces it.
    async fn answer(
        self: &Arc<Self>,
        bridge: &Arc<BridgeClient>,
        slot: &mut tokio::sync::watch::Receiver<Option<Arc<BridgeClient>>>,
    ) {
        let early = self
            .subscribed
            .lock()
            .expect("subscribed")
            .take_if(|(b, _)| Arc::ptr_eq(b, bridge));
        let mut incoming = match early {
            Some((_, incoming)) => incoming,
            None => bridge.incoming(),
        };
        loop {
            let received = tokio::select! {
                received = incoming.recv() => received,
                () = bridge.closed() => return,
                _ = slot.changed() => return,
            };
            let announcement = match received {
                Ok(a) => a,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(missed = n, "fell behind on bridge announcements");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };

            let found = {
                let synced = self.synced.lock().await;
                synced
                    .iter()
                    .find(|(_, s)| s.workspace_id == announcement.workspace_id)
                    .map(|(root, s)| (root.clone(), Arc::clone(&s.replica)))
            };
            // `replica` is still what the accepted session shares, and `root` is where its
            // files and its index live.
            let Some((root, replica)) = found else {
                // The bridge routed to us for a workspace we no longer hold. Not fatal:
                // ignore it and let the ticket expire.
                tracing::debug!(
                    workspace = %announcement.workspace_id,
                    "an announcement for a workspace this server does not hold"
                );
                continue;
            };

            let bridge = Arc::clone(bridge);
            let workspace_id = announcement.workspace_id;
            let peer = announcement.peer_device_id;
            let driver = Arc::clone(self);
            self.spawn_background(async move {
                let stream = match bridge.accept_stream(announcement.ticket).await {
                    Ok(stream) => stream,
                    Err(err) => {
                        tracing::warn!("could not claim an announced stream: {err}");
                        return;
                    }
                };
                if driver.should_skip(workspace_id, peer).await {
                    tracing::debug!(%peer, "dropped an inbound session: both ends are outside this workspace's star");
                    return;
                }
                // Live, not one-shot: the peer that dialled is keeping this session open and
                // pushing what it commits, so this side has to keep reading. A session that
                // closed after the exchange would leave the dialler pushing into a stream
                // nobody reads.
                let (_, session) = match sapphire_framework_session::open_live_session(
                    stream,
                    Arc::clone(&replica),
                    workspace_id,
                )
                .await
                {
                    Ok(started) => started,
                    Err(err) => {
                        crate::instrument::sync_session(false);
                        tracing::warn!("an inbound session failed: {err}");
                        return;
                    }
                };
                let Some((key, table)) = driver.live_peers_or_create(&root).await else {
                    return;
                };
                // The peer dialled: this stream survives only if the *peer* has the lower
                // device id. A stream that loses is dropped, which closes it, and the
                // connection ends tidily rather than being left half-read.
                let Some(updates) = table.insert(peer, session, false).await else {
                    return;
                };
                table.catch_up(peer, &replica).await;
                crate::instrument::sync_session(true);
                // This is the receiving side: the files are on disk now and nobody else will
                // index them.
                driver.reindex(&key).await;
                driver.request_embed(&key);
                driver.spawn_reader(table, updates, key, peer);
            });
        }
    }

    /// Make the first attempt to reach the bridge, when the runtime has a
    /// [`BridgeConnector`] and is not connected yet.
    ///
    /// No bridge running is not an error: the server serves without sync, and
    /// [`keep_connected`](Self::keep_connected) picks the bridge up once it starts. Any
    /// other failure — a bridge of another API version, say — is, and stops the start.
    pub async fn connect(self: &Arc<Self>) -> Result<()> {
        let Some(connector) = &self.connector else {
            return Ok(());
        };
        if self.is_connected() {
            return Ok(());
        }
        match connector.connect().await? {
            Some(client) => self.attach(client).await,
            None => tracing::warn!(
                "no sapphire-bridge is running; serving without sync until it starts"
            ),
        }
        Ok(())
    }

    /// Stay connected to the bridge: wait for the connection to close, then reach the
    /// bridge again with the runtime's [`BridgeConnector`], backing off from
    /// [`RECONNECT_BACKOFF`] to [`RECONNECT_BACKOFF_MAX`] while it cannot (#244).
    /// [`reconnect_now`](Self::reconnect_now) ends a wait early and starts the backoff over.
    ///
    /// Runs until the task is aborted, which is what the app server's shutdown does with
    /// it. Without a connector it only marks the runtime disconnected once the connection
    /// closes, and returns.
    pub async fn keep_connected(self: Arc<Self>) {
        let mut attempt = 0;
        // Whether this outage's failure has been logged at warn: once per outage, however
        // often `reconnect_now` starts the backoff over.
        let mut warned = false;
        loop {
            if let Ok(bridge) = self.bridge() {
                bridge.closed().await;
                tracing::warn!("lost the connection to sapphire-bridge; reconnecting");
                self.bridge.send_if_modified(|slot| {
                    let same = slot.as_ref().is_some_and(|b| Arc::ptr_eq(b, &bridge));
                    if same {
                        *slot = None;
                    }
                    same
                });
                attempt = 0;
                warned = false;
                continue;
            }
            let Some(connector) = &self.connector else {
                return;
            };
            match connector.connect().await {
                Ok(Some(client)) => {
                    tracing::info!("connected to sapphire-bridge");
                    self.attach(client).await;
                    attempt = 0;
                    warned = false;
                    continue;
                }
                Ok(None) => tracing::debug!("no sapphire-bridge is running yet"),
                // Logged once per outage, not once per attempt.
                Err(err) if !warned => {
                    tracing::warn!("could not connect to sapphire-bridge: {err}");
                    warned = true;
                }
                Err(err) => tracing::debug!("could not connect to sapphire-bridge: {err}"),
            }
            tokio::select! {
                () = tokio::time::sleep(reconnect_backoff(self.reconnect_first, attempt)) => {
                    attempt = attempt.saturating_add(1);
                }
                () = self.reconnect_now.notified() => attempt = 0,
            }
        }
    }

    /// Start using `client`, and bring the bridge up to date: it knows nothing of this
    /// server yet, whether it has just started or this server has.
    async fn attach(self: &Arc<Self>, client: BridgeClient) {
        // Swapped under the pending lock, so an `enable` racing this either sees the new
        // connection or leaves its workspace for the drain below.
        let client = Arc::new(client);
        *self.subscribed.lock().expect("subscribed") =
            Some((Arc::clone(&client), client.incoming()));
        let waiting = {
            let mut pending = self.pending.lock().await;
            self.bridge.send_replace(Some(client));
            std::mem::take(&mut *pending)
        };
        // What was synced before the connection was lost.
        if !self.synced.lock().await.is_empty()
            && let Err(err) = self.reregister().await
        {
            tracing::warn!("could not register with sapphire-bridge: {err}");
        }
        // What was asked for while it was away. `enable` registers and dials each one.
        for root in waiting.into_keys() {
            if let Err(err) = self.enable(&root).await {
                tracing::warn!(root = %root.display(), "could not start sync: {err}");
            }
        }
    }

    /// Every synced root, for the watcher.
    pub async fn roots(&self) -> Vec<PathBuf> {
        self.synced.lock().await.keys().cloned().collect()
    }

    /// The live session table for `root`, if this runtime syncs it.
    ///
    /// `None` is not an error: a dial, a push or a drop for a workspace this server does not
    /// hold is a no-op, and every caller below treats it that way.
    async fn live_peers(&self, root: &Path) -> Option<Arc<LivePeers>> {
        let key = root.canonicalize().ok()?;
        self.live.lock().await.get(&key).cloned()
    }

    /// The live session table for `root`, creating it if this runtime syncs it.
    ///
    /// The two locks are taken one after the other, never nested: `synced` to decide whether
    /// the workspace is ours at all, then `live` to get or make its table.
    async fn live_peers_or_create(&self, root: &Path) -> Option<(PathBuf, Arc<LivePeers>)> {
        let key = root.canonicalize().ok()?;
        if !self.synced.lock().await.contains_key(&key) {
            return None;
        }
        // A table is built by whoever walks the workspace first, and it needs this host's
        // device id — the tie-break between two simultaneous streams is decided from it.
        let me = self.device_id().await.ok()?;
        let mut live = self.live.lock().await;
        let peers = Arc::clone(
            live.entry(key.clone())
                .or_insert_with(|| Arc::new(LivePeers::new(me))),
        );
        Some((key, peers))
    }

    /// Push `updates` to every open session for `root`.
    ///
    /// Called with what a scan recorded. `from` is always `None` here: a local commit came
    /// from this host, so no session is excluded. The forwarding path, which does name a
    /// source, is [`LivePeers::fan_out`](live::LivePeers::fan_out) called directly by the
    /// reader side of a session.
    ///
    /// Best-effort by contract: the commit already happened, and a peer that is gone is the
    /// dialer's problem, not the writer's.
    pub async fn after_commit(&self, root: &Path, updates: Vec<PathUpdate>) {
        if updates.is_empty() {
            return;
        }
        if let Some(peers) = self.live_peers(root).await {
            peers.fan_out(&updates, None).await;
        }
    }

    /// The devices with an open live session for `root`.
    ///
    /// Used by tests to wait for the dialer; empty for a workspace this server does not hold.
    pub async fn live_session_devices(&self, root: &Path) -> Vec<GrainId> {
        match self.live_peers(root).await {
            Some(peers) => peers.devices().await,
            None => Vec::new(),
        }
    }

    /// End the first dials [`enable`](SyncRuntime::enable) left running, waiting until
    /// they are gone.
    ///
    /// For the app server's shutdown: each one pins the runtime, and with it the replica
    /// stores, which are redb databases that stay locked — a dial outliving its server
    /// would lock the next one out.
    pub async fn stop_dials(&self) {
        let mut dials = std::mem::take(&mut *self.first_dials.lock().expect("first dials"));
        dials.abort_all();
        while dials.join_next().await.is_some() {}
    }

    /// End the runtime's background work — inbound sessions, live-session readers and
    /// embedding passes — and wait until it has let go.
    ///
    /// The server's shutdown calls this, so a server restarted in the same process can open
    /// the replica stores again at once rather than whenever the last straggler finished
    /// (#235). An embedding pass already inside its blocking section runs that section to
    /// the end; it holds the workspace's index, never a replica.
    pub async fn stop_background(&self) {
        let mut background =
            std::mem::take(&mut *self.background.lock().expect("background tasks"));
        background.abort_all();
        while background.join_next().await.is_some() {}
    }

    /// Wait for the first dials [`enable`](SyncRuntime::enable) left running to finish on
    /// their own: for a test that must not race them.
    #[cfg(test)]
    async fn settle_dials(&self) {
        let mut dials = std::mem::take(&mut *self.first_dials.lock().expect("first dials"));
        while dials.join_next().await.is_some() {}
    }

    /// Close every open live session, as if the connections had been cut.
    ///
    /// Called for a workspace being disabled, and by tests that want to watch the dialer
    /// rebuild what was dropped.
    pub async fn drop_connections(&self) {
        let tables: Vec<Arc<LivePeers>> = self.live.lock().await.values().cloned().collect();
        for peers in tables {
            peers.drop_connections().await;
        }
    }

    /// Keep a live session open to every peer, for every synced workspace.
    ///
    /// One walk per [`DIAL_INTERVAL`](live::DIAL_INTERVAL): for each synced workspace, ask
    /// the bridge who is in the workgroup, and open a session to every peer that is connected
    /// and does not already have one.
    ///
    /// A peer that cannot be reached is retried on a per-peer exponential backoff capped at
    /// [`DIAL_BACKOFF_MAX`] — one failed attempt per walk, not one per spin — and the counter
    /// resets when a session opens. An unreachable peer is therefore no obstacle to the
    /// others: its dial fails, the walk moves on, and the next workspace is dialled.
    ///
    /// Runs until the task is aborted, which is what the app server's shutdown does with it.
    /// It is allowed to fail like [`run`](SyncRuntime::run): only sync stops, and the app
    /// server keeps serving files (spec §10).
    pub async fn dial_loop(self: Arc<Self>) -> Result<()> {
        let mut tick = tokio::time::interval(live::DIAL_INTERVAL);
        // A missed tick is a late walk, not a burst of them: the interval is a pace, not a
        // quota to make up.
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut waiting: HashMap<GrainId, std::time::Instant> = HashMap::new();
        let mut failures: HashMap<GrainId, u32> = HashMap::new();
        // The workspaces this host was the primary device of at the last walk.
        let mut primary_of: std::collections::HashSet<GrainId> = Default::default();
        // When the primary device last ran a backfill pass with nothing prompting it.
        let mut last_backfill = std::time::Instant::now();
        loop {
            tick.tick().await;
            let roots = self.roots().await;
            if roots.is_empty() {
                continue;
            }
            let peers = match self.peers().await {
                Ok(peers) => peers,
                // The bridge is down or has no workgroup: nothing to dial, and the next walk
                // asks again. Not an error for the caller — the app server is still serving.
                Err(err) => {
                    tracing::debug!("no peers to dial: {err}");
                    continue;
                }
            };
            let me = match self.device_id().await {
                Ok(me) => me,
                Err(err) => {
                    tracing::warn!("no device id, so nobody to dial: {err}");
                    continue;
                }
            };
            // A host that has just become a workspace's primary device owes it the vectors
            // nobody else computes (#187): files that arrived before the election settled
            // would otherwise wait for the next change.
            let ids: Vec<(PathBuf, GrainId)> = self
                .synced
                .lock()
                .await
                .iter()
                .map(|(root, entry)| (root.clone(), entry.workspace_id))
                .collect();
            // And every so often anyway: an author that never embedded leaves files only the
            // primary will fill in, and nothing else would prompt it (#188).
            let backfill_due = last_backfill.elapsed() >= self.backfill_timing().interval;
            if backfill_due {
                last_backfill = std::time::Instant::now();
            }
            for (root, workspace_id) in ids {
                let primary = peers
                    .roles_for(workspace_id)
                    .is_some_and(|r| r.primary == Some(me));
                if !primary {
                    primary_of.remove(&workspace_id);
                } else if primary_of.insert(workspace_id) || backfill_due {
                    self.request_embed(&root);
                }
            }
            let now = std::time::Instant::now();
            // Dial only peers whose device id is greater than ours. Every device dialling
            // every other can deadlock: two hosts that dial each other at once each hold
            // their replica's lock across their own exchange and wait for the other's
            // hello, which the other's dialler holds the lock for. With one direction per
            // pair the wait chain is a DAG — a higher-id host never dials, so it never
            // holds its lock out while a lower-id host's exchange needs it. The lower host
            // dials within `DIAL_INTERVAL`, and the exchange is symmetric, so nothing is
            // lost; the tie-break is left as the safety net it is.
            for root in roots {
                let Some((key, table)) = self.live_peers_or_create(&root).await else {
                    continue;
                };
                let workspace_id = {
                    let synced = self.synced.lock().await;
                    match synced.get(&key) {
                        Some(entry) => entry.workspace_id,
                        None => continue,
                    }
                };
                let roles = peers.roles_for(workspace_id);
                // Entering a star closes what it no longer needs. The hubs carry it all —
                // once one of them is reachable from here (#192).
                if topology::may_close_skipped(me, roles, &table.devices().await) {
                    table
                        .retain(|d| topology::link(me, *d, roles) != topology::Link::Skip)
                        .await;
                }
                for peer in peers.peers.iter().filter(|p| {
                    p.connected && topology::link(me, p.device_id, roles) == topology::Link::Dial
                }) {
                    let device = peer.device_id;
                    if table.has(&device).await {
                        // Already talking; a redial would drop a working session.
                        failures.remove(&device);
                        waiting.remove(&device);
                        continue;
                    }
                    if let Some(until) = waiting.get(&device)
                        && now < *until
                    {
                        // Backing off after a failure; the walk skips it until then.
                        continue;
                    }
                    // The same claim `sync_now` takes: this walk and it can overlap, and a
                    // double dial in one direction is the one race the tie-break cannot
                    // settle. Skipping is free — the next pass, or `sync_now` itself, dials.
                    if !table.begin_dial(device).await {
                        continue;
                    }
                    let opened = self.open_stream(workspace_id, device).await;
                    if opened.is_err() {
                        table.end_dial(device).await;
                    }
                    match opened {
                        Ok(stream) => {
                            let adopted = self
                                .adopt_live_session(stream, &root, device, workspace_id)
                                .await;
                            // The claim is out either way: the attempt is over, and a
                            // session that died mid-exchange is re-dialled next pass.
                            table.end_dial(device).await;
                            match adopted {
                                Ok(()) => {
                                    failures.remove(&device);
                                    waiting.remove(&device);
                                }
                                Err(err) => {
                                    tracing::debug!(
                                        peer = %peer.name,
                                        err = %err,
                                        "a live session could not be started"
                                    );
                                    let count = failures.entry(device).or_insert(0);
                                    *count = count.saturating_add(1);
                                    waiting.insert(device, now + dial_backoff(*count));
                                }
                            }
                        }
                        Err(err) => {
                            // Normal and cheap: a peer that does not host this workspace
                            // refuses, and one that is unreachable cannot be opened.
                            tracing::debug!("no stream to a peer: {err}");
                            let count = failures.entry(device).or_insert(0);
                            *count = count.saturating_add(1);
                            waiting.insert(device, now + dial_backoff(*count));
                        }
                    }
                }
            }
        }
    }

    /// Take ownership of an opened stream: run the exchange, then keep it live.
    ///
    /// The session is stored under `device` and its reader pumped until the stream ends. A
    /// half-open or refused stream is not stored; the dialer retries it with backoff.
    async fn adopt_live_session(
        self: &Arc<Self>,
        stream: sapphire_ipc::RawStream,
        root: &Path,
        device: GrainId,
        workspace_id: GrainId,
    ) -> Result<()> {
        let replica = self.replica_of(root).await?;
        let (_, session) = sapphire_framework_session::open_live_session(
            stream,
            Arc::clone(&replica),
            workspace_id,
        )
        .await
        .map_err(|e| {
            crate::instrument::sync_session(false);
            Error::Sync(e.to_string())
        })?;
        let Some((key, table)) = self.live_peers_or_create(root).await else {
            return Ok(());
        };
        // This stream was dialled by us, so it is the one the pair keeps if this host's
        // device id is the lower of the two. If it lost the tie-break the session is
        // dropped here, which closes the stream, and the surviving one is already in the
        // table.
        let Some(updates) = table.insert(device, session, true).await else {
            return Ok(());
        };
        table.catch_up(device, &replica).await;
        crate::instrument::sync_session(true);
        // The session wrote files during the exchange; the index has to catch up before the
        // workspace is searchable again.
        self.reindex(&key).await;
        self.request_embed(&key);
        self.spawn_reader(Arc::clone(&table), updates, key, device);
        Ok(())
    }

    /// Forward what one session receives, and re-index it.
    ///
    /// This is the loop that makes A → S → B work: a batch that arrived on S's session with A
    /// is applied (the session crate does that before publishing it), the workspace behind
    /// `key` is re-indexed, and the batch is handed to every *other* session whose peer lacks
    /// it — `from` is the device it came from, so it never goes back to A.
    fn spawn_reader(
        self: &Arc<Self>,
        table: Arc<LivePeers>,
        mut updates: broadcast::Receiver<Vec<PathUpdate>>,
        key: PathBuf,
        from: GrainId,
    ) {
        let driver = Arc::clone(self);
        self.spawn_background(async move {
            loop {
                match updates.recv().await {
                    Ok(batch) => {
                        // The files are on disk now (the session guarantees it before
                        // publishing), so the index is behind them until the sweep below.
                        driver.reindex(&key).await;
                        driver.request_embed(&key);
                        table.fan_out(&batch, Some(from)).await;
                    }
                    // A burst bigger than the channel: the newest is kept and the gap is
                    // covered by the next session's exchange. Losing a batch here cannot lose
                    // an entry — the peer's vector does not cover it, so the next dial or push
                    // carries it again.
                    Err(broadcast::error::RecvError::Lagged(missed)) => {
                        tracing::debug!(
                            device = %from,
                            missed,
                            "fell behind on a live session; the next exchange catches up"
                        );
                    }
                    // The session closed. Its table entry is reaped by the next fan-out or
                    // dial pass, and the dialer opens a fresh one.
                    Err(broadcast::error::RecvError::Closed) => {
                        return;
                    }
                }
            }
        });
    }

    /// The replica [`disable`](Self::disable) let go of for `key`, if something still holds it
    /// open.
    fn reclaim(&self, key: &Path) -> Reclaimed {
        let stopped = self.stopped.lock().expect("stopped replicas").remove(key);
        match stopped {
            None => Reclaimed::Closed,
            Some(replica) => replica
                .upgrade()
                .map_or(Reclaimed::Closing, Reclaimed::Open),
        }
    }

    /// The replica of a synced `root`.
    async fn replica_of(&self, root: &Path) -> Result<Arc<Mutex<Replica>>> {
        let key = root.canonicalize().map_err(Error::Io)?;
        let synced = self.synced.lock().await;
        match synced.get(&key) {
            Some(entry) => Ok(Arc::clone(&entry.replica)),
            None => Err(Error::UnknownWorkspace(key, self.ctx.app_name)),
        }
    }

    /// Watch the synced roots and, on a debounced report, scan and dial.
    ///
    /// This is the safety net for everything the app server did not do itself — a file
    /// edited in an editor, a `git checkout`. The exact path is [`scan`] called from
    /// `handlers.rs` right after a write the server made. A scan that finds nothing is
    /// cheap; a scan that is skipped loses an edit until the next one.
    ///
    /// Returns when the reporting channel closes — that is, when the [`Watcher`] this
    /// runtime holds is dropped, which happens when the runtime itself is.
    ///
    /// [`scan`]: SyncRuntime::scan
    pub async fn watch_changes(self: Arc<Self>) -> Result<()> {
        let (tx, mut rx) = mpsc::channel(64);
        let watcher = Arc::new(Watcher::start(self.roots().await, tx)?);
        let _ = self.watcher.set(Arc::clone(&watcher));

        // A root enabled between `roots()` above and the `set` just now was not in the
        // starting set, and `enable` could not reach a watcher that did not exist yet.
        // Watching the current set again is idempotent on `notify`'s side.
        for root in self.roots().await {
            if let Err(err) = watcher.watch(&root) {
                tracing::warn!(root = %root.display(), "could not watch: {err}");
            }
        }

        while let Some(root) = rx.recv().await {
            // An edit of the marker's `config.toml` changes the workspace's settings.
            self.refresh_settings();
            // Logged, not fatal: a failed scan or dial leaves the replica behind, and the
            // next report tries again. Neither may take the app server down (spec §10).
            if let Err(err) = self.scan(&root).await {
                tracing::warn!(root = %root.display(), "scan after a local edit failed: {err}");
            }
            self.request_embed(&root);
            if let Err(err) = self.sync_now(&root).await {
                tracing::warn!(root = %root.display(), "dial after a local edit failed: {err}");
            }
        }
        Ok(())
    }

    /// Tell the bridge the complete current set. A registration is not a delta.
    async fn reregister(&self) -> Result<()> {
        let workspaces: Vec<WorkspaceRegistration> = {
            let synced = self.synced.lock().await;
            synced
                .iter()
                .map(|(root, entry)| WorkspaceRegistration {
                    workspace_id: entry.workspace_id,
                    root: root.clone(),
                })
                .collect()
        };
        self.bridge()?
            .register(RegisterParams {
                app_name: self.ctx.app_name.to_owned(),
                exe_path: self.exe_path.clone(),
                managed_by: wire_managed_by(self.managed_by),
                workspaces,
            })
            .await
            .map_err(|e| Error::Bridge(e.to_string()))?;
        Ok(())
    }

    /// Whether the last roles the bridge reported say this pair does not link.
    ///
    /// Reads the cached device id only: asking the bridge here would register with an empty
    /// workspace list, racing `enable`'s registration. `false` (accept) when it is unknown.
    async fn should_skip(&self, workspace_id: GrainId, peer: GrainId) -> bool {
        let Some(&me) = self.device_id.get() else {
            return false;
        };
        let cached = self.last_peers.lock().await;
        let roles = cached.as_ref().and_then(|p| p.roles_for(workspace_id));
        topology::link(me, peer, roles) == topology::Link::Skip
    }

    /// Whether this host is `workspace_id`'s primary device, by the last roles the
    /// bridge reported. `false` when the bridge has not answered yet, or when this host's
    /// device id is not known yet (no workspace was enabled). Reads the cached id only:
    /// asking the bridge here would register with an empty workspace list, racing
    /// `enable`'s registration.
    pub async fn is_primary(&self, workspace_id: GrainId) -> bool {
        let Some(&me) = self.device_id.get() else {
            return false;
        };
        let cached = self.last_peers.lock().await;
        cached
            .as_ref()
            .and_then(|p| p.roles_for(workspace_id))
            .is_some_and(|r| r.primary == Some(me))
    }

    /// This host's device id, once the bridge has told it (#250).
    ///
    /// `None` until this runtime first registered — no workspace was enabled yet, or the
    /// bridge was not connected when one was. Never asks the bridge: a registration made
    /// only to learn the id would carry an empty workspace list, and briefly clear this
    /// application's routes.
    pub fn this_device(&self) -> Option<GrainId> {
        self.device_id.get().copied()
    }

    /// The workgroup's devices, this host's included, and the roles the bridge assigns per
    /// workspace (#250). Asked of the bridge each time; [`is_primary`](Self::is_primary)
    /// reads the answer afterwards.
    ///
    /// [`Error::BridgeDisconnected`] while there is no connection to the bridge.
    pub async fn peers(&self) -> Result<PeersResult> {
        let peers = self
            .bridge()?
            .peers()
            .await
            .map_err(|e| Error::Bridge(e.to_string()))?;
        *self.last_peers.lock().await = Some(peers.clone());
        Ok(peers)
    }

    /// This host's device id, asked of the bridge once.
    async fn device_id(&self) -> Result<GrainId> {
        if let Some(id) = self.device_id.get() {
            return Ok(*id);
        }
        let result = self
            .bridge()?
            .register(RegisterParams {
                app_name: self.ctx.app_name.to_owned(),
                exe_path: self.exe_path.clone(),
                managed_by: wire_managed_by(self.managed_by),
                workspaces: Vec::new(),
            })
            .await
            .map_err(|e| Error::Bridge(e.to_string()))?;
        let _ = self.device_id.set(result.device_id);
        Ok(result.device_id)
    }

    /// `<workspace cache dir>/sync/`.
    fn state_dir(&self, root: &Path) -> Result<PathBuf> {
        let workspace = Workspace::from_root(self.ctx, root)?;
        Ok(workspace.cache_dir().join("sync"))
    }
}

/// One dial of [`SyncRuntime::sync_now`].
type Dial<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;

/// Run `dials` concurrently on the calling task, until every one has finished.
async fn join_all(mut dials: Vec<Dial<'_>>) {
    std::future::poll_fn(|cx| {
        dials.retain_mut(|dial| dial.as_mut().poll(cx).is_pending());
        if dials.is_empty() {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    })
    .await
}

/// The bridge API's copy of how this server was started, for [`RegisterParams`].
fn wire_managed_by(managed_by: ManagedBy) -> sapphire_bridge_api::ManagedBy {
    match managed_by {
        ManagedBy::Service => sapphire_bridge_api::ManagedBy::Service,
        ManagedBy::Spawned => sapphire_bridge_api::ManagedBy::Spawned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::testing::StubBridge;
    use crate::test_support;
    use sapphire_workspace::AppKind;
    use std::ffi::OsString;

    static CTX: AppContext = AppContext::new("sapphire-synctest");

    /// The env vars `CTX.init` reads.
    const DIR_VARS: [&str; 3] = [
        "SAPPHIRE_SYNCTEST_CACHE_DIR",
        "SAPPHIRE_SYNCTEST_DATA_DIR",
        "SAPPHIRE_SYNCTEST_CONFIG_DIR",
    ];

    /// Points the context's directories at the test's scratch tree, and restores the
    /// previous values when dropped — including while unwinding from a panic.
    struct EnvGuard {
        previous: [Option<OsString>; 3],
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: `self._lock` still serialises the environment; it is dropped only
            // after this method returns.
            for (name, previous) in DIR_VARS.iter().zip(self.previous.iter_mut()) {
                match previous.take() {
                    Some(value) => unsafe { std::env::set_var(name, value) },
                    None => test_support::remove(name),
                }
            }
        }
    }

    /// Everything one test needs, held for the test's whole body.
    ///
    /// The context is a `static` shared by this binary, so its directories are resolved from
    /// the environment by whichever test initialises it first. The environment lock is
    /// therefore held for the whole test, as in the crate's other test modules, and `_tmp`
    /// is declared before `_env` so the scratch tree is gone only once the environment no
    /// longer points at it.
    struct Fixture {
        _tmp: tempfile::TempDir,
        _env: EnvGuard,
        root: PathBuf,
        stub: StubBridge,
        runtime: Arc<SyncRuntime>,
    }

    impl Fixture {
        /// A second workspace root beside the first, marker directory and all.
        fn second_root(&self) -> PathBuf {
            let root = self._tmp.path().join("ws2");
            std::fs::create_dir_all(root.join(".sapphire-synctest")).unwrap();
            root.canonicalize().unwrap()
        }
    }

    /// How long the fixture's runtime waits on a stream to a peer.
    const FIXTURE_OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

    /// `n` peers this host dials (a greater device id), as `bridge.peers` lists them.
    fn dialled_peers(me: GrainId, n: usize) -> Vec<sapphire_bridge_api::PeerInfo> {
        std::iter::repeat_with(GrainId::random)
            .filter(|id| *id > me)
            .take(n)
            .enumerate()
            .map(|(i, device_id)| sapphire_bridge_api::PeerInfo {
                device_id,
                name: format!("offline-{i}"),
                node_id: format!("node-{i}"),
                connected: false,
                priority: 1,
                availability: None,
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enabling_answers_at_once_with_a_peer_that_never_answers() {
        // #228: the first dial waited on a peer that was offline, past the client's limit.
        let f = fixture().await;
        let _hung = f.stub.hang_streams().await;
        f.stub.set_peers(dialled_peers(f.stub.device_id, 1));

        let started = std::time::Instant::now();
        f.runtime.enable(&f.root).await.unwrap();
        assert!(
            started.elapsed() < FIXTURE_OPEN_TIMEOUT,
            "enable waited {:?} on the dial",
            started.elapsed()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn peers_that_never_answer_are_dialled_together_and_given_up_on() {
        let f = fixture().await;
        let _hung = f.stub.hang_streams().await;
        f.runtime.enable(&f.root).await.unwrap();
        // The first dial, with nobody to dial yet, must be over before the peers appear:
        // one still running would claim them, and `sync_now` would skip them all.
        f.runtime.settle_dials().await;
        f.stub.set_peers(dialled_peers(f.stub.device_id, 3));

        let started = std::time::Instant::now();
        f.runtime.sync_now(&f.root).await.unwrap();
        let took = started.elapsed();
        assert!(took >= FIXTURE_OPEN_TIMEOUT, "gave up early: {took:?}");
        assert!(
            took < FIXTURE_OPEN_TIMEOUT * 2,
            "three dials one after another: {took:?}"
        );
    }

    async fn fixture() -> Fixture {
        let (tmp, env, root) = scratch();
        let (stub, client) = StubBridge::start_in(tmp.path().join("run")).await;
        let runtime = Arc::new(
            SyncRuntime::new(&CTX, client, "/bin/true".into(), ManagedBy::Service)
                .with_open_timeout(FIXTURE_OPEN_TIMEOUT),
        );
        Fixture {
            _tmp: tmp,
            _env: env,
            root,
            stub,
            runtime,
        }
    }

    /// The bridge connections a [`connecting`] fixture's connector hands out, in order;
    /// empty is "no bridge is running".
    type Bridges = Arc<std::sync::Mutex<std::collections::VecDeque<BridgeClient>>>;

    /// A fixture whose runtime reaches the bridge through a connector, and has not yet.
    /// `f.stub`'s client is not handed out: push it to the returned queue for that.
    async fn connecting() -> (Fixture, Bridges, BridgeClient) {
        let (tmp, env, root) = scratch();
        let (stub, client) = StubBridge::start_client_in(tmp.path().join("run")).await;
        let bridges: Bridges = Arc::default();
        let connector = BridgeConnector::new({
            let bridges = Arc::clone(&bridges);
            move || {
                let next = bridges.lock().unwrap().pop_front();
                async move { Ok(next) }
            }
        });
        let runtime = Arc::new(
            SyncRuntime::connecting(&CTX, connector, "/bin/true".into(), ManagedBy::Service)
                .with_open_timeout(FIXTURE_OPEN_TIMEOUT),
        );
        let f = Fixture {
            _tmp: tmp,
            _env: env,
            root,
            stub,
            runtime,
        };
        (f, bridges, client)
    }

    /// Wait up to ten seconds for `done`.
    async fn eventually(what: &str, mut done: impl AsyncFnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done().await {
            assert!(tokio::time::Instant::now() < deadline, "never: {what}");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_runtime_started_without_a_bridge_syncs_once_one_starts() {
        // #244: a server that started before its bridge never synced until restarted.
        let (f, bridges, client) = connecting().await;
        f.runtime.connect().await.unwrap();
        assert!(!f.runtime.is_connected());

        let err = f.runtime.enable(&f.root).await.unwrap_err();
        assert!(matches!(err, Error::BridgeDisconnected), "{err:?}");
        let status = f.runtime.status(&f.root).await;
        assert!(status.enabled, "enabled, waiting for the bridge");
        assert!(!status.bridge_available);
        let id = status.workspace_id.expect("its id");

        let keeping = tokio::spawn(Arc::clone(&f.runtime).keep_connected());
        bridges.lock().unwrap().push_back(client);
        eventually("the waiting workspace is registered", async || {
            f.stub.last_workspaces() == vec![id]
        })
        .await;
        assert_eq!(f.runtime.roots().await, vec![f.root.clone()]);
        assert!(f.runtime.status(&f.root).await.bridge_available);
        keeping.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_runtime_that_loses_its_bridge_registers_with_the_next_one() {
        let (f, bridges, client) = connecting().await;
        bridges.lock().unwrap().push_back(client);
        f.runtime.connect().await.unwrap();
        assert!(f.runtime.is_connected());
        let id = f.runtime.enable(&f.root).await.unwrap();
        let keeping = tokio::spawn(Arc::clone(&f.runtime).keep_connected());

        f.stub.disconnect();
        eventually("the runtime notices", async || !f.runtime.is_connected()).await;
        let err = f.runtime.workspaces().await.unwrap_err();
        assert!(matches!(err, Error::BridgeDisconnected), "{err:?}");

        let (next, client) = StubBridge::start_client_in(f._tmp.path().join("run")).await;
        bridges.lock().unwrap().push_back(client);
        eventually("the synced workspace is registered again", async || {
            next.last_workspaces() == vec![id]
        })
        .await;
        assert!(f.runtime.is_connected());
        keeping.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn this_device_is_known_once_a_workspace_is_enabled_and_never_asked_for() {
        let f = fixture().await;
        assert_eq!(f.runtime.this_device(), None);
        assert!(
            f.stub.seen.lock().unwrap().registrations.is_empty(),
            "asking must not register"
        );

        f.runtime.enable(&f.root).await.unwrap();
        assert_eq!(f.runtime.this_device(), Some(f.stub.device_id));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn peers_answers_the_workgroup_from_the_bridge() {
        let f = fixture().await;
        let listed = dialled_peers(f.stub.device_id, 2);
        f.stub.set_peers(listed.clone());

        let peers = f.runtime.peers().await.unwrap();
        let ids: Vec<_> = peers.peers.iter().map(|p| p.device_id).collect();
        assert_eq!(ids, listed.iter().map(|p| p.device_id).collect::<Vec<_>>());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn peers_says_so_while_the_bridge_is_away() {
        let (f, _bridges, _client) = connecting().await;
        let err = f.runtime.peers().await.unwrap_err();
        assert!(matches!(err, Error::BridgeDisconnected), "{err:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disabling_while_the_bridge_is_away_forgets_the_waiting_workspace() {
        let (f, bridges, client) = connecting().await;
        f.runtime.enable(&f.root).await.unwrap_err();
        f.runtime.disable(&f.root).await.unwrap();
        assert!(!f.runtime.status(&f.root).await.enabled);

        bridges.lock().unwrap().push_back(client);
        f.runtime.connect().await.unwrap();
        assert!(f.runtime.roots().await.is_empty());
        assert!(f.stub.last_workspaces().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn announcements_are_answered_from_a_reconnected_bridge() {
        let (f, bridges, client) = connecting().await;
        bridges.lock().unwrap().push_back(client);
        f.runtime.connect().await.unwrap();
        let id = f.runtime.enable(&f.root).await.unwrap();
        let keeping = tokio::spawn(Arc::clone(&f.runtime).keep_connected());
        let answering = tokio::spawn(Arc::clone(&f.runtime).run());

        f.stub.disconnect();
        let (next, client) = StubBridge::start_client_in(f._tmp.path().join("run")).await;
        bridges.lock().unwrap().push_back(client);
        eventually("reconnected", async || next.last_workspaces() == vec![id]).await;

        // The announcement loop follows the new connection: an announcement on it is
        // claimed, which with no data endpoint fails — and the loop lives on.
        next.announce(id, "ticket").await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(!answering.is_finished(), "the announcement loop ended");
        answering.abort();
        keeping.abort();
    }

    /// The scratch tree, the environment pointed at it, and a workspace root in it.
    fn scratch() -> (tempfile::TempDir, EnvGuard, PathBuf) {
        let lock = test_support::lock();
        let tmp = tempfile::tempdir().unwrap();
        let previous = DIR_VARS.map(std::env::var_os);
        // SAFETY (via `test_support::set`): `lock` serialises every read and write of the
        // process environment in this test binary, and it is held until `drop` has restored
        // the old values.
        for (name, dir) in DIR_VARS
            .iter()
            .zip(["cache", "data", "config"].map(|cat| tmp.path().join(cat)))
        {
            test_support::set(name, &dir);
        }
        let env = EnvGuard {
            previous,
            _lock: lock,
        };
        CTX.init(AppKind::Server);

        let root = tmp.path().join("ws");
        std::fs::create_dir_all(root.join(".sapphire-synctest")).unwrap();
        let root = root.canonicalize().unwrap();
        (tmp, env, root)
    }

    /// An application with one `Live` key in the workspace: `sapphire-synctest.note`.
    struct NoteApp;

    impl crate::settings::AppSettings for NoteApp {
        fn defs(&self) -> Vec<sapphire_framework_settings::SettingDef> {
            vec![sapphire_framework_settings::SettingDef {
                key: "sapphire-synctest.note".into(),
                kind: sapphire_framework_settings::Kind::String { non_empty: false },
                scopes: vec![sapphire_framework_settings::Scope::Workspace],
                overridable: false,
                apply: sapphire_framework_settings::Apply::Live,
                default: None,
                section: "Test".into(),
                doc: "test".into(),
                process: sapphire_framework_settings::Process::App,
            }]
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_workspace_config_changed_under_the_watcher_reaches_the_settings() {
        let f = fixture().await;
        // A context of its own, so its config directory is this test's: `CTX`'s is
        // whichever test resolved it first.
        let ctx: &'static AppContext = Box::leak(Box::new(AppContext::new("sapphire-synctest")));
        ctx.set_config_dir(f._tmp.path().join("settings"));
        let selection = crate::selection::SelectionFile::at(
            f._tmp.path().join("settings").join("workspace.toml"),
        );
        selection
            .save(&crate::selection::Selection {
                root: f.root.clone(),
                sync: true,
            })
            .unwrap();
        let settings =
            crate::settings::Settings::load_with(ctx, Arc::new(NoteApp), selection).unwrap();
        let changed = settings.changed();
        f.runtime.set_settings(Some(settings.clone()));
        f.runtime.enable(&f.root).await.unwrap();
        let watching = tokio::spawn(Arc::clone(&f.runtime).watch_changes());
        // Until the watcher is up, a write may go unseen: keep writing until it is noticed.
        let config = f.root.join(".sapphire-synctest").join("config.toml");
        eventually("the settings hear of the new value", async || {
            std::fs::write(&config, "[sapphire-synctest]\nnote = \"synced\"\n").unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            changed.has_changed().unwrap()
        })
        .await;
        assert_eq!(
            settings
                .get_as::<String>("sapphire-synctest.note")
                .as_deref(),
            Some("synced")
        );
        watching.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enabling_registers_the_workspace_with_the_bridge() {
        let f = fixture().await;

        let id = f.runtime.enable(&f.root).await.unwrap();
        assert_eq!(f.stub.last_workspaces(), vec![id]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enabling_twice_is_idempotent_and_keeps_the_same_id() {
        let f = fixture().await;

        let first = f.runtime.enable(&f.root).await.unwrap();
        let second = f.runtime.enable(&f.root).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(
            f.stub.last_workspaces(),
            vec![first],
            "still exactly one workspace"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_registration_carries_the_complete_current_list() {
        let f = fixture().await;
        let second_root = f.second_root();

        let a = f.runtime.enable(&f.root).await.unwrap();
        let b = f.runtime.enable(&second_root).await.unwrap();

        let mut seen = f.stub.last_workspaces();
        seen.sort();
        let mut want = vec![a, b];
        want.sort();
        assert_eq!(seen, want, "registration is the whole set, not a delta");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_later_registration_clears_this_apps_routes() {
        // The device id is asked of the bridge once, on the first `enable`, with an empty
        // workspace list — the one moment that is safe, because no route exists yet. Every
        // later registration must name the complete set: one with an empty list would
        // briefly clear this application's routes, and a peer asking for a workspace this
        // server owns would be told nobody has it.
        let f = fixture().await;
        let second_root = f.second_root();

        f.runtime.enable(&f.root).await.unwrap();
        f.runtime.enable(&second_root).await.unwrap();
        f.runtime.sync_now(&f.root).await.unwrap();
        f.runtime.disable(&f.root).await.unwrap();

        let seen = f.stub.seen.lock().expect("stub");
        let first_full = seen
            .registrations
            .iter()
            .position(|r| !r.workspaces.is_empty())
            .expect("enabling registers the workspace");
        let emptied_afterwards = seen.registrations[first_full + 1..]
            .iter()
            .filter(|r| r.workspaces.is_empty())
            .count();
        assert_eq!(
            emptied_afterwards, 0,
            "a registration after the first workspace must still name the whole set"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disabling_unregisters_and_leaves_the_files_alone() {
        let f = fixture().await;
        std::fs::write(f.root.join("keep.md"), "content").unwrap();

        let id = f.runtime.enable(&f.root).await.unwrap();
        f.runtime.disable(&f.root).await.unwrap();

        assert!(f.stub.last_workspaces().is_empty());
        assert!(
            f.root.join("keep.md").exists(),
            "disabling sync must not touch files"
        );
        assert!(
            crate::sync::id::sync_id_path("sapphire-synctest", &f.root).exists(),
            "the sync id stays, so re-enabling rejoins the same workspace"
        );
        let _ = id;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn re_enabling_after_disabling_reuses_the_identity() {
        let f = fixture().await;

        let first = f.runtime.enable(&f.root).await.unwrap();
        f.runtime.disable(&f.root).await.unwrap();
        let again = f.runtime.enable(&f.root).await.unwrap();

        assert_eq!(first, again);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn status_reports_disabled_for_a_workspace_that_was_never_enabled() {
        let f = fixture().await;

        let status = f.runtime.status(&f.root).await;
        assert!(!status.enabled);
        assert!(status.workspace_id.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disabling_something_that_was_never_enabled_is_not_an_error() {
        let f = fixture().await;
        f.runtime
            .disable(&f.root)
            .await
            .expect("disabling twice is fine");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_file_already_on_disk_is_recorded_on_enable() {
        // No watcher fires for a file written before the workspace was enabled; without a
        // scan on enable it was never recorded, and every session sent nothing.
        let f = fixture().await;
        std::fs::write(f.root.join("a.md"), "content").unwrap();
        f.runtime.enable(&f.root).await.unwrap();
        f.runtime.settle_dials().await;

        let replica = f.runtime.replica_of(&f.root).await.unwrap();
        let states = replica.lock().await.states().unwrap();
        assert!(
            states.iter().any(|(path, _)| path == "a.md"),
            "a file on disk before enable must be recorded: {states:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn re_enabling_while_the_replica_is_still_held_takes_it_back() {
        // The first scan after enabling runs in the background; a disable and a re-enable
        // can both land while it still holds the store, which redb opens only once.
        let f = fixture().await;
        f.runtime.enable(&f.root).await.unwrap();
        f.runtime.settle_dials().await;
        let held = f.runtime.replica_of(&f.root).await.unwrap();

        f.runtime.disable(&f.root).await.unwrap();
        f.runtime
            .enable(&f.root)
            .await
            .expect("a re-enable must not open the store a second time");

        let again = f.runtime.replica_of(&f.root).await.unwrap();
        assert!(Arc::ptr_eq(&held, &again), "the open replica is taken back");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn re_enabling_while_the_replica_is_still_closing_waits_for_it() {
        // The last holder let go just before the re-enable: the replica can no longer be
        // taken back, and its store closes a moment later, as it drops.
        let f = fixture().await;
        f.runtime.enable(&f.root).await.unwrap();
        f.runtime.settle_dials().await;
        let config = {
            let replica = f.runtime.replica_of(&f.root).await.unwrap();
            replica.lock().await.config().clone()
        };
        f.runtime.disable(&f.root).await.unwrap();

        // The store, open still, with nothing left to take back.
        let closing = Replica::open(config, Arc::new(SystemClock)).unwrap();
        let key = f.root.canonicalize().unwrap();
        f.runtime
            .stopped
            .lock()
            .unwrap()
            .insert(key, std::sync::Weak::new());
        let closed = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            drop(closing);
        });

        f.runtime
            .enable(&f.root)
            .await
            .expect("a re-enable must wait for a store that is closing");
        closed.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_workspace_whose_root_vanished_is_reported_as_paused_not_deleted() {
        let f = fixture().await;
        std::fs::write(f.root.join("a.md"), "content").unwrap();
        f.runtime.enable(&f.root).await.unwrap();
        f.runtime.scan(&f.root).await.unwrap();

        // The drive is unmounted: the marker directory goes with it.
        std::fs::remove_dir_all(f.root.join(".sapphire-synctest")).unwrap();
        let _ = f.runtime.scan(&f.root).await;

        let status = f.runtime.status(&f.root).await;
        assert!(
            status.paused.is_some(),
            "a missing root must pause, not replicate as a mass deletion"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_directory_that_is_not_a_workspace_cannot_be_enabled() {
        let f = fixture().await;
        let plain = f._tmp.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();

        assert!(f.runtime.enable(&plain).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_write_through_the_workspace_router_is_committed_to_the_replica() {
        // The brief's "main path": a write the app server itself made is scanned straight
        // after, not left to the watcher's debounce. Asserted against the replica's own
        // state, which is the only thing that can tell the two paths apart — reading the
        // file back would pass even if the scan never ran.
        let f = fixture().await;
        f.runtime.enable(&f.root).await.unwrap();

        let host = Arc::new(crate::WorkspaceHost::new(&CTX));
        host.select(&f.root, false).await.unwrap();
        let current = Arc::new(crate::Current::new(
            &CTX,
            host,
            Some(Arc::clone(&f.runtime)),
            crate::selection::SelectionFile::at(f._tmp.path().join("workspace.toml")),
        ));
        let router = Arc::new(crate::workspace_router(current));
        let (client_conn, server_conn) = sapphire_ipc::Connection::pair();
        tokio::spawn(async move {
            let info = sapphire_ipc::ServerInfo {
                version: "0.0.0".into(),
                api: sapphire_backend::protocol::API_VERSION,
                app_api: None,
                pid: std::process::id(),
                managed_by: ManagedBy::Spawned,
            };
            let _ = sapphire_ipc::serve(server_conn, router, "sapphire-synctest", info).await;
        });
        let info = sapphire_ipc::ClientInfo {
            kind: "test".into(),
            version: "0.0.0".into(),
            api: sapphire_backend::protocol::API_VERSION,
            app_api: None,
            pid: std::process::id(),
        };
        let (client, _) = sapphire_ipc::Client::handshake(client_conn, "sapphire-synctest", info)
            .await
            .unwrap();

        let _: sapphire_backend::protocol::Ack = client
            .call(
                sapphire_backend::protocol::WRITE_FILE,
                sapphire_backend::protocol::ContentParams {
                    path: PathBuf::from("through-the-server.md"),
                    content: "written by the server".into(),
                },
            )
            .await
            .unwrap();

        // `scan` runs before the reply is written, so by the time the call returns the
        // replica must already know the path.
        let synced = f.runtime.synced.lock().await;
        let entry = synced.get(&f.root).expect("enabled above");
        let replica = entry.replica.lock().await;
        let state = replica
            .state("through-the-server.md")
            .expect("a readable state");
        assert!(
            state.is_some(),
            "a write through the server must be scanned into the replica, not left to the watcher"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_announcement_for_an_unknown_workspace_is_ignored_not_fatal() {
        let f = fixture().await;
        f.runtime.enable(&f.root).await.unwrap();

        let driver = Arc::clone(&f.runtime);
        let handle = tokio::spawn(async move { driver.run().await });

        f.stub
            .announce(grain_id::GrainId::random(), "no-such-ticket")
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert!(
            !handle.is_finished(),
            "one bad announcement must not stop the runtime"
        );
        handle.abort();
    }
}
