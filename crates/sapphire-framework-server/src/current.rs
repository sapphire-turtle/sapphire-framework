//! The one workspace a server serves, and what changing it does (#215).
//!
//! [`WorkspaceHost`] holds the open workspace; this decides which one it is. Selecting
//! another closes the current one — its index, its replica, its live sessions — and opens
//! the new one; sync follows the workspace that has it (decision 3 of the spec). Every
//! `workspace.*` and `sync.*` method acts through here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use grain_id::GrainId;
use sapphire_backend::protocol as proto;
use sapphire_backend::{WorkspaceBackend, WorkspaceEntry, WorkspaceRegistry};
use sapphire_ipc::{Router, RpcError};
use sapphire_workspace::{AppContext, Workspace};
use tokio::sync::Mutex as AsyncMutex;

use crate::error::{Error, Result};
use crate::handlers::rpc_error;
use crate::host::WorkspaceHost;
use crate::selection::{Selection, SelectionFile, slug};
use crate::sync::{SYNC_ID_FILE, SyncRuntime};

/// How long a move or a delete waits for the workspace's replica store to close.
const RELEASE_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The server's current workspace.
pub struct Current {
    ctx: &'static AppContext,
    host: Arc<WorkspaceHost>,
    sync: Option<Arc<SyncRuntime>>,
    file: SelectionFile,
    /// One selection, and one sync switch, at a time.
    switching: AsyncMutex<()>,
}

impl std::fmt::Debug for Current {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Current")
            .field("root", &self.host.root())
            .finish()
    }
}

impl Current {
    /// The current workspace of `host`, synced by `sync` if there is one, remembered in
    /// `file`.
    pub fn new(
        ctx: &'static AppContext,
        host: Arc<WorkspaceHost>,
        sync: Option<Arc<SyncRuntime>>,
        file: SelectionFile,
    ) -> Current {
        Current {
            ctx,
            host,
            sync,
            file,
            switching: AsyncMutex::new(()),
        }
    }

    /// The host holding the open workspace.
    pub fn host(&self) -> &Arc<WorkspaceHost> {
        &self.host
    }

    /// The sync runtime, when this server syncs.
    pub fn sync(&self) -> Option<&Arc<SyncRuntime>> {
        self.sync.as_ref()
    }

    /// The current workspace's root.
    pub fn root(&self) -> Option<PathBuf> {
        self.host.root()
    }

    /// Make the workspace at `dir` current. Sync is on for it when it has a sync id.
    pub async fn select(&self, dir: &Path) -> Result<proto::CurrentWorkspace> {
        let root = resolve(dir)?;
        let sync = has_sync_id(self.ctx, &root);
        self.switch(&root, sync, false).await?;
        self.describe().await.ok_or(Error::NoWorkspace)
    }

    /// Create this app's workspace at `dir` — the marker and the marker registry's entry,
    /// idempotently — and make it current.
    pub async fn init(&self, dir: &Path) -> Result<proto::WorkspaceInitResult> {
        let result = init_workspace(self.ctx, dir)?;
        let sync = has_sync_id(self.ctx, &result.root);
        self.switch(&result.root, sync, false).await?;
        Ok(result)
    }

    /// Place the workgroup workspace `selector` at `dir`, make it current and sync it.
    pub async fn map(&self, selector: &str, dir: &Path) -> Result<GrainId> {
        let runtime = self.runtime()?;
        let root = runtime.place(selector, dir).await?;
        self.switch(&root, false, false).await?;
        self.enable_sync().await
    }

