//! What the relay logs (#314): the method and the outcome, never the params.
//!
//! Its own test binary, because it installs the process's global subscriber.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sapphire_framework_server::sync::testing::StubBridge;
use sapphire_framework_server::{AppServer, BridgeConnector};
use sapphire_ipc::{ClientInfo, Endpoint, connect_or_absent};
use sapphire_workspace::{AppContext, AppKind};
use serde_json::{Value, json};

static CTX: AppContext = AppContext::new("sapphire-relaylog");

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

#[tokio::test(flavor = "multi_thread")]
async fn relayed_params_are_never_logged() {
    let captured = Captured::default();
    // Installed before the context's own, which then gives way to it.
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(captured.clone())
            .finish(),
    )
    .unwrap();

    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary; nothing else reads the environment meanwhile.
    for (name, cat) in [
        ("SAPPHIRE_RELAYLOG_CACHE_DIR", "cache"),
        ("SAPPHIRE_RELAYLOG_DATA_DIR", "data"),
        ("SAPPHIRE_RELAYLOG_CONFIG_DIR", "config"),
    ] {
        unsafe { std::env::set_var(name, tmp.path().join(cat)) };
    }
    CTX.init(AppKind::Server);

    let (stub, client) = StubBridge::start_client_in(std::env::temp_dir()).await;
    stub.answer("embed.key_set", json!({"ok": true}));
    let client = Mutex::new(Some(client));
    let connector = BridgeConnector::new(move || {
        let next = client.lock().unwrap().take();
        async move { Ok(next) }
    });

    let endpoint = Endpoint::in_dir("sapphire-relaylog", tmp.path().to_path_buf());
    let server = tokio::spawn(
        AppServer::new(&CTX, "0.0.0")
            .endpoint(endpoint.clone())
            .bridge_connector(connector)
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
    let (app, _) = connect_or_absent(&endpoint, "sapphire-relaylog", info)
        .await
        .unwrap()
        .expect("the server is listening");

    let answer: Value = app
        .call("embed.key_set", json!({"key": "sk-SECRET-123"}))
        .await
        .unwrap();
    assert_eq!(answer, json!({"ok": true}));
    assert_eq!(
        stub.calls(),
        vec![("embed.key_set".to_owned(), json!({"key": "sk-SECRET-123"}))]
    );
    server.abort();

    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("embed.key_set"), "the method is logged: {log}");
    assert!(!log.contains("sk-SECRET-123"), "the key leaked: {log}");
}
