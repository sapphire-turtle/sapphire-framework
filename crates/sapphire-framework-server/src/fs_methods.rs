//! The `fs.*` and `journal.*` methods: the current workspace's files for an explorer, with
//! moves, renames, new folders and trashing as undoable batches
//! (`docs/superpowers/specs/2026-11-13-workspace-explorer-design.md`).
//!
//! The engine is corundum-framework-fs's [`FsService`]; this module only keeps one for the
//! current workspace, says who calls, and tells sync about a change.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use corundum_framework_fs::api::{
    ApplyParams, FS_APPLY, FS_LIST, FS_READ, FS_STAT, FS_VERSION, JOURNAL_HISTORY, JOURNAL_REDO,
    JOURNAL_STATUS, JOURNAL_UNDO, JOURNAL_UNDO_BATCH,
};
use corundum_framework_fs::journal::Actor;
use corundum_framework_fs::{FsService, Workspace, WorkspaceApp};
use sapphire_ipc::{RequestCtx, Router, RpcError};
use serde_json::Value;

use crate::current::Current;
use crate::error::Error;
use crate::handlers::{rpc_error, scanned};

/// The folder at the workspace's root that `fs.trash` keeps the trash in.
///
/// Not `.<app>`: the sync filter syncs `.<app>/` and no other dot-name, so the trash stays
/// on this device. To the other devices a trashed entry is deleted, and an undo makes it
/// again.
pub const OWN_FOLDER: &str = ".sapphire-local";

/// The principal of every IPC client: they are all the person signed in on this device.
pub const LOCAL_PRINCIPAL: &str = "local";

/// The `ClientInfo::kind` of the MCP server's connection (`sapphire_framework_mcp::CLIENT_KIND`),
/// whose changes an agent makes.
const MCP_CLIENT_KIND: &str = "mcp";

/// Every method this module answers.
pub const METHODS: [&str; 10] = [
    FS_VERSION,
    FS_LIST,
    FS_STAT,
    FS_READ,
    FS_APPLY,
    JOURNAL_UNDO,
    JOURNAL_REDO,
    JOURNAL_UNDO_BATCH,
    JOURNAL_HISTORY,
    JOURNAL_STATUS,
];

/// sapphire's side of the engine: no managed files, every dot-name hidden (corundum's
/// defaults), and the trash in [`OWN_FOLDER`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SapphireApp;

impl WorkspaceApp for SapphireApp {
    fn own_folder(&self) -> &str {
        OWN_FOLDER
    }
}

/// Who a connection is, to the journal.
pub fn actor_for(client_kind: &str) -> Actor {
    Actor {
        principal: LOCAL_PRINCIPAL.to_owned(),
        agent: client_kind == MCP_CLIENT_KIND,
    }
}

/// The service of one workspace root, made again when the current root changes: a switch
/// starts an empty history, as a restart does.
struct Services {
    current: Arc<Current>,
    served: Mutex<Option<(PathBuf, FsService<SapphireApp>)>>,
}

impl Services {
    /// The current root and its service.
    fn get(&self) -> Result<(PathBuf, FsService<SapphireApp>), RpcError> {
        let root = self
            .current
            .root()
            .ok_or_else(|| rpc_error(&Error::NoWorkspace))?;
        let mut served = self.served.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, service)) = served.as_ref()
            && *at == root
        {
            return Ok((root, service.clone()));
        }
        let ws = Workspace::open(&root).map_err(fs_error)?;
        let service = FsService::new(ws, SapphireApp);
        *served = Some((root.clone(), service.clone()));
        Ok((root, service))
    }
}

/// An engine error as the JSON-RPC error a client of `fs-api` expects: its code from
/// `fs_api::codes`, its message.
fn fs_error(err: corundum_framework_fs::FsError) -> RpcError {
    RpcError {
        code: err.code(),
        message: err.to_string(),
        data: None,
    }
}

/// Whether `method` with `params` may have changed the workspace.
fn changes(method: &str, params: &Value) -> bool {
    match method {
        FS_APPLY => serde_json::from_value::<ApplyParams>(params.clone())
            .map(|p| !p.dry_run)
            .unwrap_or(false),
        JOURNAL_UNDO | JOURNAL_REDO | JOURNAL_UNDO_BATCH => true,
        _ => false,
    }
}

/// Add the `fs.*` and `journal.*` methods, on `current`'s workspace, to `router`.
pub(crate) fn fs_methods(current: Arc<Current>, router: Router) -> Router {
    let services = Arc::new(Services {
        current,
        served: Mutex::new(None),
    });
    METHODS.iter().fold(router, |router, &method| {
        let services = Arc::clone(&services);
        router.method(method, move |ctx: RequestCtx| {
            let services = Arc::clone(&services);
            async move {
                let actor = actor_for(&ctx.peer.client().kind);
                let (root, service) = services.get()?;
                let changed = changes(method, &ctx.params);
                let result = service
                    .dispatch(method, ctx.params, &actor)
                    .await
                    .map_err(fs_error);
                // A refused undo changed nothing, but a batch whose every operation was
                // skipped is still a success: scan after any answer that is not an error.
                if changed && result.is_ok() {
                    scanned(services.current.sync(), &root, method).await;
                }
                result
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mcp_client_is_an_agent_and_the_rest_are_the_person() {
        assert_eq!(
            actor_for("mcp"),
            Actor {
                principal: "local".into(),
                agent: true
            }
        );
        for kind in ["desktop", "cli", "sapphire-sync"] {
            assert!(!actor_for(kind).agent, "{kind}");
            assert_eq!(actor_for(kind).principal, "local");
        }
    }

    #[test]
    fn only_a_real_apply_and_the_undo_methods_change_anything() {
        let apply = |dry_run| serde_json::json!({"ops": [], "origin": "o", "dry_run": dry_run});
        assert!(changes(FS_APPLY, &apply(false)));
        assert!(!changes(FS_APPLY, &apply(true)));
        assert!(changes(JOURNAL_UNDO, &serde_json::json!({})));
        assert!(changes(JOURNAL_UNDO_BATCH, &serde_json::json!({})));
        for read in [
            FS_LIST,
            FS_STAT,
            FS_READ,
            FS_VERSION,
            JOURNAL_HISTORY,
            JOURNAL_STATUS,
        ] {
            assert!(!changes(read, &serde_json::json!({})), "{read}");
        }
    }

    #[test]
    fn the_trash_folder_is_not_synced() {
        let dir = tempfile::tempdir().unwrap();
        let filter = sapphire_sync::SyncFilter::load(dir.path(), "sapphire-sync").unwrap();
        assert!(!filter.allows(&format!("{OWN_FOLDER}/trash/x/a.txt"), false));
        assert!(filter.allows(".sapphire-sync/embedded/a.vec", false));
    }
}
