//! `config.*` on the app server (#314): its own settings merged with the bridge's.
//!
//! The app server answers the five methods itself: its keys from its own files, the
//! bridge's from the bridge, and a batch of both checked on both sides before either side
//! writes. Each test owns its `AppContext` (the directories are first-writer-wins per
//! context), and its application's keys are named after it.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sapphire_bridge_api::settings::{Entry, EntryValue, Origin};
use sapphire_bridge_api::{
    CONFIG_LIST, CONFIG_SCHEMA, CONFIG_SET, CONFIG_UNSET, ConfigEntry, ConfigFieldError,
    ConfigFieldErrors, ConfigListResult, ConfigSchemaResult, ConfigWriteResult,
};
use sapphire_bridge_client::{BridgeClient, ConfigFieldErrorsExt};
use sapphire_framework_server::sync::testing::StubBridge;
use sapphire_framework_server::{
    AppServer, AppSettings, Apply, BridgeConnector, Kind, Process, Scope, Selection, SelectionFile,
    SettingDef, Settings, Value,
};
use sapphire_ipc::{ClientInfo, Endpoint, Error, RpcError, connect_or_absent};
use sapphire_workspace::AppContext;
use serde_json::json;

/// The application's settings: a note on the device or in the workspace, a level that
/// applies on a restart, and a secret.
struct TestApp(&'static str);

impl AppSettings for TestApp {
    fn defs(&self) -> Vec<SettingDef> {
        let def = |name: &str, kind: Kind, scopes: Vec<Scope>, apply: Apply| SettingDef {
            key: format!("{}.{name}", self.0),
            kind,
            scopes,
            overridable: false,
            apply,
            default: None,
            section: "Test".into(),
            doc: "test".into(),
            process: Process::Bridge,
        };
        vec![
            def(
                "note",
                Kind::String { non_empty: false },
                vec![Scope::Device, Scope::Workspace],
                Apply::Live,
            ),
            def(
                "level",
                Kind::Int { min: 0, max: 10 },
                vec![Scope::Device],
                Apply::Restart,
            ),
            def("token", Kind::Secret, vec![Scope::Device], Apply::Live),
        ]
    }
}

/// The bridge's one setting, as the stub defines it.
fn bridge_def() -> SettingDef {
    SettingDef {
        key: "net.discovery".into(),
        kind: Kind::Bool,
        scopes: vec![Scope::Device, Scope::Workgroup],
        overridable: true,
        apply: Apply::Restart,
        default: Some(Value::Bool(true)),
        section: "Network".into(),
        doc: "bridge".into(),
        process: Process::Bridge,
    }
}

/// The bridge's one setting, as the stub lists it.
fn bridge_entry() -> Entry {
    Entry {
        key: "net.discovery".into(),
        value: EntryValue::Value(Value::Bool(true)),
        origin: Origin::Default,
        pending_restart: false,
        note: None,
    }
}

/// A stub answering `config.*` as a bridge with `net.discovery` does, every write passing.
async fn stub() -> (StubBridge, BridgeClient) {
    let (stub, client) = StubBridge::start_client_in(std::env::temp_dir()).await;
    stub.answer(
        CONFIG_SCHEMA,
        serde_json::to_value(ConfigSchemaResult {
            settings: vec![bridge_def()],
        })
        .unwrap(),
    );
    stub.answer(
        CONFIG_LIST,
        serde_json::to_value(ConfigListResult {
            entries: vec![bridge_entry()],
        })
        .unwrap(),
    );
    stub.respond(CONFIG_SET, |params| {
        Ok(json!({
            "changed": params["entries"].as_array().unwrap().iter().map(|e| e["key"].clone()).collect::<Vec<_>>(),
            "pending_restart": [],
        }))
    });
    (stub, client)
}

/// A server for one test, and a client of its `config.*`.
struct Fixture {
    tmp: tempfile::TempDir,
    app: &'static str,
    config: BridgeClient,
    settings: Settings,
    server: tokio::task::JoinHandle<sapphire_framework_server::Result<()>>,
}

impl Fixture {
    /// Serve `ctx`'s application over `bridge`, or with no bridge running.
    async fn start(ctx: &'static AppContext, bridge: Option<BridgeClient>) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        ctx.set_cache_dir(tmp.path().join("cache"));
        ctx.set_data_dir(tmp.path().join("data"));
        ctx.set_config_dir(tmp.path().join("config"));
        std::fs::create_dir_all(tmp.path().join("config")).unwrap();
        std::fs::create_dir_all(tmp.path().join("run")).unwrap();
        let endpoint = Endpoint::in_dir(ctx.app_name, tmp.path().join("run"));

        let bridge = Mutex::new(bridge);
        let connector = BridgeConnector::new(move || {
            let next = bridge.lock().unwrap().take();
            async move { Ok(next) }
        });
        let server = AppServer::new(ctx, "0.0.0")
            .endpoint(endpoint.clone())
            .selection_file(tmp.path().join("selection.toml"))
            .bridge_connector(connector)
            .app_settings(Arc::new(TestApp(ctx.app_name)))
            .unwrap();
        let settings = server.settings().expect("read").clone();
        let server = tokio::spawn(server.run());

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !sapphire_ipc::probe(&endpoint).await.unwrap() {
            assert!(!server.is_finished(), "the server stopped");
            assert!(
                tokio::time::Instant::now() < deadline,
                "the server never started listening"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let info = ClientInfo {
            kind: "test".into(),
            version: "0.0.0".into(),
            api: sapphire_backend::protocol::API_VERSION,
            app_api: None,
            pid: std::process::id(),
        };
        let (client, _) = connect_or_absent(&endpoint, ctx.app_name, info)
            .await
            .unwrap()
            .expect("the server is listening");
        Fixture {
            config: BridgeClient::relayed(Arc::new(client), tmp.path().join("run")),
            tmp,
            app: ctx.app_name,
            settings,
            server,
        }
    }

    /// The application's key `name`.
    fn key(&self, name: &str) -> String {
        format!("{}.{name}", self.app)
    }

    /// The device's settings file, as it is now; empty when there is none.
    fn device_file(&self) -> String {
        std::fs::read_to_string(self.tmp.path().join("config").join("config.toml"))
            .unwrap_or_default()
    }

    /// Select `root` as the server's workspace.
    fn select(&self, root: &Path) {
        std::fs::create_dir_all(root).unwrap();
        SelectionFile::at(self.tmp.path().join("selection.toml"))
            .save(&Selection {
                root: root.to_path_buf(),
                sync: false,
            })
            .unwrap();
    }

    /// Set `entries` in their home scopes.
    async fn set(&self, entries: &[(&str, Value)]) -> sapphire_ipc::Result<ConfigWriteResult> {
        self.config
            .config_set(
                None,
                entries
                    .iter()
                    .map(|(key, value)| ConfigEntry {
                        key: (*key).to_owned(),
                        value: value.clone(),
                    })
                    .collect(),
            )
            .await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// The JSON-RPC error a call failed with.
fn rpc<T: std::fmt::Debug>(result: sapphire_ipc::Result<T>) -> RpcError {
    match result {
        Err(Error::Rpc(e)) => e,
        other => panic!("expected an RPC error, got {other:?}"),
    }
}

/// The `config.set` calls the stub saw, as their `dry_run` flags, in order.
fn bridge_sets(stub: &StubBridge) -> Vec<bool> {
    stub.calls()
        .into_iter()
        .filter(|(method, _)| method == CONFIG_SET)
        .map(|(_, params)| params["dry_run"].as_bool().unwrap_or(false))
        .collect()
}

fn keys(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|e| e.key.as_str()).collect()
}

static CTX_SCHEMA: AppContext = AppContext::new("cfgschema");

#[tokio::test(flavor = "multi_thread")]
async fn the_schema_merges_both_sides() {
    let (_stub, bridge) = stub().await;
    let f = Fixture::start(&CTX_SCHEMA, Some(bridge)).await;

    let schema = f.config.config_schema().await.unwrap().settings;
    let shown: Vec<(&str, Process)> = schema.iter().map(|d| (d.key.as_str(), d.process)).collect();
    assert_eq!(
        shown,
        vec![
            ("http.listen", Process::App),
            ("http.allowed_hosts", Process::App),
            ("http.mcp.enabled", Process::App),
            ("http.metrics.enabled", Process::App),
            ("update.check", Process::App),
            ("cfgschema.note", Process::App),
            ("cfgschema.level", Process::App),
            ("cfgschema.token", Process::App),
            ("net.discovery", Process::Bridge),
        ]
    );
}

static CTX_LIST: AppContext = AppContext::new("cfglist");

#[tokio::test(flavor = "multi_thread")]
async fn list_filters_by_scope() {
    let (stub, bridge) = stub().await;
    let f = Fixture::start(&CTX_LIST, Some(bridge)).await;
    let root = f.tmp.path().join("workspace");
    f.select(&root);
    f.config
        .config_set(
            Some(Scope::Workspace),
            vec![ConfigEntry {
                key: f.key("note"),
                value: Value::String("in the workspace".into()),
            }],
        )
        .await
        .unwrap();
    // The selected workspace's file, as the server's own selection file names it.
    let text = std::fs::read_to_string(root.join(".cfglist").join("config.toml")).unwrap();
    assert!(text.contains("in the workspace"), "{text}");

    let all = f.config.config_list(None).await.unwrap().entries;
    let all = keys(&all);
    assert_eq!(all.first(), Some(&"http.listen"), "own keys first: {all:?}");
    assert_eq!(all.last(), Some(&"net.discovery"), "{all:?}");
    assert!(all.contains(&"cfglist.note"), "{all:?}");

    let workgroup = f
        .config
        .config_list(Some(Scope::Workgroup))
        .await
        .unwrap()
        .entries;
    assert_eq!(
        keys(&workgroup),
        vec!["net.discovery"],
        "the bridge's alone"
    );

    let before = stub.calls().len();
    let workspace = f
        .config
        .config_list(Some(Scope::Workspace))
        .await
        .unwrap()
        .entries;
    assert_eq!(
        keys(&workspace),
        vec!["cfglist.note"],
        "the app server's alone"
    );
    assert_eq!(
        workspace[0].value,
        EntryValue::Value(Value::String("in the workspace".into()))
    );
    assert_eq!(stub.calls().len(), before, "the bridge was not asked");
}

static CTX_BAD_BRIDGE: AppContext = AppContext::new("cfgbadbridge");

#[tokio::test(flavor = "multi_thread")]
async fn a_mixed_batch_with_a_bad_bridge_value_writes_nothing() {
    let (stub, bridge) = stub().await;
    stub.respond(CONFIG_SET, |_| {
        Err(RpcError::internal("invalid value for `net.discovery`"))
    });
    let f = Fixture::start(&CTX_BAD_BRIDGE, Some(bridge)).await;

    let e = rpc(f
        .set(&[
            (&f.key("note"), Value::String("x".into())),
            ("net.discovery", Value::Bool(false)),
        ])
        .await);
    assert!(e.message.contains("net.discovery"), "{}", e.message);
    assert_eq!(
        bridge_sets(&stub),
        vec![true],
        "checked only, never written"
    );
    assert!(!f.device_file().contains("note"), "{}", f.device_file());
}

static CTX_BAD_APP: AppContext = AppContext::new("cfgbadapp");

#[tokio::test(flavor = "multi_thread")]
async fn a_mixed_batch_with_a_bad_app_value_never_reaches_the_bridge() {
    let (stub, bridge) = stub().await;
    let f = Fixture::start(&CTX_BAD_APP, Some(bridge)).await;

    let e = rpc(f
        .set(&[
            (&f.key("level"), Value::Int(99)),
            ("net.discovery", Value::Bool(false)),
        ])
        .await);
    assert!(e.message.contains("cfgbadapp.level"), "{}", e.message);
    assert!(bridge_sets(&stub).is_empty(), "{:?}", stub.calls());
    assert!(!f.device_file().contains("level"), "{}", f.device_file());
}

static CTX_ORDER: AppContext = AppContext::new("cfgorder");

#[tokio::test(flavor = "multi_thread")]
async fn a_mixed_batch_writes_bridge_then_app() {
    let (stub, bridge) = stub().await;
    // The first write fails, after its check passed: the app's part must not be written.
    let writes = Arc::new(AtomicUsize::new(0));
    stub.respond(CONFIG_SET, {
        let writes = Arc::clone(&writes);
        move |params| {
            if params["dry_run"].as_bool().unwrap_or(false) {
                return Ok(json!({"changed": ["net.discovery"], "pending_restart": []}));
            }
            if writes.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(RpcError::internal("the disk is full"));
            }
            Ok(json!({"changed": ["net.discovery"], "pending_restart": ["net.discovery"]}))
        }
    });
    let f = Fixture::start(&CTX_ORDER, Some(bridge)).await;
    let batch = [
        (f.key("level"), Value::Int(3)),
        ("net.discovery".to_owned(), Value::Bool(false)),
    ];
    let batch: Vec<(&str, Value)> = batch.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();

    let e = rpc(f.set(&batch).await);
    assert!(e.message.contains("the disk is full"), "{}", e.message);
    assert!(!f.device_file().contains("level"), "{}", f.device_file());

    let result = f.set(&batch).await.unwrap();
    assert_eq!(bridge_sets(&stub), vec![true, false, true, false]);
    assert!(f.device_file().contains("level = 3"), "{}", f.device_file());
    let mut changed = result.changed.clone();
    changed.sort();
    assert_eq!(changed, vec!["cfgorder.level", "net.discovery"]);
    let mut pending = result.pending_restart.clone();
    pending.sort();
    assert_eq!(pending, vec!["cfgorder.level", "net.discovery"]);
}

static CTX_ALONE: AppContext = AppContext::new("cfgalone");

#[tokio::test(flavor = "multi_thread")]
async fn without_a_bridge_app_keys_still_read_and_write() {
    let f = Fixture::start(&CTX_ALONE, None).await;

    let schema = f.config.config_schema().await.unwrap().settings;
    assert_eq!(schema.len(), 8, "the app server's own");
    assert!(schema.iter().all(|d| d.process == Process::App));

    f.set(&[(&f.key("note"), Value::String("hello".into()))])
        .await
        .unwrap();
    let entry = f.config.config_get(&f.key("note")).await.unwrap();
    assert_eq!(
        entry.value,
        EntryValue::Value(Value::String("hello".into()))
    );
    let listed = f.config.config_list(None).await.unwrap().entries;
    assert!(keys(&listed).contains(&"cfgalone.note"));
    assert!(!keys(&listed).contains(&"net.discovery"));

    f.config
        .config_unset(None, vec![f.key("note")])
        .await
        .unwrap();
    let entry = f.config.config_get(&f.key("note")).await.unwrap();
    assert_eq!(entry.value, EntryValue::Unset);

    // A key the app server does not know is the bridge's to answer.
    let e = f.config.config_get("net.discovery").await.unwrap_err();
    assert!(matches!(e, Error::NotRunning(_)), "{e:?}");
}

static CTX_ALONE_BATCH: AppContext = AppContext::new("cfgalonebatch");

#[tokio::test(flavor = "multi_thread")]
async fn without_a_bridge_a_batch_with_a_bridge_key_fails() {
    let f = Fixture::start(&CTX_ALONE_BATCH, None).await;

    let e = f
        .set(&[
            (&f.key("note"), Value::String("x".into())),
            ("net.discovery", Value::Bool(false)),
        ])
        .await
        .unwrap_err();
    assert!(matches!(e, Error::NotRunning(_)), "{e:?}");
    assert!(!f.device_file().contains("note"), "{}", f.device_file());

    let e = rpc(f.set(&[("nope.x", Value::Bool(true))]).await);
    assert_eq!(
        e.message,
        "unknown setting nope.x (or sapphire-bridge, which may know it, is not running)"
    );
}

static CTX_ALONE_GUI: AppContext = AppContext::new("cfgalonegui");

#[tokio::test(flavor = "multi_thread")]
async fn without_a_bridge_a_gui_key_needs_the_bridge() {
    let f = Fixture::start(&CTX_ALONE_GUI, None).await;
    let e = f
        .set(&[("gui.theme.contrast", Value::String("high".into()))])
        .await
        .unwrap_err();
    assert!(matches!(e, Error::NotRunning(_)), "{e:?}");
}

static CTX_LIVE: AppContext = AppContext::new("cfglive");

#[tokio::test(flavor = "multi_thread")]
async fn a_live_write_updates_settings() {
    let f = Fixture::start(&CTX_LIVE, None).await;
    let mut changed = f.settings.changed();

    let result = f
        .set(&[(&f.key("note"), Value::String("now".into()))])
        .await
        .unwrap();
    assert!(result.pending_restart.is_empty(), "{result:?}");
    assert!(changed.has_changed().unwrap());
    changed.mark_unchanged();
    assert_eq!(
        f.settings.get_as::<String>("cfglive.note").as_deref(),
        Some("now")
    );

    let result = f.set(&[(&f.key("level"), Value::Int(4))]).await.unwrap();
    assert_eq!(result.pending_restart, vec!["cfglive.level"]);
    assert!(
        !changed.has_changed().unwrap(),
        "a restart key announces nothing"
    );
    assert_eq!(
        f.settings.get("cfglive.level"),
        None,
        "kept until the restart"
    );
    let entry = f.config.config_get("cfglive.level").await.unwrap();
    assert_eq!(entry.value, EntryValue::Value(Value::Int(4)));
    assert!(entry.pending_restart);
}

static CTX_RACE: AppContext = AppContext::new("cfgrace");

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_writes_both_land() {
    let f = Fixture::start(&CTX_RACE, None).await;
    let (note, level) = (f.key("note"), f.key("level"));

    for round in 0..20 {
        let a = [(note.as_str(), Value::String(format!("round {round}")))];
        let b = [(level.as_str(), Value::Int(round % 10))];
        let (a, b) = tokio::join!(f.set(&a), f.set(&b));
        a.unwrap();
        b.unwrap();
        let text = f.device_file();
        assert!(text.contains(&format!("\"round {round}\"")), "{text}");
        assert!(text.contains(&format!("level = {}", round % 10)), "{text}");
    }
}

static CTX_UNSET: AppContext = AppContext::new("cfgunset");

#[tokio::test(flavor = "multi_thread")]
async fn a_mixed_unset_is_split_by_owner() {
    let (stub, bridge) = stub().await;
    stub.respond(CONFIG_UNSET, |params| {
        Ok(json!({"changed": params["keys"].clone(), "pending_restart": []}))
    });
    let f = Fixture::start(&CTX_UNSET, Some(bridge)).await;
    f.set(&[(&f.key("note"), Value::String("x".into()))])
        .await
        .unwrap();

    let result = f
        .config
        .config_unset(None, vec![f.key("note"), "net.discovery".to_owned()])
        .await
        .unwrap();
    let mut changed = result.changed.clone();
    changed.sort();
    assert_eq!(changed, vec!["cfgunset.note", "net.discovery"]);
    let unsets: Vec<serde_json::Value> = stub
        .calls()
        .into_iter()
        .filter(|(m, _)| m == CONFIG_UNSET)
        .map(|(_, p)| p)
        .collect();
    assert_eq!(unsets.len(), 2, "checked, then written");
    for params in &unsets {
        assert_eq!(
            params["keys"],
            json!(["net.discovery"]),
            "the bridge's part alone"
        );
    }
    assert!(!f.device_file().contains("note"), "{}", f.device_file());
}

static CTX_FIELDS: AppContext = AppContext::new("cfgfields");

#[tokio::test(flavor = "multi_thread")]
async fn an_app_key_error_carries_its_field_errors() {
    let f = Fixture::start(&CTX_FIELDS, None).await;

    for dry_run in [false, true] {
        let result = if dry_run {
            f.config
                .config_check_set(
                    None,
                    vec![ConfigEntry {
                        key: f.key("level"),
                        value: Value::Int(99),
                    }],
                )
                .await
        } else {
            f.set(&[(&f.key("level"), Value::Int(99))]).await
        };
        let fields = ConfigFieldErrors::from_rpc(&rpc(result)).expect("field errors");
        assert_eq!(fields.fields.len(), 1, "{fields:?}");
        assert_eq!(fields.fields[0].key, f.key("level"));
    }
}

static CTX_FIELDS_PASS: AppContext = AppContext::new("cfgfieldspass");

#[tokio::test(flavor = "multi_thread")]
async fn a_bridge_key_error_passes_its_field_errors_through() {
    let (stub, bridge) = stub().await;
    let data = ConfigFieldErrors {
        fields: vec![ConfigFieldError {
            key: "net.discovery".into(),
            message: "expected a bool".into(),
        }],
    };
    let sent = data.to_data();
    stub.respond(CONFIG_SET, move |_| {
        Err(RpcError {
            data: Some(sent.clone()),
            ..RpcError::internal("invalid value for `net.discovery`")
        })
    });
    let f = Fixture::start(&CTX_FIELDS_PASS, Some(bridge)).await;

    let e = rpc(f.set(&[("net.discovery", Value::Bool(false))]).await);
    assert_eq!(ConfigFieldErrors::from_rpc(&e), Some(data));
}
