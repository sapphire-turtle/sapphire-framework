use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sapphire_bridge_api::settings::{EntryValue, Process};
use sapphire_bridge_api::{ConfigEntry, Entry, Scope, Value};
use sapphire_framework_bridge::{Bridge, BridgeDir, LoopbackNetwork, NetConfig};
use sapphire_framework_gui::client::*;
use sapphire_framework_server::{AppServer, BridgeConnector, Settings, SyncRuntime};
use sapphire_ipc::{Endpoint, ManagedBy};
use sapphire_workspace::{AppContext, AppKind};

static CTX: AppContext = AppContext::new("sapphire-guitest");
const APP: AppIdentity = AppIdentity {
    app_name: "sapphire-guitest",
    version: "0.0.0",
};
const NODE: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

/// Serialises the env vars `CTX` reads; one test at a time owns them.
static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Fixture {
    tmp: tempfile::TempDir,
    _env: std::sync::MutexGuard<'static, ()>,
    endpoints: Endpoints,
    /// The bridge's control plane: the app server's to reach, never the GUI's.
    bridge: Endpoint,
}

fn fixture() -> Fixture {
    let env = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    for (var, cat) in [("CACHE", "cache"), ("DATA", "data"), ("CONFIG", "config")] {
        // SAFETY: serialised by `ENV` for every test in this binary.
        unsafe { std::env::set_var(format!("SAPPHIRE_GUITEST_{var}_DIR"), tmp.path().join(cat)) };
    }
    CTX.init(AppKind::Server);
    // `CTX`'s directories are the first test's for the whole binary: a workspace an earlier
    // test selected would still be selected.
    let selection = CTX
        .config_dir()
        .join(sapphire_framework_server::SELECTION_FILE);
    if let Err(e) = std::fs::remove_file(&selection) {
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound, "{e}");
    }
    let run = tmp.path().join("run");
    std::fs::create_dir_all(&run).unwrap();
    // The bridge listens in a runtime directory of its own: the GUI reaches it only
    // through the app server.
    let bridge_run = tmp.path().join("bridge-run");
    std::fs::create_dir_all(&bridge_run).unwrap();
    let endpoints = Endpoints {
        app: Endpoint::in_dir(APP.app_name, run),
    };
    Fixture {
        tmp,
        _env: env,
        endpoints,
        bridge: Endpoint::in_dir("bridge", bridge_run),
    }
}

fn config(f: &Fixture) -> ClientConfig {
    ClientConfig {
        app: APP,
        endpoints: f.endpoints.clone(),
        service_exes: ServiceExes {
            bridge: PathBuf::from("/nope/bridge"),
            app: PathBuf::from("/nope/app"),
        },
        refresh: Duration::from_millis(100),
        fetch_timeout: Duration::from_secs(5),
        command_timeout: Duration::from_secs(30),
        install_timeout: Duration::from_secs(300),
    }
}

/// A bridge running on a runtime of its own, so that stopping it ends every task it spawned.
///
/// Aborting one task would leave the per-connection handlers alive, and the client's cached
/// connection would keep answering — not what a bridge process exiting looks like.
struct BridgeProcess(Option<tokio::runtime::Runtime>);

impl BridgeProcess {
    fn stop(mut self) {
        if let Some(rt) = self.0.take() {
            rt.shutdown_background();
        }
    }
}

impl Drop for BridgeProcess {
    fn drop(&mut self) {
        if let Some(rt) = self.0.take() {
            rt.shutdown_background();
        }
    }
}

/// A bridge with no workgroup, on the fixture's endpoints.
async fn start_bridge(f: &Fixture) -> BridgeProcess {
    start_bridge_as(f, &LoopbackNetwork::new(), NODE, "bridge", &f.bridge).await
}

/// A bridge with no workgroup: node `node` on `net`, its directory `<tmp>/<name>`, its
/// control plane on `control`.
async fn start_bridge_as(
    f: &Fixture,
    net: &LoopbackNetwork,
    node: &str,
    name: &str,
    control: &Endpoint,
) -> BridgeProcess {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let guard = rt.enter();
    let dir = BridgeDir::at(f.tmp.path().join(name)).unwrap();
    let data = Endpoint::in_dir(format!("{name}-data"), control.dir.clone());
    let bridge = Bridge::new(dir, Arc::new(net.transport(node)), "0.0.0")
        .unwrap()
        .control_endpoint(control.clone())
        .data_endpoint(data);
    drop(guard);
    rt.spawn(async move {
        let _ = Arc::new(bridge).run_shared(NetConfig::default()).await;
    });
    wait(|| async { sapphire_ipc::probe(control).await.unwrap() }).await;
    BridgeProcess(Some(rt))
}