    /// Move the current workspace's folder to `dir`, which must not exist yet, and keep
    /// serving it from there, syncing as before.
    ///
    /// The replica's state moves with it, so sync carries on as the same replica rather
    /// than meeting its own files as strangers. The search index does not — it records
    /// absolute paths — and is rebuilt in the background.
    pub async fn relocate(&self, dir: &Path) -> Result<proto::CurrentWorkspace> {
        {
            let _one = self.switching.lock().await;
            let from = self.root().ok_or(Error::NoWorkspace)?;
            let to = destination(dir)?;
            if to.starts_with(&from) {
                return Err(Error::Config(format!(
                    "cannot move {} into itself",
                    from.display()
                )));
            }
            // Read before the move: the cache is keyed by the root's path.
            let from_cache = Workspace::from_root(self.ctx, &from)?.cache_dir();
            let synced = self.let_go(&from).await?;
            self.host.close();
            if let Err(err) = std::fs::rename(&from, &to) {
                // Back where it was, as it was.
                self.reopen(&from, synced).await;
                return Err(match err.kind() {
                    std::io::ErrorKind::CrossesDevices => Error::Config(format!(
                        "cannot move {} to another drive ({}); move it there yourself, then open it",
                        from.display(),
                        to.display()
                    )),
                    _ => Error::Io(std::io::Error::new(
                        err.kind(),
                        format!(
                            "could not move {} to {}: {err}",
                            from.display(),
                            to.display()
                        ),
                    )),
                });
            }

            let workspace = Workspace::from_root(self.ctx, &to)?;
            carry_cache(&from_cache, &workspace.cache_dir(), &to);
            if let Err(err) = rebase_registry(&workspace.config_path(), &from, &to) {
                tracing::warn!(root = %to.display(), "could not update the workspace registry: {err}");
            }
            self.reopen(&to, synced).await;
        }
        // Outside the switch lock: a reindex walks every file, and a switch must not wait
        // on it. One that lands first finds this workspace no longer current, and stops.
        if let Ok(backend) = self.host.backend().await {
            tokio::spawn(async move {
                if let Err(err) = backend.sync().await {
                    tracing::warn!("re-indexing the moved workspace: {err}");
                }
            });
        }
        self.describe().await.ok_or(Error::NoWorkspace)
    }

