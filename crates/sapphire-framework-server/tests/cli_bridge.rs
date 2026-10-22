//! Integration tests for framework CLI commands that go through the bridge, by way of the
//! app server's relay.

use std::path::Path;
use std::sync::Arc;

use sapphire_framework_bridge::{Bridge, BridgeDir, LoopbackNetwork, NetConfig};
use sapphire_framework_server::{AppServer, BridgeConnector, DeviceCommand, WorkgroupCommand};
use sapphire_ipc::Endpoint;
use sapphire_workspace::{AppContext, AppKind};

const APP: &str = "sapphire-clibridge";
static CTX: AppContext = AppContext::new(APP);

/// Process-wide lock for environment variable manipulation.
static ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Start a bridge with no workgroup on the default endpoints of the given directory.
///
/// Not the app's runtime directory: the CLI must reach the bridge only through the app server.
async fn start_bridge_no_workgroup(runtime: &Path) -> (Arc<Bridge>, Endpoint) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();

    let control = Endpoint::in_dir("bridge", runtime.to_path_buf());
    let data = Endpoint::in_dir("bridge-data", runtime.to_path_buf());

    let node_id = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    let bridge = Arc::new(
        Bridge::new(
            dir,
            Arc::new(LoopbackNetwork::new().transport(node_id)),
            "0.0.0",
        )
        .unwrap()
        .net(NetConfig::default())
        .control_endpoint(control.clone())
        .data_endpoint(data),
    );

    let shared = Arc::clone(&bridge);
    tokio::spawn(async move {
        let _ = shared.run_shared(NetConfig::default()).await;
    });

    // Wait for the control endpoint to come up
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !sapphire_ipc::probe(&control).await.unwrap_or(false) {
        assert!(
            std::time::Instant::now() < deadline,
            "the bridge never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // Keep tmp alive for the duration of the test
    std::mem::forget(tmp);

    (bridge, control)
}

/// A bridge and an app server relaying to it, in their own runtime directories.
struct Fixture {
    _tmp: tempfile::TempDir,
    _bridge: Arc<Bridge>,
    control: Endpoint,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn start() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let runtime = tmp.path().join("run");
        let bridge_runtime = tmp.path().join("bridge-run");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::create_dir_all(&bridge_runtime).unwrap();
        // SAFETY: callers hold `ENV`, so nothing else reads the environment meanwhile.
        unsafe {
            std::env::set_var(sapphire_ipc::RUNTIME_DIR_ENV, &runtime);
        }
        let (bridge, control) = start_bridge_no_workgroup(&bridge_runtime).await;
        let server = start_app_server(tmp.path(), control.clone());
        wait_for_app_server().await;
        Fixture {
            _tmp: tmp,
            _bridge: bridge,
            control,
            server,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        unsafe {
            std::env::remove_var(sapphire_ipc::RUNTIME_DIR_ENV);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn workgroup_create_succeeds() {
    let _lock = ENV.lock().await;
    let fixture = Fixture::start().await;

    let result = WorkgroupCommand::Create {
        name: "home".into(),
        device_name: "desk".into(),
    }
    .dispatch(APP, "0.0.0")
    .await;

    assert!(result.is_ok(), "Create should succeed: {:?}", result);
    assert_eq!(result.unwrap(), 0, "Create should return exit code 0");

    // The bridge itself holds the workgroup.
    let info = sapphire_ipc::ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: sapphire_bridge_api::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    let (direct, _) = sapphire_ipc::connect_or_absent(&fixture.control, "bridge", info)
        .await
        .unwrap()
        .expect("the bridge is listening");
    let client = sapphire_bridge_client::BridgeClient::from_client(
        Arc::new(direct),
        fixture._tmp.path().to_path_buf(),
    );
    let status = client.status().await.expect("should get status");
    assert_eq!(status.workgroup.expect("the workgroup").name, "home");
}

#[tokio::test(flavor = "multi_thread")]
async fn device_retire_own_device_fails() {
    let _lock = ENV.lock().await;
    let _fixture = Fixture::start().await;

    let create_result = WorkgroupCommand::Create {
        name: "home".into(),
        device_name: "desk".into(),
    }
    .dispatch(APP, "0.0.0")
    .await;
    assert!(create_result.is_ok(), "Create should succeed");

    let retire_result = DeviceCommand::Retire {
        selector: "desk".into(),
    }
    .dispatch(APP, "0.0.0")
    .await;

    assert!(
        retire_result.is_err(),
        "Retire own device should fail: {:?}",
        retire_result
    );

    let err_msg = format!("{:?}", retire_result.unwrap_err());
    assert!(
        err_msg.to_lowercase().contains("own device"),
        "Error should mention 'own device', got: {}",
        err_msg
    );
}

/// Run an app server for [`APP`] (at the environment's runtime directory) that relays to the bridge at `control`.
fn start_app_server(tmp: &Path, control: Endpoint) -> tokio::task::JoinHandle<()> {
    // SAFETY: callers hold `ENV`, so nothing else reads the environment meanwhile.
    unsafe {
        for cat in ["cache", "data", "config"] {
            std::env::set_var(
                format!("SAPPHIRE_CLIBRIDGE_{}_DIR", cat.to_uppercase()),
                tmp.join(cat),
            );
        }
    }
    CTX.init(AppKind::Server);
    let server = AppServer::new(&CTX, "0.0.0")
        .endpoint(Endpoint::for_app(APP).unwrap())
        .selection_file(tmp.join("workspace.toml"))
        .bridge_connector(BridgeConnector::at(control, "test", "0.0.0"));
    tokio::spawn(async move {
        let _ = server.run().await;
    })
}

async fn wait_for_app_server() {
    let endpoint = Endpoint::for_app(APP).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !sapphire_ipc::probe(&endpoint).await.unwrap_or(false) {
        assert!(
            std::time::Instant::now() < deadline,
            "the app server never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_commands_without_an_app_server_say_so() {
    let _lock = ENV.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var(sapphire_ipc::RUNTIME_DIR_ENV, tmp.path());
    }

    let result = WorkgroupCommand::List.dispatch(APP, "0.0.0").await;
    assert_eq!(result.unwrap(), 1);

    unsafe {
        std::env::remove_var(sapphire_ipc::RUNTIME_DIR_ENV);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn workspace_show_lists_the_workgroup_through_the_app_server() {
    let _lock = ENV.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let runtime = tmp.path().join("run");
    let bridge_runtime = tmp.path().join("bridge-run");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::create_dir_all(&bridge_runtime).unwrap();
    unsafe {
        std::env::set_var(sapphire_ipc::RUNTIME_DIR_ENV, &runtime);
    }
    let (bridge, control) = start_bridge_no_workgroup(&bridge_runtime).await;
    let server = start_app_server(tmp.path(), control);
    wait_for_app_server().await;
    let created = WorkgroupCommand::Create {
        name: "home".into(),
        device_name: "desk".into(),
    }
    .dispatch(APP, "0.0.0")
    .await;
    assert_eq!(created.unwrap(), 0);

    let info = sapphire_ipc::ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: sapphire_backend::protocol::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    let (client, _) = sapphire_ipc::connect_or_absent(&Endpoint::for_app(APP).unwrap(), APP, info)
        .await
        .unwrap()
        .expect("the app server is listening");
    let mut out = String::new();
    let code = sapphire_framework_server::render_workspace_show(Arc::new(client), APP, &mut out)
        .await
        .unwrap();
    assert_eq!(code, 1, "no workspace yet: {out}");
    assert!(out.contains("no workspace yet"), "{out}");
    assert!(
        !out.contains("no sapphire-bridge is running"),
        "the bridge is reached through the app server: {out}"
    );

    server.abort();
    drop(bridge);
    unsafe {
        std::env::remove_var(sapphire_ipc::RUNTIME_DIR_ENV);
    }
}