/// A sync-mounted app server on the fixture's endpoint, reaching the fixture's bridge
/// whenever one runs.
async fn start_server(f: &Fixture) -> tokio::task::JoinHandle<()> {
    let runtime = Arc::new(SyncRuntime::connecting(
        &CTX,
        BridgeConnector::at(f.bridge.clone(), "test", "0.0.0"),
        PathBuf::from("/bin/true"),
        ManagedBy::Service,
    ));
    let server = AppServer::new(&CTX, "0.0.0")
        .endpoint(f.endpoints.app.clone())
        .signals(false)
        .sync(runtime);
    let task = tokio::spawn(async move {
        let _ = server.run().await;
    });
    wait(|| async { sapphire_ipc::probe(&f.endpoints.app).await.unwrap() }).await;
    task
}

/// An app server without sync on `app`, relaying to the bridge on `bridge`.
async fn start_relay(app: &Endpoint, bridge: &Endpoint) -> tokio::task::JoinHandle<()> {
    let server = AppServer::new(&CTX, "0.0.0")
        .endpoint(app.clone())
        .signals(false)
        .bridge_connector(BridgeConnector::at(bridge.clone(), "test", "0.0.0"));
    let task = tokio::spawn(async move {
        let _ = server.run().await;
    });
    wait(|| async { sapphire_ipc::probe(app).await.unwrap() }).await;
    task
}

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

