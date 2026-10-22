//! A real `AppServer` relaying the bridge's client-facing methods (#314).
//!
//! The relay forwards each method of `RELAYED` to the bridge under its own name, over the
//! sync runtime's connection when there is one and over its own otherwise; anything else
//! gets `method not found`, and no bridge is `BRIDGE_NOT_RUNNING`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sapphire_bridge_api::{BRIDGE_NOT_RUNNING, StatusResult};
use sapphire_bridge_client::BridgeClient;
use sapphire_framework_server::sync::testing::StubBridge;
use sapphire_framework_server::{AppServer, BridgeConnector, SyncRuntime};
use sapphire_ipc::{ClientInfo, Endpoint, Error, ManagedBy, codes, connect_or_absent};
use sapphire_workspace::{AppContext, AppKind};
use serde_json::{Value, json};

static CTX: AppContext = AppContext::new("sapphire-relay");

const DIR_VARS: [&str; 3] = [
    "SAPPHIRE_RELAY_CACHE_DIR",
    "SAPPHIRE_RELAY_DATA_DIR",
    "SAPPHIRE_RELAY_CONFIG_DIR",
];

/// Point the context's directories at this test's scratch tree, and put them back on drop.
struct EnvGuard {
    previous: [Option<std::ffi::OsString>; 3],
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: `self._lock` still serialises the environment; it is dropped only after
        // this method returns.
        for (name, previous) in DIR_VARS.iter().zip(self.previous.iter_mut()) {
            match previous.take() {
                Some(value) => unsafe { std::env::set_var(name, value) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
    }
}

/// Point the context's directories at `tmp` while holding the environment lock.
fn point_at(tmp: &std::path::Path) -> EnvGuard {
    // The static `CTX` is first-writer-wins, so every test in this binary must serialise on
    // the environment even though they do not share a root.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let previous = DIR_VARS.map(std::env::var_os);
    // SAFETY (via the lock above): serialised against every other use in this binary.
    for (name, dir) in DIR_VARS
        .iter()
        .zip(["cache", "data", "config"].map(|cat| tmp.join(cat)))
    {
        unsafe { std::env::set_var(name, dir) };
    }
    CTX.init(AppKind::Server);
    EnvGuard {
        previous,
        _lock: lock,
    }
}

/// Wait until the server listens on `endpoint`.
async fn wait_until_listening(endpoint: &Endpoint) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !sapphire_ipc::probe(endpoint).await.unwrap() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the server never started listening"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn client_info() -> ClientInfo {
    ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: sapphire_backend::protocol::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    }
}

/// The bridge connections a [`queued`] connector hands out, in order; empty is "no bridge
/// is running".
type Bridges = Arc<Mutex<VecDeque<BridgeClient>>>;

/// A connector handing out `bridges` in order, counting its attempts in `attempts`.
fn queued(bridges: &Bridges, attempts: &Arc<AtomicUsize>) -> BridgeConnector {
    let bridges = Arc::clone(bridges);
    let attempts = Arc::clone(attempts);
    BridgeConnector::new(move || {
        attempts.fetch_add(1, Ordering::Relaxed);
        let next = bridges.lock().unwrap().pop_front();
        async move { Ok(next) }
    })
}

/// A server for one test, and a client of it.
struct Fixture {
    _tmp: tempfile::TempDir,
    _env: EnvGuard,
    client: sapphire_ipc::Client,
    server: tokio::task::JoinHandle<sapphire_framework_server::Result<()>>,
}

