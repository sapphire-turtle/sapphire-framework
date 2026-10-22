//! The phone's topology on a desktop: the bridge, the app server and the GUI's client in one
//! process, over the in-process carrier — no socket anywhere.
//!
//! Its own test binary, because the carrier is chosen per process.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use sapphire_framework_bridge::{Bridge, BridgeDir, LoopbackNetwork, NetConfig};
use sapphire_framework_gui::client::*;
use sapphire_framework_server::{AppServer, BridgeConnector, SyncRuntime};
use sapphire_ipc::{Carrier, Endpoint, ManagedBy};
use sapphire_workspace::{AppContext, AppKind};

static CTX: AppContext = AppContext::new("sapphire-inproc");
const APP: AppIdentity = AppIdentity {
    app_name: "sapphire-inproc",
    version: "0.0.0",
};
const NODE: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

async fn wait<F, Fut>(mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(10);
    while !check().await {
        assert!(Instant::now() < deadline, "condition never held");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Every file under `dir` whose name ends in `.sock`, or that is a socket.
fn sockets_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            found.extend(sockets_under(&path));
        } else {
            #[cfg(unix)]
            let is_socket = std::os::unix::fs::FileTypeExt::is_socket(&file_type);
            #[cfg(not(unix))]
            let is_socket = false;
            if is_socket || path.extension().is_some_and(|e| e == "sock") {
                found.push(path);
            }
        }
    }
    found
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gui_client_sees_bridge_and_server_without_sockets() {
    sapphire_ipc::set_carrier(Carrier::InProcess);
    let tmp = tempfile::tempdir().unwrap();
    for (var, cat) in [("CACHE", "cache"), ("DATA", "data"), ("CONFIG", "config")] {
        // SAFETY: this binary's only test sets them, before anything reads them.
        unsafe { std::env::set_var(format!("SAPPHIRE_INPROC_{var}_DIR"), tmp.path().join(cat)) };
    }
    CTX.init(AppKind::Server);

    let run = tmp.path().join("run");
    let endpoints = Endpoints {
        app: Endpoint::in_dir(APP.app_name, run.clone()),
    };
    let control = Endpoint::in_dir("bridge", run.clone());

    // The bridge.
    let net = LoopbackNetwork::new();
    let bridge = Bridge::new(
        BridgeDir::at(tmp.path().join("bridge")).unwrap(),
        Arc::new(net.transport(NODE)),
        "0.0.0",
    )
    .unwrap()
    .control_endpoint(control.clone())
    .data_endpoint(Endpoint::in_dir("bridge-data", run.clone()));
    tokio::spawn(async move { Arc::new(bridge).run_shared(NetConfig::default()).await });

    // The app server, reaching the bridge the way a phone's does, without signal handlers.
    let runtime = Arc::new(SyncRuntime::connecting(
        &CTX,
        BridgeConnector::at(control, "test", "0.0.0"),
        PathBuf::from("/nonexistent/app"),
        ManagedBy::Service,
    ));
    let server = AppServer::new(&CTX, "0.0.0")
        .endpoint(endpoints.app.clone())
        .signals(false)
        .sync(runtime);
    tokio::spawn(async move { server.run().await });

    // The GUI's client.
    let client = FrameworkClient::spawn(
        &tokio::runtime::Handle::current(),
        ClientConfig {
            app: APP,
            endpoints,
            service_exes: ServiceExes {
                bridge: PathBuf::from("/nonexistent/bridge"),
                app: PathBuf::from("/nonexistent/app"),
            },
            refresh: Duration::from_millis(100),
            fetch_timeout: Duration::from_secs(5),
            command_timeout: Duration::from_secs(30),
            install_timeout: Duration::from_secs(300),
        },
        Arc::new(|| {}),
    );
    wait(|| async {
        let s = client.snapshot();
        s.bridge.up().is_some() && s.server.up().is_some()
    })
    .await;

    // A command that goes through the bridge's control plane works too.
    let id = client.send(Command::WorkgroupCreate {
        name: "home".into(),
        device_name: "phone".into(),
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let result = loop {
        if let Some(o) = client.drain_outcomes().into_iter().find(|o| o.id == id) {
            break o.result;
        }
        assert!(Instant::now() < deadline, "no outcome");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(result, Ok(CommandOutput::Done));

    assert_eq!(sockets_under(tmp.path()), Vec::<PathBuf>::new());
}