    /// Stop serving the current workspace; with `delete`, delete its folder and its cache
    /// too. Sync stops for this host only: the workgroup and the other devices keep it.
    pub async fn remove(&self, delete: bool) -> Result<()> {
        let _one = self.switching.lock().await;
        let root = self.root().ok_or(Error::NoWorkspace)?;
        // Read before anything goes: the cache is keyed by the root's path.
        let cache = Workspace::from_root(self.ctx, &root)
            .ok()
            .map(|w| w.cache_dir());
        // Stopped first, so the watcher never reports the files going as deletions to
        // replicate; and, before a delete, closed, so its store can go too.
        if delete {
            self.let_go(&root).await?;
        } else if let Some(runtime) = &self.sync
            && let Err(err) = runtime.disable(&root).await
        {
            tracing::warn!(root = %root.display(), "stopping sync before the removal: {err}");
        }
        self.host.close();
        self.file.clear()?;
        if delete {
            match std::fs::remove_dir_all(&root) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(Error::Io(std::io::Error::new(
                        e.kind(),
                        format!("could not delete {}: {e}", root.display()),
                    )));
                }
            }
            if let Some(cache) = cache
                && let Err(err) = std::fs::remove_dir_all(&cache)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(cache = %cache.display(), "could not delete the cache: {err}");
            }
        }
        Ok(())
    }

    /// Stop syncing `root` and close its replica store, before its folder moves or goes,
    /// returning whether it was synced. A store still in use is the error, and leaves the
    /// workspace syncing as before.
    async fn let_go(&self, root: &Path) -> Result<bool> {
        let Some(runtime) = &self.sync else {
            return Ok(false);
        };
        if !runtime.roots().await.iter().any(|r| r == root) {
            return Ok(false);
        }
        if let Err(err) = runtime.release(root, RELEASE_LIMIT).await {
            if let Err(again) = runtime.enable(root).await {
                tracing::warn!(root = %root.display(), "could not restart sync: {again}");
            }
            return Err(err);
        }
        Ok(true)
    }

    /// Open `root` again after [`relocate`](Self::relocate) closed it, and sync it again
    /// when it was. Failures leave it selected, unopened, the way a restore does.
    async fn reopen(&self, root: &Path, sync: bool) {
        if let Err(err) = self.host.select(root, true).await {
            tracing::warn!(root = %root.display(), "could not reopen the workspace: {err}");
        }
        if sync
            && let Some(runtime) = &self.sync
            && let Err(err) = runtime.enable(root).await
        {
            tracing::warn!(root = %root.display(), "could not restart sync: {err}");
        }
        self.remember(root, sync);
    }

    /// Start syncing the current workspace, and remember that.
    pub async fn enable_sync(&self) -> Result<GrainId> {
        let runtime = self.runtime()?;
        let _one = self.switching.lock().await;
        let root = self.root().ok_or(Error::NoWorkspace)?;
        let id = match runtime.enable(&root).await {
            Ok(id) => id,
            // The runtime enables it once the bridge is back, so remember it as synced: the
            // next start tries again too (#244).
            Err(err @ Error::BridgeDisconnected) => {
                self.remember(&root, true);
                return Err(err);
            }
            Err(err) => return Err(err),
        };
        self.remember(&root, true);
        Ok(id)
    }

    /// Stop syncing the current workspace, and remember that.
    pub async fn disable_sync(&self) -> Result<()> {
        let runtime = self.runtime()?;
        let _one = self.switching.lock().await;
        let root = self.root().ok_or(Error::NoWorkspace)?;
        runtime.disable(&root).await?;
        self.remember(&root, false);
        Ok(())
    }

    /// The current workspace's sync state; not synced when there is none.
    pub async fn status(&self) -> proto::SyncStatusResult {
        match &self.sync {
            // An empty root names nothing, which the runtime reports as not synced, with
            // the bridge's view all the same.
            Some(runtime) => runtime
                .status(&self.root().unwrap_or_default())
                .await
                .into(),
            None => proto::SyncStatusResult::not_synced(),
        }
    }

    /// The current workspace as `workspace.current` reports it.
    pub async fn describe(&self) -> Option<proto::CurrentWorkspace> {
        let root = self.root()?;
        let marker = root.join(format!(".{}", self.ctx.app_name));
        // Read, never minted: describing must not create a sync id nobody asked for.
        let workspace_id = std::fs::read_to_string(marker.join(SYNC_ID_FILE))
            .ok()
            .and_then(|s| s.trim().parse().ok());
        let sync = match &self.sync {
            Some(runtime) => runtime.status(&root).await.into(),
            None => proto::SyncStatusResult::not_synced(),
        };
        Some(proto::CurrentWorkspace {
            reachable: marker.is_dir(),
            root,
            workspace_id,
            sync,
        })
    }

    /// Come back to the workspace `workspace.toml` names. A root that is gone stays
    /// selected, unopened, so `workspace.current` can say so.
    ///
    /// The open happens now, so the first client finds the workspace; sync, which talks to
    /// the bridge and may dial peers, starts on the returned task.
    pub(crate) async fn restore(self: &Arc<Self>) -> Option<tokio::task::JoinHandle<()>> {
        let selection = match self.file.load(self.ctx.app_name) {
            Ok(Some(selection)) => selection,
            Ok(None) => return None,
            Err(err) => {
                tracing::warn!("could not read the selected workspace: {err}");
                return None;
            }
        };
        {
            let _one = self.switching.lock().await;
            if let Err(err) = self.host.select(&selection.root, true).await {
                tracing::warn!(root = %selection.root.display(), "could not restore the workspace: {err}");
                return None;
            }
        }
        if !selection.sync {
            return None;
        }
        let runtime = self.sync.clone()?;
        let current = Arc::clone(self);
        Some(tokio::spawn(async move {
            // A switch that landed first wins: sync starts only for a workspace still
            // current.
            let _one = current.switching.lock().await;
            if current.root().as_deref() != Some(&selection.root) {
                return;
            }
            if let Err(err) = runtime.enable(&selection.root).await {
                tracing::warn!(root = %selection.root.display(), "could not restore sync: {err}");
            }
        }))
    }

    /// Make `root` current; sync it when `sync`. The previous workspace stops syncing.
    ///
    /// The open comes first: a workspace that does not open leaves the previous one current,
    /// syncing as before. `restoring` keeps a root that is gone selected, unopened.
    async fn switch(&self, root: &Path, sync: bool, restoring: bool) -> Result<()> {
        let _one = self.switching.lock().await;
        let previous = self.root();
        self.host.select(root, restoring).await?;
        if let Some(runtime) = &self.sync {
            if let Some(previous) = previous.filter(|p| p != root)
                && let Err(err) = runtime.disable(&previous).await
            {
                tracing::warn!(root = %previous.display(), "stopping sync of the previous workspace: {err}");
            }
            if sync && let Err(err) = runtime.enable(root).await {
                // Selected all the same: the workspace works without sync, and the status
                // carries the failure.
                tracing::warn!(root = %root.display(), "could not start sync: {err}");
            }
        }
        self.remember(root, sync);
        Ok(())
    }

    fn remember(&self, root: &Path, sync: bool) {
        let selection = Selection {
            root: root.to_owned(),
            sync,
        };
        if let Err(err) = self.file.save(&selection) {
            tracing::warn!(root = %root.display(), "could not record the selected workspace: {err}");
        }
    }

    fn runtime(&self) -> Result<&Arc<SyncRuntime>> {
        self.sync
            .as_ref()
            .ok_or_else(|| Error::Config("this server does not sync".to_owned()))
    }
}