async fn outcome(client: &FrameworkClient, id: CommandId) -> Result<CommandOutput, CommandError> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(o) = client.drain_outcomes().into_iter().find(|o| o.id == id) {
            return o.result;
        }
        assert!(Instant::now() < deadline, "no outcome for {id:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn spawn(f: &Fixture) -> FrameworkClient {
    FrameworkClient::spawn(
        &tokio::runtime::Handle::current(),
        config(f),
        Arc::new(|| {}),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_running_reads_absent_twice() {
    let f = fixture();
    let client = spawn(&f);
    wait(|| async { client.snapshot().fetched }).await;
    let s = (*client.snapshot()).clone();
    // Without its app server the GUI cannot know whether a bridge runs.
    assert!(
        matches!(&s.bridge, Conn::Error(m) if m.contains("server is not running")),
        "{:?}",
        s.bridge
    );
    assert!(matches!(s.server, Conn::Absent));
    let id = client.send(Command::SyncEnable);
    let err = outcome(&client, id).await.unwrap_err();
    assert!(err.message.contains("not running"), "{err:?}");
    let id = client.send(Command::DeviceInvite {
        name: "laptop".into(),
        ttl_secs: None,
    });
    let err = outcome(&client, id).await.unwrap_err();
    assert!(err.message.contains("server is not running"), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_without_a_bridge_reads_the_bridge_absent() {
    let f = fixture();
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().server.up().is_some() }).await;
    assert!(
        matches!(client.snapshot().bridge, Conn::Absent),
        "{:?}",
        client.snapshot().bridge
    );
    let id = client.send(Command::DeviceInvite {
        name: "laptop".into(),
        ttl_secs: None,
    });
    let err = outcome(&client, id).await.unwrap_err();
    assert_eq!(err.message, "sapphire-bridge is not running");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bridge_that_stops_reads_absent_and_one_that_returns_reads_up() {
    let f = fixture();
    let bridge = start_bridge(&f).await;
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;
    bridge.stop();
    wait(|| async { matches!(client.snapshot().bridge, Conn::Absent | Conn::Error(_)) }).await;
    wait(|| async { matches!(client.snapshot().bridge, Conn::Absent) }).await;
    let _bridge = start_bridge(&f).await;
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn create_invite_and_retire_round_trip() {
    let f = fixture();
    let _bridge = start_bridge(&f).await;
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;

    let id = client.send(Command::WorkgroupCreate {
        name: "home".into(),
        device_name: "desk".into(),
    });
    assert_eq!(outcome(&client, id).await, Ok(CommandOutput::Done));
    wait(|| async {
        client
            .snapshot()
            .bridge
            .up()
            .and_then(|b| b.status.workgroup.as_ref())
            .map(|w| w.name == "home")
            == Some(true)
    })
    .await;

    let id = client.send(Command::DeviceInvite {
        name: "laptop".into(),
        ttl_secs: Some(3600),
    });
    match outcome(&client, id).await {
        Ok(CommandOutput::Ticket(t)) => assert!(!t.is_empty()),
        other => panic!("expected a ticket, got {other:?}"),
    }

    let id = client.send(Command::DeviceRetire {
        selector: "desk".into(),
    });
    let err = outcome(&client, id).await.unwrap_err();
    assert!(err.message.contains("own device"), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn workspace_init_sync_toggle_and_switch() {
    let f = fixture();
    let _bridge = start_bridge(&f).await;
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;
    let id = client.send(Command::WorkgroupCreate {
        name: "home".into(),
        device_name: "desk".into(),
    });
    outcome(&client, id).await.unwrap();
    wait(|| async { client.snapshot().server.up().is_some() }).await;
    let current = |client: &FrameworkClient| {
        client
            .snapshot()
            .server
            .up()
            .and_then(|s| s.current.clone())
    };

    let notes = f.tmp.path().join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let id = client.send(Command::WorkspaceInit {
        dir: notes.clone(),
        sync: true,
    });
    outcome(&client, id).await.unwrap();
    wait(|| async { current(&client).is_some_and(|w| w.sync.enabled) }).await;
    let notes = current(&client).unwrap().root;

    let id = client.send(Command::SyncDisable);
    outcome(&client, id).await.unwrap();
    wait(|| async { current(&client).is_some_and(|w| !w.sync.enabled) }).await;

    // Another folder becomes the workspace; then back to the first, which syncs again
    // because it has a sync id.
    let other = f.tmp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let id = client.send(Command::WorkspaceInit {
        dir: other.clone(),
        sync: false,
    });
    outcome(&client, id).await.unwrap();
    wait(|| async { current(&client).is_some_and(|w| w.root.ends_with("other")) }).await;
    let id = client.send(Command::WorkspaceSelect { dir: notes.clone() });
    outcome(&client, id).await.unwrap();
    wait(|| async { current(&client).is_some_and(|w| w.root == notes && w.sync.enabled) }).await;
}

/// A workgroup, a running server, and one synced workspace published into the ledger.
/// Returns the client and the published workspace's id.
async fn published_workspace(f: &Fixture) -> (FrameworkClient, grain_id::GrainId) {
    let client = spawn(f);
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;
    let id = client.send(Command::WorkgroupCreate {
        name: "home".into(),
        device_name: "desk".into(),
    });
    outcome(&client, id).await.unwrap();
    wait(|| async { client.snapshot().server.up().is_some() }).await;

    let dir = f.tmp.path().join("notes");
    std::fs::create_dir_all(&dir).unwrap();
    let id = client.send(Command::WorkspaceInit { dir, sync: true });
    outcome(&client, id).await.unwrap();
    wait(|| async {
        client
            .snapshot()
            .bridge
            .up()
            .is_some_and(|b| b.ledger.iter().any(|w| w.app_name == APP.app_name))
    })
    .await;
    let workspace_id = client.snapshot().bridge.up().unwrap().ledger[0].workspace_id;
    (client, workspace_id)
}

#[tokio::test(flavor = "multi_thread")]
async fn workspace_map_creates_the_folder_and_syncs_it_under_the_ledger_id() {
    let f = fixture();
    let _bridge = start_bridge(&f).await;
    let _server = start_server(&f).await;
    let (client, workspace_id) = published_workspace(&f).await;

    // Switch this host to another workspace, so the published one is one it does not
    // serve — what "Bring to this host…" offers.
    let plain = f.tmp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let id = client.send(Command::WorkspaceInit {
        dir: plain,
        sync: false,
    });
    outcome(&client, id).await.unwrap();

    // A folder that does not exist yet, as `bring_target` makes it.
    let dir = f.tmp.path().join("elsewhere").join("notes");
    let id = client.send(Command::WorkspaceMap {
        workspace_id,
        dir: dir.clone(),
    });
    assert_eq!(outcome(&client, id).await, Ok(CommandOutput::Done));
    assert!(
        dir.join(format!(".{}", APP.app_name)).is_dir(),
        "the marker"
    );
    wait(|| async {
        client.snapshot().server.up().is_some_and(|s| {
            s.current
                .as_ref()
                .is_some_and(|w| w.sync.enabled && w.workspace_id == Some(workspace_id))
        })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn service_install_with_a_missing_binary_says_what_to_run() {
    let f = fixture();
    let mut cfg = config(&f);
    let missing = f.tmp.path().join("no-such-dir").join("sapphire-bridge");
    cfg.service_exes.bridge = missing.clone();
    let client = FrameworkClient::spawn(&tokio::runtime::Handle::current(), cfg, Arc::new(|| {}));
    let id = client.send(Command::ServiceInstall(ServiceTarget::Bridge));
    let err = outcome(&client, id).await.unwrap_err();
    assert!(err.message.contains("run: "), "{err:?}");
    assert!(
        err.message.contains(&missing.display().to_string()),
        "{err:?}"
    );
    assert!(err.message.contains("service install"), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn workgroup_join_pairs_with_another_bridge() {
    const NODE_B: &str = "b1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    let f = fixture();
    let net = LoopbackNetwork::new();
    // A, the inviter, behind an app server of its own; B, the joiner, behind the one
    // `config` points at.
    let control_a = Endpoint::in_dir("bridge-a", f.bridge.dir.clone());
    let _a = start_bridge_as(&f, &net, NODE, "bridge-a", &control_a).await;
    let _b = start_bridge_as(&f, &net, NODE_B, "bridge", &f.bridge).await;
    let app_a = Endpoint::in_dir("app-a", f.endpoints.app.dir.clone());
    let _server_a = start_relay(&app_a, &control_a).await;
    let _server_b = start_server(&f).await;

    let mut cfg_a = config(&f);
    cfg_a.endpoints.app = app_a;
    let client_a =
        FrameworkClient::spawn(&tokio::runtime::Handle::current(), cfg_a, Arc::new(|| {}));
    wait(|| async { client_a.snapshot().bridge.up().is_some() }).await;
    let id = client_a.send(Command::WorkgroupCreate {
        name: "home".into(),
        device_name: "a".into(),
    });
    outcome(&client_a, id).await.unwrap();
    let id = client_a.send(Command::DeviceInvite {
        name: "b".into(),
        ttl_secs: Some(3600),
    });
    let Ok(CommandOutput::Ticket(ticket)) = outcome(&client_a, id).await else {
        panic!("expected a ticket");
    };

    let client_b = spawn(&f);
    wait(|| async { client_b.snapshot().bridge.up().is_some() }).await;
    let id = client_b.send(Command::WorkgroupJoin {
        ticket,
        device_name: Some("b".into()),
    });
    assert_eq!(outcome(&client_b, id).await, Ok(CommandOutput::Done));
    wait(|| async {
        client_b
            .snapshot()
            .bridge
            .up()
            .and_then(|b| b.status.workgroup.as_ref())
            .is_some_and(|w| w.name == "home")
    })
    .await;
}

/// Accept connections on `endpoint` and never answer them.
async fn hang(endpoint: &Endpoint) -> tokio::task::JoinHandle<()> {
    #[cfg(unix)]
    let listener = sapphire_ipc::bind(endpoint).await.unwrap();
    #[cfg(windows)]
    let mut listener = sapphire_ipc::bind(endpoint).unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok(conn) = listener.accept().await {
            held.push(conn);
        }
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_process_that_never_answers_reads_error_and_does_not_stall_the_loop() {
    let f = fixture();
    let _hung = hang(&f.endpoints.app).await;
    let mut cfg = config(&f);
    cfg.fetch_timeout = Duration::from_millis(300);
    cfg.command_timeout = Duration::from_millis(300);
    let client = FrameworkClient::spawn(&tokio::runtime::Handle::current(), cfg, Arc::new(|| {}));
    // The loop kept ticking: both halves, which share the one connection, read as not
    // answering.
    wait(|| async {
        let s = client.snapshot();
        matches!(&s.bridge, Conn::Error(m) if m.contains("did not answer"))
            && matches!(&s.server, Conn::Error(m) if m.contains("did not answer"))
    })
    .await;
    // A command queued behind the hung server still gets an outcome.
    let id = client.send(Command::DeviceInvite {
        name: "laptop".into(),
        ttl_secs: None,
    });
    let err = outcome(&client, id).await.unwrap_err();
    assert!(err.message.contains("did not answer"), "{err:?}");
}

// ── settings (#314) ─────────────────────────────────────────────────────────
//
// `CTX`'s directories are the first test's for the whole binary, so the app server's
// settings files are shared: each test writes keys of its own, or puts them back.

/// A bridge that `bridge.restart` rebuilds in place, as the real one does.
async fn start_restarting_bridge(f: &Fixture) -> BridgeProcess {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let net = LoopbackNetwork::new();
    let dir = BridgeDir::at(f.tmp.path().join("bridge")).unwrap();
    let control = f.bridge.clone();
    let data = Endpoint::in_dir("bridge-data", control.dir.clone());
    let build = move || {
        std::future::ready(
            Bridge::new(dir.clone(), Arc::new(net.transport(NODE)), "0.0.0").map(|bridge| {
                bridge
                    .control_endpoint(control.clone())
                    .data_endpoint(data.clone())
            }),
        )
    };
    rt.spawn(async move {
        let _ = sapphire_framework_bridge::serve_restarting(build).await;
    });
    wait(|| async { sapphire_ipc::probe(&f.bridge).await.unwrap() }).await;
    BridgeProcess(Some(rt))
}

/// An app server run by `serve_restarting`, so `server.restart` rebuilds it in place.
async fn start_restarting_server(f: &Fixture) -> tokio::task::JoinHandle<()> {
    let endpoint = f.endpoints.app.clone();
    let bridge = f.bridge.clone();
    let task = tokio::spawn(async move {
        let _ = sapphire_framework_server::serve_restarting(&CTX, (), move |_: &Settings| {
            AppServer::new(&CTX, "0.0.0")
                .endpoint(endpoint.clone())
                .signals(false)
                .bridge_connector(BridgeConnector::at(bridge.clone(), "test", "0.0.0"))
        })
        .await;
    });
    wait(|| async { sapphire_ipc::probe(&f.endpoints.app).await.unwrap() }).await;
    task
}

/// What `ConfigLoad` returns; panics on anything else.
async fn load(client: &FrameworkClient) -> ConfigState {
    let id = client.send(Command::ConfigLoad);
    match outcome(client, id).await {
        Ok(CommandOutput::Config(state)) => state,
        other => panic!("expected the settings, got {other:?}"),
    }
}

/// `key`'s value in `entries`, when it is listed.
fn value_of<'a>(entries: &'a [Entry], key: &str) -> Option<&'a EntryValue> {
    entries.iter().find(|e| e.key == key).map(|e| &e.value)
}

fn set(key: &str, value: Value) -> ConfigEntry {
    ConfigEntry {
        key: key.to_owned(),
        value,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn config_load_returns_schema_and_scopes() {
    let f = fixture();
    let _bridge = start_bridge(&f).await;
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;

    let state = load(&client).await;
    let keys: Vec<&str> = state.schema.iter().map(|d| d.key.as_str()).collect();
    assert!(keys.contains(&"net.discovery"), "{keys:?}");
    assert!(keys.contains(&"http.listen"), "{keys:?}");
    assert!(
        value_of(&state.effective, "net.discovery").is_some(),
        "the effective values"
    );
    assert!(state.by_scope.contains_key(&Scope::Device));
    assert!(state.by_scope.contains_key(&Scope::Workgroup));
    assert!(
        !state.by_scope.contains_key(&Scope::Workspace),
        "no workspace is selected"
    );

    let notes = f.tmp.path().join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let id = client.send(Command::WorkspaceInit {
        dir: notes,
        sync: false,
    });
    outcome(&client, id).await.unwrap();
    let state = load(&client).await;
    assert!(state.by_scope.contains_key(&Scope::Workspace));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mixed_write_lands_in_both_processes() {
    let f = fixture();
    let _bridge = start_bridge(&f).await;
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;

    let id = client.send(Command::ConfigWrite {
        scope: Scope::Device,
        set: vec![
            set("http.metrics.enabled", Value::Bool(true)),
            set("net.discovery", Value::Bool(false)),
        ],
        unset: Vec::new(),
    });
    let Ok(CommandOutput::Written(written)) = outcome(&client, id).await else {
        panic!("expected a write");
    };
    assert!(
        written.changed.contains(&"net.discovery".to_owned())
            && written.changed.contains(&"http.metrics.enabled".to_owned()),
        "{written:?}"
    );
    let device = &load(&client).await.by_scope[&Scope::Device];
    assert_eq!(
        value_of(device, "net.discovery"),
        Some(&EntryValue::Value(Value::Bool(false)))
    );
    assert_eq!(
        value_of(device, "http.metrics.enabled"),
        Some(&EntryValue::Value(Value::Bool(true)))
    );

    // And an unset takes them back out.
    let id = client.send(Command::ConfigWrite {
        scope: Scope::Device,
        set: Vec::new(),
        unset: vec!["http.metrics.enabled".into(), "net.discovery".into()],
    });
    assert!(
        matches!(outcome(&client, id).await, Ok(CommandOutput::Written(_))),
        "the unset"
    );
    let device = &load(&client).await.by_scope[&Scope::Device];
    for key in ["net.discovery", "http.metrics.enabled"] {
        assert!(
            !matches!(value_of(device, key), Some(EntryValue::Value(_))),
            "{key} is unset: {device:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_write_keeps_its_field_errors() {
    let f = fixture();
    let _bridge = start_bridge(&f).await;
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;

    let id = client.send(Command::ConfigWrite {
        scope: Scope::Device,
        set: vec![
            set("http.mcp.enabled", Value::Bool(true)),
            set("net.discovery", Value::String("not a bool".into())),
        ],
        unset: Vec::new(),
    });
    let err = outcome(&client, id).await.unwrap_err();
    assert!(!err.fields.is_empty(), "{err:?}");
    assert_eq!(err.fields[0].key, "net.discovery");
    // Checked first: nothing was written.
    let device = &load(&client).await.by_scope[&Scope::Device];
    assert!(
        !matches!(
            value_of(device, "http.mcp.enabled"),
            Some(EntryValue::Value(_))
        ),
        "{device:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_unset_writes_nothing() {
    let f = fixture();
    let _bridge = start_bridge(&f).await;
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;

    let id = client.send(Command::ConfigWrite {
        scope: Scope::Device,
        set: vec![set(
            "http.allowed_hosts",
            Value::List(vec![Value::String("notes.example".into())]),
        )],
        unset: vec!["no.such.key".into()],
    });
    let err = outcome(&client, id).await.unwrap_err();
    assert!(err.message.contains("no.such.key"), "{err:?}");
    // The unset was checked before the set was written.
    let device = &load(&client).await.by_scope[&Scope::Device];
    assert!(
        !matches!(
            value_of(device, "http.allowed_hosts"),
            Some(EntryValue::Value(_))
        ),
        "{device:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_load_without_a_bridge_waits_for_one_only_once() {
    let f = fixture();
    // With sync: each relayed call waits up to a second for a bridge to appear.
    let _server = start_server(&f).await;
    // No refresh but the first, which also waits for the bridge: the load is timed alone.
    let mut cfg = config(&f);
    cfg.refresh = Duration::from_secs(3600);
    let client = FrameworkClient::spawn(&tokio::runtime::Handle::current(), cfg, Arc::new(|| {}));
    wait(|| async { client.snapshot().server.up().is_some() }).await;

    let started = Instant::now();
    let state = load(&client).await;
    let took = started.elapsed();
    assert!(state.schema.iter().any(|d| d.key == "http.listen"));
    // Four relayed calls one after another took four seconds and more.
    assert!(took < Duration::from_millis(3500), "{took:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_of_app_keys_works_without_a_bridge() {
    let f = fixture();
    // Without sync, so a relayed call finds no bridge at once rather than after the sync
    // runtime's wait for one.
    let _server = start_relay(&f.endpoints.app, &f.bridge).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().server.up().is_some() }).await;

    let id = client.send(Command::ConfigWrite {
        scope: Scope::Device,
        set: vec![set("update.check", Value::Bool(false))],
        unset: Vec::new(),
    });
    assert!(
        matches!(outcome(&client, id).await, Ok(CommandOutput::Written(_))),
        "the write"
    );
    let state = load(&client).await;
    assert!(
        !state.schema.iter().any(|d| d.key == "net.discovery"),
        "the bridge's keys are missing"
    );
    assert_eq!(
        value_of(&state.by_scope[&Scope::Device], "update.check"),
        Some(&EntryValue::Value(Value::Bool(false)))
    );
    // Put back: the settings files are shared.
    let id = client.send(Command::ConfigWrite {
        scope: Scope::Device,
        set: Vec::new(),
        unset: vec!["update.check".into()],
    });
    assert!(
        matches!(outcome(&client, id).await, Ok(CommandOutput::Written(_))),
        "the unset"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_with_a_bridge_key_fails_without_a_bridge() {
    let f = fixture();
    // Without sync, so a relayed call finds no bridge at once rather than after the sync
    // runtime's wait for one.
    let _server = start_relay(&f.endpoints.app, &f.bridge).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().server.up().is_some() }).await;

    let id = client.send(Command::ConfigWrite {
        scope: Scope::Device,
        set: vec![
            set("http.listen", Value::String("127.0.0.1:1".into())),
            set("net.discovery", Value::Bool(false)),
        ],
        unset: Vec::new(),
    });
    let err = outcome(&client, id).await.unwrap_err();
    assert_eq!(err.message, "sapphire-bridge is not running");
    let device = &load(&client).await.by_scope[&Scope::Device];
    assert!(
        !matches!(value_of(device, "http.listen"), Some(EntryValue::Value(_))),
        "{device:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_bridge_comes_back() {
    let f = fixture();
    let _bridge = start_restarting_bridge(&f).await;
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;

    let id = client.send(Command::Restart(Process::Bridge));
    assert_eq!(outcome(&client, id).await, Ok(CommandOutput::Done));
    // The relay answered once the new bridge was connected: it answers at once.
    let state = load(&client).await;
    assert!(state.schema.iter().any(|d| d.key == "net.discovery"));
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_app_comes_back_and_the_client_reconnects() {
    let f = fixture();
    let _server = start_restarting_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().server.up().is_some() }).await;
    let pid = client.snapshot().server.up().unwrap().info.pid;

    let id = client.send(Command::Restart(Process::App));
    assert_eq!(outcome(&client, id).await, Ok(CommandOutput::Done));
    let state = load(&client).await;
    assert!(state.schema.iter().any(|d| d.key == "http.listen"));
    wait(|| async { client.snapshot().server.up().is_some() }).await;
    assert_eq!(
        client.snapshot().server.up().unwrap().info.pid,
        pid,
        "restarted in place"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_app_of_a_server_that_cannot_says_so() {
    let f = fixture();
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().server.up().is_some() }).await;

    let id = client.send(Command::Restart(Process::App));
    let err = outcome(&client, id).await.unwrap_err();
    assert!(!err.message.is_empty(), "{err:?}");
    // The refusal was an answer: the server stays up.
    let _ = load(&client).await;
}

/// The snapshot's theme value of `key`, once the bridge is up.
fn gui_value(client: &FrameworkClient, key: &str) -> Option<Value> {
    let snapshot = client.snapshot();
    snapshot.bridge.up()?.gui.as_ref()?.get(key).cloned()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_theme_reaches_the_snapshot() {
    let f = fixture();
    let _bridge = start_bridge(&f).await;
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;

    let id = client.send(Command::ConfigWrite {
        scope: Scope::Workgroup,
        set: vec![set("gui.theme.contrast", Value::String("high".into()))],
        unset: Vec::new(),
    });
    assert!(matches!(
        outcome(&client, id).await,
        Ok(CommandOutput::Written(_))
    ));
    wait(|| async {
        gui_value(&client, "gui.theme.contrast") == Some(Value::String("high".into()))
    })
    .await;
    assert_eq!(
        gui_value(&client, "gui.theme.accent_hue"),
        Some(Value::Int(255))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_override_is_what_the_snapshot_shows() {
    let f = fixture();
    let _bridge = start_bridge(&f).await;
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().bridge.up().is_some() }).await;

    for (scope, value) in [(Scope::Workgroup, "high"), (Scope::Device, "low")] {
        let id = client.send(Command::ConfigWrite {
            scope,
            set: vec![set("gui.theme.contrast", Value::String(value.into()))],
            unset: Vec::new(),
        });
        assert!(matches!(
            outcome(&client, id).await,
            Ok(CommandOutput::Written(_))
        ));
    }
    wait(|| async {
        gui_value(&client, "gui.theme.contrast") == Some(Value::String("low".into()))
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_bridge_leaves_the_theme_unread_and_the_server_up() {
    let f = fixture();
    let _server = start_server(&f).await;
    let client = spawn(&f);
    wait(|| async { client.snapshot().fetched }).await;
    let snapshot = client.snapshot();
    assert!(snapshot.server.up().is_some());
    assert!(snapshot.bridge.up().is_none());
}