impl Fixture {
    /// Serve the app server `build` makes of a fresh one.
    async fn start(build: impl FnOnce(AppServer) -> AppServer) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let env = point_at(tmp.path());
        let endpoint = Endpoint::in_dir("sapphire-relay", tmp.path().to_path_buf());
        let server =
            tokio::spawn(build(AppServer::new(&CTX, "0.0.0").endpoint(endpoint.clone())).run());
        wait_until_listening(&endpoint).await;
        let (client, _) = connect_or_absent(&endpoint, "sapphire-relay", client_info())
            .await
            .unwrap()
            .expect("the server is listening");
        Fixture {
            _tmp: tmp,
            _env: env,
            client,
            server,
        }
    }

    /// A server without sync relaying over a [`queued`] connector.
    async fn own(bridges: &Bridges, attempts: &Arc<AtomicUsize>) -> Fixture {
        let connector = queued(bridges, attempts);
        Fixture::start(|server| server.bridge_connector(connector)).await
    }

    async fn call(&self, method: &str, params: Value) -> sapphire_ipc::Result<Value> {
        self.client.call::<_, Value>(method, params).await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// The JSON-RPC error a call failed with.
fn rpc(result: sapphire_ipc::Result<Value>) -> sapphire_ipc::RpcError {
    match result {
        Err(Error::Rpc(e)) => e,
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

/// A stub bridge's client, queued for a connector.
async fn stub_in_queue(bridges: &Bridges) -> StubBridge {
    let (stub, client) = StubBridge::start_client_in(std::env::temp_dir()).await;
    bridges.lock().unwrap().push_back(client);
    stub
}

#[tokio::test(flavor = "multi_thread")]
async fn an_allowlisted_method_returns_the_bridges_answer() {
    let (bridges, attempts) = (Bridges::default(), Arc::default());
    let stub = stub_in_queue(&bridges).await;
    stub.answer("embed.settings", json!({"x": 1}));
    let f = Fixture::own(&bridges, &attempts).await;

    let answer = f.call("embed.settings", json!({"a": 2})).await.unwrap();
    assert_eq!(answer, json!({"x": 1}));
    assert_eq!(
        stub.calls(),
        vec![("embed.settings".to_owned(), json!({"a": 2}))],
        "the params reach the bridge as the client sent them"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_method_off_the_allowlist_is_not_found() {
    let (bridges, attempts) = (Bridges::default(), Arc::default());
    let stub = stub_in_queue(&bridges).await;
    let f = Fixture::own(&bridges, &attempts).await;

    let e = rpc(f.call("bridge.register", json!({})).await);
    assert_eq!(e.code, codes::METHOD_NOT_FOUND);
    assert!(stub.calls().is_empty());
    assert!(stub.seen.lock().unwrap().registrations.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn no_bridge_answers_bridge_not_running() {
    let (bridges, attempts) = (Bridges::default(), Arc::default());
    let f = Fixture::own(&bridges, &attempts).await;

    let e = rpc(f.call("bridge.status", json!({})).await);
    assert_eq!(e.code, BRIDGE_NOT_RUNNING);
    assert_eq!(e.message, "no sapphire-bridge is running");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_without_sync_relays_over_its_own_connection() {
    let (bridges, attempts) = (Bridges::default(), Arc::default());
    let _stub = stub_in_queue(&bridges).await;
    let f = Fixture::own(&bridges, &attempts).await;

    let status: StatusResult =
        serde_json::from_value(f.call("bridge.status", json!({})).await.unwrap()).unwrap();
    assert_eq!(status.version, "stub");
    // A second call reuses the connection rather than making another.
    f.call("bridge.status", json!({})).await.unwrap();
    assert_eq!(attempts.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_bridge_is_reconnected_on_the_next_call() {
    let (bridges, attempts) = (Bridges::default(), Arc::default());
    let first = stub_in_queue(&bridges).await;
    let second = stub_in_queue(&bridges).await;
    second.answer("embed.settings", json!({"from": "second"}));
    first.disconnect();
    let f = Fixture::own(&bridges, &attempts).await;

    let e = rpc(f.call("embed.settings", json!({})).await);
    assert_eq!(e.code, BRIDGE_NOT_RUNNING);
    let answer = f.call("embed.settings", json!({})).await.unwrap();
    assert_eq!(answer, json!({"from": "second"}));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_bridges_own_errors_come_back_unchanged() {
    let (bridges, attempts) = (Bridges::default(), Arc::default());
    let stub = stub_in_queue(&bridges).await;
    let f = Fixture::own(&bridges, &attempts).await;

    let e = rpc(f.call("config.schema", json!({})).await);
    assert_eq!(e.code, codes::METHOD_NOT_FOUND);
    assert_eq!(e.message, "the stub has no answer for config.schema");
    assert_eq!(stub.calls().len(), 1, "the bridge was asked");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_synced_server_relays_over_the_sync_connection() {
    let (bridges, attempts) = (Bridges::default(), Arc::<AtomicUsize>::default());
    let (_stub, bridge) = StubBridge::start().await;
    let connector = queued(&bridges, &attempts);
    let f = Fixture::start(|server| {
        let runtime = Arc::new(SyncRuntime::new(
            &CTX,
            bridge,
            "/bin/true".into(),
            ManagedBy::Service,
        ));
        server.sync(runtime).bridge_connector(connector)
    })
    .await;

    let status: StatusResult =
        serde_json::from_value(f.call("bridge.status", json!({})).await.unwrap()).unwrap();
    assert_eq!(status.version, "stub");
    assert_eq!(
        attempts.load(Ordering::Relaxed),
        0,
        "the relay must not make a connection of its own"
    );
}