/// Whether `root` has synced before: it carries a sync id.
fn has_sync_id(ctx: &AppContext, root: &Path) -> bool {
    root.join(format!(".{}", ctx.app_name))
        .join(SYNC_ID_FILE)
        .is_file()
}

/// `dir` as a canonical root. A relative `dir` is resolved against the server's cwd.
fn resolve(dir: &Path) -> Result<PathBuf> {
    std::env::current_dir()
        .map_err(Error::Io)?
        .join(dir)
        .canonicalize()
        .map_err(Error::Io)
}

/// Where a move to `dir` puts the root: `dir` made absolute, through its parent's
/// canonical path, since `dir` itself must not exist yet.
fn destination(dir: &Path) -> Result<PathBuf> {
    let dir = std::env::current_dir().map_err(Error::Io)?.join(dir);
    if dir.exists() {
        return Err(Error::Config(format!(
            "{} already exists; choose a new folder name",
            dir.display()
        )));
    }
    let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) else {
        return Err(Error::Config(format!(
            "{} cannot be a workspace root",
            dir.display()
        )));
    };
    Ok(parent.canonicalize().map_err(Error::Io)?.join(name))
}

/// Carry a moved workspace's cache from `from` to `to`, its new key: the replica's state,
/// rebased onto `root`. The rest — the search index, which records absolute paths — is
/// dropped, to be rebuilt.
///
/// Best-effort: a replica that does not come along is a replica that starts over, and
/// syncing reports why.
fn carry_cache(from: &Path, to: &Path, root: &Path) {
    let sync_from = from.join("sync");
    if sync_from.is_dir() {
        let sync_to = to.join("sync");
        let moved =
            std::fs::create_dir_all(to).and_then(|()| std::fs::rename(&sync_from, &sync_to));
        match moved {
            Ok(()) => {
                let store = sync_to.join("sync.redb");
                if store.is_file()
                    && let Err(err) =
                        sapphire_sync::ReplicaStore::relocate(&store, &root.to_string_lossy())
                {
                    tracing::warn!(store = %store.display(), "could not rebase the replica: {err}");
                }
            }
            Err(err) => {
                tracing::warn!(cache = %from.display(), "could not carry the replica over: {err}");
            }
        }
    }
    if let Err(err) = std::fs::remove_dir_all(from)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(cache = %from.display(), "could not delete the old cache: {err}");
    }
}

/// Point the registry entries naming `from` at `to`, in the marker's `config.toml`.
fn rebase_registry(config_path: &Path, from: &Path, to: &Path) -> Result<()> {
    let mut registry = read_registry(config_path)?;
    let mut changed = false;
    for entry in registry.0.values_mut() {
        if entry.path.as_deref() == Some(from) {
            entry.path = Some(to.to_owned());
            changed = true;
        }
    }
    if changed {
        write_registry(config_path, &registry)?;
    }
    Ok(())
}

