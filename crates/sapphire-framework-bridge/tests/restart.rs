//! Restarting the bridge in place: `bridge.restart` over IPC, served by `serve_restarting`.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use sapphire_bridge_api::{ConfigEntry, Value};
use sapphire_bridge_client::BridgeClient;
use sapphire_framework_bridge::{
    Bridge, BridgeDir, LoopbackNetwork, Result, Workgroup, serve_restarting,
};
use sapphire_ipc::{Endpoint, ServerInfo};

/// One host's directories and endpoints, and a bridge served over them by
/// `serve_restarting`.
struct Restarting {
    /// Held so the directories outlive the test.
    _tmp: tempfile::TempDir,
    control: Endpoint,
    runtime: PathBuf,
    /// How many bridges the builder has built.
    builds: Arc<AtomicUsize>,
    /// The serving task, kept so a test can see it is still running.
    serving: tokio::task::JoinHandle<Result<()>>,
}

/// Serve a bridge over a loopback transport with `serve_restarting`, and wait until it
/// answers.
async fn start(net: &LoopbackNetwork) -> Restarting {
    let tmp = tempfile::tempdir().unwrap();
    let runtime = tmp.path().join("run");
    std::fs::create_dir_all(&runtime).unwrap();
    let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
    // A workgroup, so each build opens the workgroup's replica again: the old one has to be
    // closed by then.
    Workgroup::create(&dir, "test", "host-a", common::NODE_A).unwrap();

    let control = Endpoint::in_dir("bridge", runtime.clone());
    let data = Endpoint::in_dir("bridge-data", runtime.clone());
    let builds = Arc::new(AtomicUsize::new(0));
    let serving = tokio::spawn(serve_restarting({
        let net = net.clone();
        let control = control.clone();
        let builds = Arc::clone(&builds);
        move || {
            builds.fetch_add(1, Ordering::SeqCst);
            let bridge = Bridge::new(
                dir.clone(),
                Arc::new(net.transport(common::NODE_A)),
                "0.0.0",
            )
            .map(|bridge| {
                bridge
                    .control_endpoint(control.clone())
                    .data_endpoint(data.clone())
            });
            async move { bridge }
        }
    }));
    let host = Restarting {
        _tmp: tmp,
        control,
        runtime,
        builds,
        serving,
    };
    wait_until_up(&host).await;
    host
}

/// Wait until the control endpoint answers. A bridge that stopped instead says why.
async fn wait_until_up(host: &Restarting) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !sapphire_ipc::probe(&host.control).await.unwrap_or(false) {
        assert!(
            !host.serving.is_finished(),
            "the bridge stopped serving instead of restarting"
        );
        assert!(
            std::time::Instant::now() < deadline,
            "the bridge did not come back"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Connect to the bridge, with what it said about itself in the handshake.
async fn connect(host: &Restarting) -> (BridgeClient, ServerInfo) {
    let (client, info) =
        sapphire_ipc::connect_or_absent(&host.control, "bridge", common::client_info())
            .await
            .unwrap()
            .expect("the bridge is listening");
    (
        BridgeClient::from_client(Arc::new(client), host.runtime.clone()),
        info,
    )
}

/// Wait until `client`'s connection closes: the old bridge has stopped serving.
async fn wait_closed(client: &BridgeClient) {
    tokio::time::timeout(Duration::from_secs(10), client.closed())
        .await
        .expect("the restart closed the control connection");
}

#[tokio::test]
async fn restart_rebuilds_and_serves_again() {
    let net = LoopbackNetwork::new();
    let host = start(&net).await;
    assert_eq!(host.builds.load(Ordering::SeqCst), 1);

    let (client, _) = connect(&host).await;
    // The reply arrives before the connection goes.
    client.restart().await.unwrap();
    wait_closed(&client).await;

    wait_until_up(&host).await;
    let (client, info) = connect(&host).await;
    client.status().await.unwrap();
    assert_eq!(host.builds.load(Ordering::SeqCst), 2);
    // In place: the same process serves the new bridge.
    assert_eq!(info.pid, std::process::id());
    assert!(!host.serving.is_finished());
}

#[tokio::test]
async fn a_restart_picks_up_a_changed_restart_key() {
    let net = LoopbackNetwork::new();
    let host = start(&net).await;
    let (client, _) = connect(&host).await;

    let result = client
        .config_set(
            None,
            vec![ConfigEntry {
                key: "net.wake_on_sync".to_owned(),
                value: Value::Bool(false),
            }],
        )
        .await
        .unwrap();
    assert_eq!(result.pending_restart, vec!["net.wake_on_sync".to_owned()]);

    client.restart().await.unwrap();
    wait_closed(&client).await;
    wait_until_up(&host).await;

    let (client, _) = connect(&host).await;
    let listed = client.config_list(None).await.unwrap();
    let pending: Vec<_> = listed
        .entries
        .iter()
        .filter(|e| e.pending_restart)
        .map(|e| e.key.clone())
        .collect();
    assert!(pending.is_empty(), "still pending: {pending:?}");
    let found = listed
        .entries
        .iter()
        .find(|e| e.key == "net.wake_on_sync")
        .expect("listed");
    assert_eq!(
        found.value,
        sapphire_bridge_api::EntryValue::Value(Value::Bool(false))
    );
}

#[tokio::test]
async fn a_connected_client_sees_the_connection_close_and_can_reconnect() {
    let net = LoopbackNetwork::new();
    let host = start(&net).await;
    let (asking, _) = connect(&host).await;
    let (watching, _) = connect(&host).await;
    watching.status().await.unwrap();

    asking.restart().await.unwrap();
    // Every connection to the old bridge ends — not only the one that asked.
    wait_closed(&watching).await;
    assert!(watching.is_closed());

    wait_until_up(&host).await;
    let (again, _) = connect(&host).await;
    again.status().await.unwrap();
}
