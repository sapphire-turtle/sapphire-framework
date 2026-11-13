//! The served workspace's files in corundum's explorer, over the app server's `fs.*` and
//! `journal.*` (`docs/superpowers/specs/2026-11-13-workspace-explorer-design.md`).
//!
//! [`WorkspaceFiles`] holds the connection and the workspace the window shows; it hands
//! out [`WorkspaceExplorer`], the Files screen, and [`HistorySection`], the History page
//! of Settings. Both start again when the workspace changes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use corundum_framework_gui::fs_client::{
    DriverEvent, ExplorerDriver, FsClient, FsError, FsTransport, HistoryPage,
};
use corundum_framework_gui::preview::PreviewResponse;
use corundum_framework_gui::settings::SettingsSection;
use corundum_framework_gui::tr;
use corundum_framework_platform::pending::{self, Pending};
use sapphire_backend::protocol as proto;
use sapphire_ipc::{Client, ClientInfo, Endpoint};
use serde_json::Value;

use crate::client::AppIdentity;

/// A [`FsTransport`] over the app server's IPC: a connection of its own, made on the first
/// call and again after one fails, so the explorer never waits behind the framework
/// client's commands.
#[derive(Clone)]
pub struct IpcFsTransport {
    inner: Arc<Inner>,
}

struct Inner {
    runtime: tokio::runtime::Handle,
    endpoint: Endpoint,
    app: AppIdentity,
    repaint: Arc<dyn Fn() + Send + Sync>,
    /// The connection, once made; a failed call drops it.
    client: tokio::sync::Mutex<Option<Arc<Client>>>,
}

impl IpcFsTransport {
    /// Calls the app server at `endpoint`, on `runtime`; `repaint` runs when an answer
    /// comes.
    pub fn new(
        runtime: tokio::runtime::Handle,
        endpoint: Endpoint,
        app: AppIdentity,
        repaint: Arc<dyn Fn() + Send + Sync>,
    ) -> IpcFsTransport {
        IpcFsTransport {
            inner: Arc::new(Inner {
                runtime,
                endpoint,
                app,
                repaint,
                client: tokio::sync::Mutex::new(None),
            }),
        }
    }
}

impl Inner {
    /// The connection, made if there is none.
    async fn client(&self) -> Result<Arc<Client>, FsError> {
        let mut slot = self.client.lock().await;
        if let Some(client) = slot.as_ref().filter(|c| !c.is_closed()) {
            return Ok(Arc::clone(client));
        }
        let info = ClientInfo {
            kind: "gui".to_owned(),
            version: self.app.version.to_owned(),
            api: proto::API_VERSION,
            app_api: None,
            pid: std::process::id(),
        };
        let client = sapphire_ipc::connect_or_absent(&self.endpoint, self.app.app_name, info)
            .await
            .map_err(|e| FsError::Transport(e.to_string()))?
            .map(|(client, _)| Arc::new(client))
            .ok_or_else(|| {
                FsError::Transport(format!("the {} server is not running", self.app.app_name))
            })?;
        *slot = Some(Arc::clone(&client));
        Ok(client)
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, FsError> {
        let client = self.client().await?;
        match client.call::<_, Value>(method, params).await {
            Ok(value) => Ok(value),
            Err(sapphire_ipc::Error::Rpc(err)) => Err(FsError::Rpc {
                code: err.code,
                message: err.message,
            }),
            Err(sapphire_ipc::Error::Codec(err)) => Err(FsError::Decode(err.to_string())),
            Err(err) => {
                // The connection is gone or broken: the next call makes a new one.
                *self.client.lock().await = None;
                Err(FsError::Transport(err.to_string()))
            }
        }
    }
}

impl FsTransport for IpcFsTransport {
    fn call(&self, method: &str, params: Value) -> Pending<Result<Value, FsError>> {
        let repaint = Arc::clone(&self.inner.repaint);
        let (answer, sender) = pending::channel(move || repaint());
        let (inner, method) = (Arc::clone(&self.inner), method.to_owned());
        self.inner.runtime.spawn(async move {
            sender.send(Some(inner.call(&method, params).await));
        });
        answer.map(|answer| answer.unwrap_or_else(|| Err(FsError::default())))
    }
}

/// The workspace shown, and the client of its files: one origin per workspace.
#[derive(Default)]
struct Current {
    root: Option<PathBuf>,
    client: Option<FsClient>,
    /// Bumped on every change of workspace, so each view knows to start again.
    generation: u64,
}

/// The served workspace's files, for the window's views: give it the current workspace
/// each frame with [`set_root`](Self::set_root), and draw the views it hands out.
#[derive(Clone)]
pub struct WorkspaceFiles {
    transport: IpcFsTransport,
    current: Arc<Mutex<Current>>,
}

impl WorkspaceFiles {
    /// The files over `transport`, no workspace yet.
    pub fn new(transport: IpcFsTransport) -> WorkspaceFiles {
        WorkspaceFiles {
            transport,
            current: Arc::default(),
        }
    }