/// Add `workspace.init`, `workspace.select`, `workspace.current`, `workspace.move` and
/// `workspace.remove` to `router`.
pub(crate) fn current_methods(current: Arc<Current>, router: Router) -> Router {
    let init = Arc::clone(&current);
    let select = Arc::clone(&current);
    let relocate = Arc::clone(&current);
    let remove = Arc::clone(&current);
    router
        .method(proto::WORKSPACE_MOVE, move |req| {
            let current = Arc::clone(&relocate);
            async move {
                let p: proto::WorkspaceMoveParams = serde_json::from_value(req.params)
                    .map_err(|e| RpcError::invalid_params(format!("bad parameters: {e}")))?;
                let result = current.relocate(&p.dir).await.map_err(|e| rpc_error(&e))?;
                serde_json::to_value(result).map_err(|e| RpcError::internal(e.to_string()))
            }
        })
        .method(proto::WORKSPACE_REMOVE, move |req| {
            let current = Arc::clone(&remove);
            async move {
                let p: proto::WorkspaceRemoveParams = serde_json::from_value(req.params)
                    .map_err(|e| RpcError::invalid_params(format!("bad parameters: {e}")))?;
                current.remove(p.delete).await.map_err(|e| rpc_error(&e))?;
                serde_json::to_value(proto::Ack {}).map_err(|e| RpcError::internal(e.to_string()))
            }
        })
        .method(proto::WORKSPACE_INIT, move |req| {
            let current = Arc::clone(&init);
            async move {
                let p: proto::WorkspaceInitParams = serde_json::from_value(req.params)
                    .map_err(|e| RpcError::invalid_params(format!("bad parameters: {e}")))?;
                let result = current.init(&p.dir).await.map_err(|e| rpc_error(&e))?;
                serde_json::to_value(result).map_err(|e| RpcError::internal(e.to_string()))
            }
        })
        .method(proto::WORKSPACE_SELECT, move |req| {
            let current = Arc::clone(&select);
            async move {
                let p: proto::WorkspaceSelectParams = serde_json::from_value(req.params)
                    .map_err(|e| RpcError::invalid_params(format!("bad parameters: {e}")))?;
                let result = current.select(&p.dir).await.map_err(|e| rpc_error(&e))?;
                serde_json::to_value(result).map_err(|e| RpcError::internal(e.to_string()))
            }
        })
        .method(proto::WORKSPACE_CURRENT, move |_| {
            let current = Arc::clone(&current);
            async move {
                let workspace = current.describe().await;
                serde_json::to_value(proto::WorkspaceCurrentResult { workspace })
                    .map_err(|e| RpcError::internal(e.to_string()))
            }
        })
}

/// Create the workspace home at `dir`: the marker directory and the marker registry's
/// entry, idempotently. The sync id is `sync.enable` / `sync.map`'s to mint, not this one's.
///
/// A relative `dir` is resolved against the server's cwd. The registry entry's id is the
/// root's directory name, slugified; `created` is `false` when the marker was already there,
/// and an entry already there is kept as it is.
fn init_workspace(ctx: &'static AppContext, dir: &Path) -> Result<proto::WorkspaceInitResult> {
    let root = resolve(dir)?;
    let marker = root.join(format!(".{}", ctx.app_name));
    let created = !marker.is_dir();
    if created {
        std::fs::create_dir(&marker).map_err(Error::Io)?;
    }

    // Reads the marker's `config.toml`, keyed the way the apps' CLIs key their
    // `--workspace` selectors. The registry lives in the marker, so it travels with the
    // workspace when it syncs.
    let workspace = Workspace::from_root(ctx, &root)?;
    let id = slug(&root);
    let config_path = workspace.config_path();
    let registry = read_registry(&config_path)?;
    if registry.get(&id).is_none() {
        let mut registry = registry;
        registry.insert(id.clone(), WorkspaceEntry::local(&root));
        write_registry(&config_path, &registry)?;
    }

    Ok(proto::WorkspaceInitResult {
        root,
        workspace_id: id,
        created,
    })
}

/// The registry as the marker's `config.toml` holds it, or an empty one.
///
/// A file another application wrote without a `[workspace]` table is an empty registry,
/// not an error: the marker's config is the app's own file, and a workspace created
/// before this table existed is a workspace with no entries.
fn read_registry(path: &Path) -> Result<WorkspaceRegistry> {
    #[derive(serde::Deserialize, Default)]
    struct Config {
        #[serde(default)]
        workspace: WorkspaceRegistry,
    }
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let config: Config = toml::from_str(&text).map_err(|e| Error::SyncId(e.to_string()))?;
            Ok(config.workspace)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(WorkspaceRegistry::default()),
        Err(e) => Err(Error::Io(e)),
    }
}

/// Write the registry back into the marker's `config.toml`, keeping the rest of the file.
///
/// The marker's config is the app's own document, and a rewrite that dropped the rest of
/// it would eat an application's settings.
fn write_registry(path: &Path, registry: &WorkspaceRegistry) -> Result<()> {
    #[derive(serde::Deserialize, serde::Serialize, Default)]
    struct Config {
        #[serde(default, skip_serializing_if = "WorkspaceRegistry::is_empty")]
        workspace: WorkspaceRegistry,
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(Error::Io(e)),
    };
    let mut config: Config = toml::from_str(&text).unwrap_or_default();
    config.workspace = registry.clone();
    let out = toml::to_string_pretty(&config).map_err(|e| Error::SyncId(e.to_string()))?;
    std::fs::write(path, out).map_err(Error::Io)
}
