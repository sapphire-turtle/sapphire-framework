//! The served workspace's API keys (#364): corundum's keys page over the workspace's files.

use std::path::{Path, PathBuf};

use corundum_framework_gui::keys::{KeysPage, LocalKeys};
use corundum_framework_gui::tr;

use super::ViewCtx;

/// Settings › API keys. The page is built over the store of the workspace the app server
/// serves, and built again when that changes. The keys are files in the workspace, so this
/// works on them directly, with or without the bridge.
#[derive(Default)]
pub struct ApiKeysView {
    /// The workspace the page is over, and the page.
    page: Option<(PathBuf, KeysPage)>,
    /// Why the store could not be opened.
    error: Option<String>,
}

impl ApiKeysView {
    /// Draw the page.
    pub fn ui(&mut self, ui: &mut egui::Ui, cx: &ViewCtx<'_>) {
        // An unreachable root (moved, unmounted) is not opened: opening would create it.
        let served = cx
            .snapshot
            .server
            .up()
            .and_then(|s| s.current.as_ref())
            .filter(|c| c.reachable)
            .map(|c| c.root.clone());
        let Some(root) = served else {
            self.page = None;
            self.error = None;
            ui.heading(tr!(ui, "sapphire-settings-api-keys"));
            ui.label(tr!(ui, "sapphire-api-keys-no-workspace"));
            return;
        };
        if self.root() != Some(root.as_path()) {
            self.page = None;
            match sapphire_framework_keys::open_store(cx.app_name, &root) {
                Ok(store) => {
                    self.error = None;
                    self.page = Some((root, KeysPage::new(Box::new(LocalKeys::new(store)))));
                }
                Err(err) => self.error = Some(err.to_string()),
            }
        }
        if let Some(error) = &self.error {
            ui.heading(tr!(ui, "sapphire-settings-api-keys"));
            ui.colored_label(ui.visuals().error_fg_color, error);
            return;
        }
        if let Some((_, page)) = &mut self.page {
            page.ui(ui);
        }
    }

    /// The workspace the page is over, if one is shown.
    pub fn root(&self) -> Option<&Path> {
        self.page.as_ref().map(|(root, _)| root.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Conn, ServerState, Snapshot};
    use crate::test_support::render;
    use sapphire_backend::protocol::{CurrentWorkspace, StatusReport, SyncStatusResult};

    const APP: &str = "guikeytest";

    fn serving(root: Option<&Path>, reachable: bool) -> Snapshot {
        Snapshot {
            fetched: true,
            server: Conn::Up(ServerState {
                info: StatusReport {
                    running: true,
                    version: Some("1".into()),
                    pid: Some(1),
                    managed_by: None,
                    app: vec![],
                },
                current: root.map(|root| CurrentWorkspace {
                    root: root.to_owned(),
                    reachable,
                    workspace_id: None,
                    sync: SyncStatusResult::not_synced(),
                }),
                update: None,
            }),
            ..Snapshot::default()
        }
    }

    fn draw(view: &mut ApiKeysView, snapshot: &Snapshot) {
        render([800.0, 600.0], |ui| {
            let cx = ViewCtx {
                snapshot,
                app_name: APP,
                busy: false,
            };
            view.ui(ui, &cx);
        });
    }

    #[test]
    fn the_page_is_over_the_served_workspace() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let mut view = ApiKeysView::default();
        draw(&mut view, &serving(Some(a.path()), true));
        assert_eq!(view.root(), Some(a.path()));
        assert!(sapphire_framework_keys::store_dir(APP, a.path()).is_dir());
        draw(&mut view, &serving(Some(b.path()), true));
        assert_eq!(view.root(), Some(b.path()), "rebuilt on a switch");
    }

    #[test]
    fn without_a_workspace_there_is_no_page() {
        let mut view = ApiKeysView::default();
        draw(&mut view, &serving(None, true));
        assert_eq!(view.root(), None);
        draw(&mut view, &Snapshot::default());
        assert_eq!(view.root(), None);
    }

    #[test]
    fn an_unreachable_workspace_is_not_recreated() {
        let tmp = tempfile::tempdir().unwrap();
        let gone = tmp.path().join("gone");
        let mut view = ApiKeysView::default();
        draw(&mut view, &serving(Some(&gone), false));
        assert_eq!(view.root(), None);
        assert!(!gone.exists());
    }
}