    /// The workspace the app server serves now, from the framework client's snapshot
    /// (`ServerState::current`). A change starts the views again, with a new origin: the
    /// server's history of the old workspace is gone too.
    pub fn set_root(&self, root: Option<&Path>) {
        let mut current = self.lock();
        if current.root.as_deref() == root {
            return;
        }
        current.root = root.map(Path::to_path_buf);
        current.client = root.map(|_| FsClient::new(self.transport.clone()));
        current.generation += 1;
    }

    /// The Files screen.
    pub fn explorer(&self) -> WorkspaceExplorer {
        WorkspaceExplorer {
            files: self.clone(),
            generation: None,
            driver: None,
        }
    }

    /// The History page, for [`Settings::with_app_sections`](crate::Settings::with_app_sections).
    pub fn history(&self) -> HistorySection {
        HistorySection {
            files: self.clone(),
            generation: None,
            page: None,
            drawn: None,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Current> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The client and its root when the generation differs from `seen`, after which `seen`
    /// is the current one; `None` when nothing changed. The outer `Some(None)` is "no
    /// workspace".
    #[allow(clippy::option_option)]
    fn changed(&self, seen: &mut Option<u64>) -> Option<Option<(PathBuf, FsClient)>> {
        let current = self.lock();
        if *seen == Some(current.generation) {
            return None;
        }
        *seen = Some(current.generation);
        Some(current.root.clone().zip(current.client.clone()))
    }
}

/// The name of `root`'s folder, for the explorer's trail.
fn root_label(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
}

/// The Files screen: the workspace's folders, moves, renames and trashing with undo
/// (Ctrl+Z), and the picked file's preview for the side pane.
pub struct WorkspaceExplorer {
    files: WorkspaceFiles,
    generation: Option<u64>,
    driver: Option<ExplorerDriver>,
}

impl WorkspaceExplorer {
    fn follow(&mut self) {
        if let Some(changed) = self.files.changed(&mut self.generation) {
            self.driver =
                changed.map(|(root, client)| ExplorerDriver::new(client, root_label(&root)));
        }
    }

    /// Draw the explorer filling `ui`, or a hint while there is no workspace. Returns
    /// whether a file was opened, for the side pane to show it.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        super::register(ui);
        self.follow();
        let Some(driver) = self.driver.as_mut() else {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() / 3.0);
                ui.weak(tr!(ui, "sapphire-files-no-workspace"));
            });
            return false;
        };
        driver
            .ui(ui)
            .into_iter()
            .any(|event| matches!(event, DriverEvent::Opened { .. }))
    }

    /// Draw the picked file's preview, for the side pane.
    pub fn detail_ui(&mut self, ui: &mut egui::Ui) -> Option<PreviewResponse> {
        self.follow();
        self.driver.as_mut().map(|driver| driver.detail_ui(ui))
    }
}

/// Settings › History: the workspace's recent changes, newest first, each with Undo.
pub struct HistorySection {
    files: WorkspaceFiles,
    generation: Option<u64>,
    page: Option<HistoryPage>,
    /// The pass this page was last drawn in: drawn again after a gap, it was opened anew,
    /// and loads the history again.
    drawn: Option<u64>,
}

