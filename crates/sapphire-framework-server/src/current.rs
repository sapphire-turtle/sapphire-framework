//! The one workspace a server serves, and what changing it does (#215).
//!
//! [`WorkspaceHost`] holds the open workspace; this decides which one it is. Selecting
//! another closes the current one — its index, its replica, its live sessions — and opens
//! the new one; sync follows the workspace that has it (decision 3 of the spec). Every
//! `workspace.*` and `sync.*` method acts through here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use grain_id::GrainId;
use sapphire_backend::WorkspaceBackend;
use sapphire_backend::protocol as proto;
use sapphire_ipc::{Router, RpcError};
use sapphire_workspace::{AppContext, Workspace};
use tokio::sync::Mutex as AsyncMutex;

use crate::error::{Error, Result};
use crate::handlers::rpc_error;
use crate::host::WorkspaceHost;
use crate::selection::{Selection, SelectionFile, slug};
use crate::settings::Settings;
use crate::sync::{SYNC_ID_FILE, SyncRuntime};

/// How long a move or a delete waits for the workspace's replica store to close.
const RELEASE_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The server's current workspace.
pub struct Current {
    ctx: &'static AppContext,
    host: Arc<WorkspaceHost>,
    sync: Option<Arc<SyncRuntime>>,
    file: SelectionFile,
    /// The settings whose workspace scope is the selected workspace's file, read again
    /// whenever the selection changes.
    settings: Option<Settings>,
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
            settings: None,
            switching: AsyncMutex::new(()),
        }
    }

    /// Read `settings` again whenever the selected workspace changes, so its workspace
    /// keys follow the selection.
    pub(crate) fn with_settings(mut self, settings: Settings) -> Current {
        self.settings = Some(settings);
        self
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

    /// Create this app's workspace at `dir` — its marker, idempotently — and make it
    /// current.
    pub async fn init(&self, dir: &Path) -> Result<proto::WorkspaceInitResult> {
        let result = init_workspace(self.ctx, dir)?;
        let sync = has_sync_id(self.ctx, &result.root);
        self.switch(&result.root, sync, false).await?;
        Ok(result)
    }

    /// Place the workgroup workspace `selector` at `dir`, make it current and sync it.
    ///
    /// `dir` needs no marker: placing makes one. One that an older `workspace.init` made
    /// first may hold a `config.toml` with nothing but the `[workspace]` registry, which
    /// would meet the workspace's own `config.toml` as a conflict on the first sync and win
    /// (#253), so a directory mapped for the first time loses it.
    pub async fn map(&self, selector: &str, dir: &Path) -> Result<GrainId> {
        let runtime = self.runtime()?;
        let first = !has_sync_id(self.ctx, &resolve(dir)?);
        let root = runtime.place(selector, dir).await?;
        if first {
            let config = Workspace::from_root(self.ctx, &root)?.config_path();
            if let Err(err) = drop_bare_registry(&config) {
                tracing::warn!(config = %config.display(), "could not clear the registry-only config: {err}");
            }
        }
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
            self.reopen(&to, synced).await;
        }
        // Outside the switch lock: a reindex walks every file, and a switch must not wait
        // on it. One that lands first finds this workspace no longer current, and stops.
        if let Ok(backend) = self.host.backend().await {
            tokio::spawn(async move {
                let started = std::time::Instant::now();
                let result = backend.sync().await;
                crate::instrument::reindexed(started, result.is_ok());
                if let Err(err) = result {
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
        self.refresh_settings();
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
            self.refresh_settings();
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
        self.refresh_settings();
    }

    /// Read the settings again, the workspace scope now the selected workspace's file.
    fn refresh_settings(&self) {
        if let Some(settings) = &self.settings
            && settings.refresh().is_err()
        {
            // The error may quote the file; the selection itself succeeded.
            tracing::warn!("could not read the settings of the selected workspace");
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

/// Create the workspace home at `dir`: the marker directory, idempotently. The sync id is
/// `sync.enable` / `sync.map`'s to mint, not this one's.
///
/// A relative `dir` is resolved against the server's cwd. `created` is `false` when the
/// marker was already there.
///
/// No `[workspace]` registry is written into the marker's `config.toml` any more (#217):
/// it keyed the apps' `--workspace` selectors, which went with the server serving one
/// workspace, and a `config.toml` holding only it lost the application's settings on the
/// first sync (#253).
fn init_workspace(ctx: &'static AppContext, dir: &Path) -> Result<proto::WorkspaceInitResult> {
    let root = resolve(dir)?;
    let marker = root.join(format!(".{}", ctx.app_name));
    let created = !marker.is_dir();
    if created {
        std::fs::create_dir(&marker).map_err(Error::Io)?;
    }

    Ok(proto::WorkspaceInitResult {
        workspace_id: slug(&root),
        root,
        created,
    })
}

/// Delete the marker's `config.toml` when it holds the `[workspace]` registry an older
/// `workspace.init` wrote, and nothing else.
fn drop_bare_registry(path: &Path) -> Result<()> {
    let config = read_config(path)?;
    if config.len() == 1 && config.contains_key("workspace") {
        std::fs::remove_file(path).map_err(Error::Io)?;
    }
    Ok(())
}

/// The marker's `config.toml` as a table; an empty one when there is no file.
fn read_config(path: &Path) -> Result<toml::Table> {
    match std::fs::read_to_string(path) {
        Ok(text) => text.parse().map_err(|e: toml::de::Error| {
            Error::Config(format!(
                "{}: {}",
                path.display(),
                sapphire_framework_settings::describe_toml_error(&text, e.span(), e.message())
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(Error::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTRY_ONLY: &str = "[workspace.ledger]\npath = \"/home/me/ledger\"\n";

    #[test]
    fn only_a_registry_only_config_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");

        std::fs::write(&path, REGISTRY_ONLY).unwrap();
        drop_bare_registry(&path).unwrap();
        assert!(!path.exists());
        // Nothing there is nothing to drop.
        drop_bare_registry(&path).unwrap();

        std::fs::write(&path, format!("schema_version = 2\n\n{REGISTRY_ONLY}")).unwrap();
        drop_bare_registry(&path).unwrap();
        assert!(path.exists());
    }

    /// An application with one `Live` key in the workspace: `<app>.note`.
    struct NoteApp(&'static str);

    impl crate::settings::AppSettings for NoteApp {
        fn defs(&self) -> Vec<sapphire_framework_settings::SettingDef> {
            vec![sapphire_framework_settings::SettingDef {
                key: format!("{}.note", self.0),
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
    async fn workspace_settings_follow_the_selection() {
        const APP: &str = "sapphire-curtest";
        let dir = tempfile::tempdir().unwrap();
        let ctx: &'static AppContext = Box::leak(Box::new(AppContext::new(APP)));
        ctx.set_config_dir(dir.path().join("config"));
        ctx.set_cache_dir(dir.path().join("cache"));
        ctx.set_data_dir(dir.path().join("data"));
        let file = SelectionFile::at(dir.path().join("config").join("workspace.toml"));
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        for (root, value) in [(&a, "from a"), (&b, "from b")] {
            std::fs::create_dir_all(root).unwrap();
            init_workspace(ctx, root).unwrap();
            std::fs::write(
                root.join(format!(".{APP}")).join("config.toml"),
                format!("[{APP}]\nnote = \"{value}\"\n"),
            )
            .unwrap();
        }
        let settings = crate::settings::Settings::load_with(
            ctx,
            std::sync::Arc::new(NoteApp(APP)),
            file.clone(),
        )
        .unwrap();
        let key = format!("{APP}.note");
        let current = Current::new(ctx, Arc::new(WorkspaceHost::new(ctx)), None, file)
            .with_settings(settings.clone());
        let mut changed = settings.changed();

        current.select(&a).await.unwrap();
        assert!(changed.has_changed().unwrap());
        changed.mark_unchanged();
        assert_eq!(settings.get_as::<String>(&key).as_deref(), Some("from a"));

        current.select(&b).await.unwrap();
        assert!(changed.has_changed().unwrap());
        changed.mark_unchanged();
        assert_eq!(settings.get_as::<String>(&key).as_deref(), Some("from b"));

        // No workspace, no workspace value.
        current.remove(false).await.unwrap();
        assert!(changed.has_changed().unwrap());
        assert_eq!(settings.get(&key), None);
    }

    #[test]
    fn an_unparsable_marker_config_is_never_quoted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[app]\ntoken = \"hunter2\" x\n").unwrap();
        let err = read_config(&path).unwrap_err().to_string();
        assert!(err.contains(&path.display().to_string()), "{err}");
        assert!(err.contains("line 2"), "{err}");
        assert!(!err.contains("hunter2"), "{err}");
    }

    #[test]
    fn init_writes_no_config() {
        let dir = tempfile::tempdir().unwrap();
        let ctx: &'static AppContext = Box::leak(Box::new(AppContext::new("sapphire-initapp")));
        let result = init_workspace(ctx, dir.path()).unwrap();
        assert!(result.created);
        let workspace = Workspace::from_root(ctx, dir.path()).unwrap();
        assert!(workspace.marker_dir().is_dir());
        assert!(!workspace.config_path().exists());
    }
}
