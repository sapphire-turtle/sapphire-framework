//! A restart that fails (#314) says so in the log before the server returns.
//!
//! Its own test binary, because [`serve_restarting`] routes the process's one log file.

use std::time::Duration;

use sapphire_backend::protocol as proto;
use sapphire_framework_server::{AppServer, BridgeConnector, Settings, serve_restarting};
use sapphire_ipc::{ClientInfo, Endpoint, connect_or_absent};
use sapphire_workspace::AppContext;
use serde_json::json;

const DEADLINE: Duration = Duration::from_secs(10);

static CTX: AppContext = AppContext::new("rstlog");

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_restart_is_logged() {
    let tmp = tempfile::tempdir().unwrap();
    CTX.set_cache_dir(tmp.path().join("cache"));
    CTX.set_data_dir(tmp.path().join("data"));
    CTX.set_config_dir(tmp.path().join("config"));
    std::fs::create_dir_all(tmp.path().join("config")).unwrap();
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let endpoint = Endpoint::in_dir(CTX.app_name, tmp.path().join("run"));
    let selection = tmp.path().join("selection.toml");

    let server = tokio::spawn(serve_restarting(&CTX, (), {
        let endpoint = endpoint.clone();
        move |_: &Settings| {
            AppServer::new(&CTX, "0.0.0")
                .endpoint(endpoint.clone())
                .selection_file(selection.clone())
                .bridge_connector(BridgeConnector::new(|| async { Ok(None) }))
        }
    }));
    let deadline = tokio::time::Instant::now() + DEADLINE;
    while !sapphire_ipc::probe(&endpoint).await.unwrap() {
        assert!(!server.is_finished(), "the server stopped");
        assert!(tokio::time::Instant::now() < deadline, "never listened");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // The next round cannot read its settings.
    std::fs::write(
        tmp.path().join("config").join("config.toml"),
        "[http]\nlisten = 5\n",
    )
    .unwrap();
    let info = ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    let (client, _) = connect_or_absent(&endpoint, CTX.app_name, info)
        .await
        .unwrap()
        .unwrap();
    let _: proto::Ack = client.call(proto::SERVER_RESTART, json!({})).await.unwrap();
    let err = tokio::time::timeout(DEADLINE, server)
        .await
        .expect("the server gives up")
        .unwrap()
        .unwrap_err();
    assert!(err.to_string().contains("http.listen"), "{err}");

    let log = std::fs::read_to_string(tmp.path().join("data").join("logs").join("app.log"))
        .unwrap_or_default();
    assert!(log.contains("restart failed"), "{log}");
    assert!(log.contains("http.listen"), "{log}");
}