impl SettingsSection for HistorySection {
    fn label(&self, ctx: &egui::Context) -> String {
        crate::i18n::register(ctx, crate::RESOURCES);
        tr!(ctx, "sapphire-history-title")
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        super::register(ui);
        if let Some(changed) = self.files.changed(&mut self.generation) {
            self.page = changed.map(|(_, client)| HistoryPage::new(client));
        }
        let pass = ui.ctx().cumulative_pass_nr();
        let reopened = self.drawn.is_some_and(|last| pass > last + 1);
        self.drawn = Some(pass);
        if reopened && let Some(page) = self.page.as_mut() {
            page.refresh();
        }
        ui.heading(tr!(ui, "sapphire-history-title"));
        match self.page.as_mut() {
            Some(page) => page.ui(ui),
            None => {
                ui.weak(tr!(ui, "sapphire-files-no-workspace"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sapphire_ipc::{Carrier, ManagedBy, RpcError, ServerInfo};

    const APP: AppIdentity = AppIdentity {
        app_name: "sapphire-explorertest",
        version: "0.0.0",
    };

    /// Serves `fs.version` and a method that always fails at `endpoint`, in this process.
    async fn serve_at(endpoint: Endpoint) {
        let listener = sapphire_ipc::bind_with(Carrier::InProcess, &endpoint)
            .await
            .unwrap();
        let router = Arc::new(
            sapphire_ipc::Router::new()
                .method("fs.version", |_| async {
                    Ok(serde_json::json!({ "api": 1 }))
                })
                .method("fs.stat", |_| async {
                    Err(RpcError {
                        code: -32102,
                        message: "not found".into(),
                        data: None,
                    })
                }),
        );
        tokio::spawn(async move {
            while let Ok(conn) = listener.accept().await {
                let router = Arc::clone(&router);
                let info = ServerInfo {
                    version: "0.0.0".into(),
                    api: proto::API_VERSION,
                    app_api: None,
                    pid: std::process::id(),
                    managed_by: ManagedBy::Spawned,
                };
                tokio::spawn(sapphire_ipc::serve(conn, router, APP.app_name, info));
            }
        });
    }

    #[test]
    fn calls_answer_results_and_errors_and_a_server_started_late_is_reached() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let endpoint = Endpoint::in_dir("explorertest", dir.path().to_owned());
        let transport = IpcFsTransport::new(
            runtime.handle().clone(),
            endpoint.clone(),
            APP,
            Arc::new(|| {}),
        );

        // No server yet: the call fails, and makes no connection to keep.
        let absent = transport.call("fs.version", Value::Null).wait();
        assert!(matches!(absent, Err(FsError::Transport(_))), "{absent:?}");

        runtime.block_on(serve_at(endpoint));
        let version = transport.call("fs.version", Value::Null).wait().unwrap();
        assert_eq!(version["api"], 1);
        let failed = transport.call("fs.stat", Value::Null).wait();
        assert_eq!(
            failed,
            Err(FsError::Rpc {
                code: -32102,
                message: "not found".into()
            })
        );
    }

    #[test]
    fn a_change_of_workspace_starts_the_views_again() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let transport = IpcFsTransport::new(
            runtime.handle().clone(),
            Endpoint::in_dir("unused", dir.path().to_owned()),
            APP,
            Arc::new(|| {}),
        );
        let files = WorkspaceFiles::new(transport);
        let mut seen = None;
        assert!(matches!(files.changed(&mut seen), Some(None)));
        assert!(files.changed(&mut seen).is_none());

        files.set_root(Some(Path::new("/w/a")));
        let first = files.changed(&mut seen).unwrap().unwrap();
        assert_eq!(first.0, Path::new("/w/a"));
        // The same root again changes nothing.
        files.set_root(Some(Path::new("/w/a")));
        assert!(files.changed(&mut seen).is_none());

        files.set_root(Some(Path::new("/w/b")));
        let second = files.changed(&mut seen).unwrap().unwrap();
        assert_ne!(first.1.origin(), second.1.origin());
        files.set_root(None);
        assert!(matches!(files.changed(&mut seen), Some(None)));
        assert_eq!(root_label(Path::new("/w/notes")), "notes");
    }
}
