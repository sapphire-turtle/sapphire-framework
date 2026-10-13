//! The CLI's `workspace sync …` and `workspace select --sync`, through the real
//! `WorkspaceCommand::dispatch` against a running server.
//!
//! The CLI finds the server at `Endpoint::for_app`, under `SAPPHIRE_RUNTIME_DIR`, so this
//! binary points that variable — and the context's directories — at one scratch tree, and
//! has a single test: the environment is the process's.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sapphire_backend::protocol as proto;
use sapphire_framework_server::sync::testing::StubBridge;
use sapphire_framework_server::{AppServer, SyncRuntime, WorkspaceCommand, WorkspaceSyncCommand};
use sapphire_ipc::{Client, ClientInfo, Endpoint, ManagedBy, connect_or_absent};
use sapphire_workspace::{AppContext, AppKind};

static CTX: AppContext = AppContext::new("sapphire-wssynccli");

async fn client(endpoint: &Endpoint) -> Client {
    let info = ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    connect_or_absent(endpoint, CTX.app_name, info)
        .await
        .unwrap()
        .unwrap()
        .0
}

async fn current(c: &Client) -> proto::CurrentWorkspace {
    c.call::<_, proto::WorkspaceCurrentResult>(proto::WORKSPACE_CURRENT, serde_json::json!({}))
        .await
        .unwrap()
        .workspace
        .expect("a current workspace")
}

async fn run(command: WorkspaceCommand) -> i32 {
    command.dispatch(CTX.app_name, "0.0.0").await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn sync_is_switched_by_select_and_by_workspace_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    // SAFETY: the only test in this binary, before anything else reads the environment.
    unsafe {
        std::env::set_var(sapphire_ipc::RUNTIME_DIR_ENV, &run_dir);
        for cat in ["cache", "data", "config"] {
            std::env::set_var(
                format!("SAPPHIRE_WSSYNCCLI_{}_DIR", cat.to_uppercase()),
                tmp.path().join(cat),
            );
        }
    }
    CTX.init(AppKind::Server);

    let endpoint = Endpoint::for_app(CTX.app_name).unwrap();
    let (stub, bridge) = StubBridge::start().await;
    let runtime = Arc::new(SyncRuntime::new(
        &CTX,
        bridge,
        PathBuf::from("/bin/true"),
        ManagedBy::Service,
    ));
    let server = AppServer::new(&CTX, "0.0.0").sync(runtime);
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !sapphire_ipc::probe(&endpoint).await.unwrap() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the server never listened"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let c = client(&endpoint).await;

    // A workspace that has never synced.
    let first = tmp.path().join("first");
    std::fs::create_dir_all(&first).unwrap();
    let init = WorkspaceCommand::Init {
        dir: Some(first.clone()),
        sync: false,
    };
    assert_eq!(run(init).await, 0);
    let ws = current(&c).await;
    assert!(!ws.sync.enabled);
    assert_eq!(ws.workspace_id, None);
    assert_eq!(
        run(WorkspaceCommand::Sync(WorkspaceSyncCommand::Status)).await,
        0
    );

    // `workspace sync enable` / `disable` switch it after the fact; disabling keeps the id.
    assert_eq!(
        run(WorkspaceCommand::Sync(WorkspaceSyncCommand::Enable)).await,
        0
    );
    let ws = current(&c).await;
    assert!(ws.sync.enabled);
    let id = ws.workspace_id.expect("enabling mints a sync id");
    assert_eq!(
        run(WorkspaceCommand::Sync(WorkspaceSyncCommand::Status)).await,
        0
    );
    assert_eq!(
        run(WorkspaceCommand::Sync(WorkspaceSyncCommand::Disable)).await,
        0
    );
    let ws = current(&c).await;
    assert!(!ws.sync.enabled);
    assert_eq!(
        ws.workspace_id,
        Some(id),
        "a disabled workspace keeps its id"
    );

    // `select --sync` on an existing workspace that never synced turns it on.
    let second = tmp.path().join("second");
    std::fs::create_dir_all(&second).unwrap();
    let init = WorkspaceCommand::Init {
        dir: Some(second.clone()),
        sync: false,
    };
    assert_eq!(run(init).await, 0);
    let select_first = WorkspaceCommand::Select {
        dir: Some(first.clone()),
        sync: false,
    };
    assert_eq!(run(select_first).await, 0);
    assert_eq!(current(&c).await.root, first.canonicalize().unwrap());
    let select_second = WorkspaceCommand::Select {
        dir: Some(second.clone()),
        sync: true,
    };
    assert_eq!(run(select_second).await, 0);
    let ws = current(&c).await;
    assert_eq!(ws.root, second.canonicalize().unwrap());
    assert!(ws.sync.enabled, "select --sync turned sync on");
    assert!(ws.workspace_id.is_some());

    drop(stub);
    let _: serde_json::Value = c
        .call(sapphire_ipc::SHUTDOWN_METHOD, serde_json::json!({}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the server stopped")
        .unwrap();
}
