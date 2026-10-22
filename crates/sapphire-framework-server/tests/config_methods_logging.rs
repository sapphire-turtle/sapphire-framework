//! What the app server's `config.*` logs (#314): the method and the outcome, never a
//! setting's value.
//!
//! Its own test binary, because it installs the process's global subscriber.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sapphire_bridge_api::ConfigEntry;
use sapphire_bridge_client::BridgeClient;
use sapphire_framework_server::{
    AppServer, AppSettings, Apply, BridgeConnector, Kind, Process, Scope, SettingDef, Value,
};
use sapphire_ipc::{ClientInfo, Endpoint, connect_or_absent};
use sapphire_workspace::AppContext;

static CTX: AppContext = AppContext::new("cfglog");

/// Every log line, captured.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Captured {
        self.clone()
    }
}

/// An application with one secret.
struct TestApp;

impl AppSettings for TestApp {
    fn defs(&self) -> Vec<SettingDef> {
        vec![SettingDef {
            key: "cfglog.token".into(),
            kind: Kind::Secret,
            scopes: vec![Scope::Device],
            overridable: false,
            apply: Apply::Live,
            default: None,
            section: "Test".into(),
            doc: "test".into(),
            process: Process::App,
        }]
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn setting_values_are_never_logged() {
    let captured = Captured::default();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(captured.clone())
            .finish(),
    )
    .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    CTX.set_cache_dir(tmp.path().join("cache"));
    CTX.set_data_dir(tmp.path().join("data"));
    CTX.set_config_dir(tmp.path().join("config"));
    std::fs::create_dir_all(tmp.path().join("config")).unwrap();
    let endpoint = Endpoint::in_dir("cfglog", tmp.path().to_path_buf());
    let server = tokio::spawn(
        AppServer::new(&CTX, "0.0.0")
            .endpoint(endpoint.clone())
            .bridge_connector(BridgeConnector::new(|| async { Ok(None) }))
            .app_settings(Arc::new(TestApp))
            .unwrap()
            .run(),
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !sapphire_ipc::probe(&endpoint).await.unwrap() {
        assert!(tokio::time::Instant::now() < deadline, "never listened");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let info = ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: sapphire_backend::protocol::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    let (app, _) = connect_or_absent(&endpoint, "cfglog", info)
        .await
        .unwrap()
        .expect("the server is listening");
    let config = BridgeClient::relayed(Arc::new(app), tmp.path().to_path_buf());

    config
        .config_set(
            None,
            vec![
                ConfigEntry {
                    key: "http.allowed_hosts".into(),
                    value: Value::List(vec![Value::String("hidden-host.example".into())]),
                },
                ConfigEntry {
                    key: "cfglog.token".into(),
                    value: Value::String("sk-SECRET-123".into()),
                },
            ],
        )
        .await
        .unwrap();
    // A refused batch, whose error might have quoted the value.
    config
        .config_set(
            None,
            vec![ConfigEntry {
                key: "http.listen".into(),
                value: Value::String("not-an-address-SECRET".into()),
            }],
        )
        .await
        .unwrap_err();
    config.config_list(None).await.unwrap();
    config.config_get("http.allowed_hosts").await.unwrap();
    server.abort();

    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("config.set"), "the method is logged: {log}");
    for value in [
        "hidden-host.example",
        "sk-SECRET-123",
        "not-an-address-SECRET",
    ] {
        assert!(!log.contains(value), "{value} leaked: {log}");
    }
}
