//! The server's settings at start (#314): `server.toml` becomes `config.toml`, and a file
//! the server cannot take stops it from starting, saying why.
//!
//! Each test owns its `AppContext` (the directories are first-writer-wins per context).

use std::path::Path;
use std::time::Duration;

use sapphire_backend::protocol as proto;
use sapphire_framework_server::AppServer;
use sapphire_ipc::{ClientInfo, Endpoint, connect_or_absent};
use sapphire_workspace::AppContext;

static CTX_MIGRATE: AppContext = AppContext::new("sapphire-settings-migrate");
static CTX_MCP: AppContext = AppContext::new("sapphire-settings-mcp");
static CTX_SHARED: AppContext = AppContext::new("sapphire-settings-shared");

/// Point `ctx`'s directories at `tmp`, and give it an endpoint there.
fn point_at(ctx: &'static AppContext, tmp: &Path) -> Endpoint {
    ctx.set_cache_dir(tmp.join("cache"));
    ctx.set_data_dir(tmp.join("data"));
    ctx.set_config_dir(tmp.join("config"));
    std::fs::create_dir_all(tmp.join("config")).unwrap();
    std::fs::create_dir_all(tmp.join("run")).unwrap();
    Endpoint::in_dir(ctx.app_name, tmp.join("run"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_starting_server_moves_server_toml_to_config_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let endpoint = point_at(&CTX_MIGRATE, tmp.path());
    let config = tmp.path().join("config");
    let text = "# kept as written\n[update]\ncheck = false\n";
    std::fs::write(config.join("server.toml"), text).unwrap();

    let server = AppServer::new(&CTX_MIGRATE, "0.0.0").endpoint(endpoint.clone());
    let task = tokio::spawn(async move { server.run().await });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !sapphire_ipc::probe(&endpoint).await.unwrap() {
        assert!(!task.is_finished(), "the server stopped: {:?}", task.await);
        assert!(
            tokio::time::Instant::now() < deadline,
            "the server never started listening"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // The socket is bound before the settings are read, so wait for the handshake —
    // answered only once the server serves — before looking at the files.
    let info = ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    let (client, _) = connect_or_absent(&endpoint, CTX_MIGRATE.app_name, info)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(config.join("config.toml")).unwrap(),
        text
    );
    assert_eq!(
        std::fs::read_to_string(config.join("server.toml.v1")).unwrap(),
        text
    );
    assert!(!config.join("server.toml").exists());

    let _: serde_json::Value = client
        .call(sapphire_ipc::SHUTDOWN_METHOD, serde_json::json!({}))
        .await
        .unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_old_mcp_table_stops_the_server_and_says_where_it_went() {
    let tmp = tempfile::tempdir().unwrap();
    let endpoint = point_at(&CTX_MCP, tmp.path());
    let config = tmp.path().join("config");
    std::fs::write(config.join("server.toml"), "[mcp]\nhttp = true\n").unwrap();

    let err = AppServer::new(&CTX_MCP, "0.0.0")
        .endpoint(endpoint)
        .run()
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("[mcp] http moved to [http.mcp] enabled"),
        "{err}"
    );
    assert!(
        err.contains(&config.join("config.toml").display().to_string()),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_starts_with_the_apps_own_tables_in_config_toml() {
    let tmp = tempfile::tempdir().unwrap();
    let endpoint = point_at(&CTX_SHARED, tmp.path());
    let config = tmp.path().join("config");
    let text = "sync_interval_minutes = 5\n\n[anthropic]\napi_key = \"x\"\n\n\
                [workspace]\ndefault = \"a\"\n\n[update]\ncheck = false\n";
    std::fs::write(config.join("config.toml"), text).unwrap();

    let server = AppServer::new(&CTX_SHARED, "0.0.0").endpoint(endpoint.clone());
    let task = tokio::spawn(async move { server.run().await });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !sapphire_ipc::probe(&endpoint).await.unwrap() {
        assert!(!task.is_finished(), "the server stopped: {:?}", task.await);
        assert!(
            tokio::time::Instant::now() < deadline,
            "the server never started listening"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        std::fs::read_to_string(config.join("config.toml")).unwrap(),
        text
    );

    let info = ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    let (client, _) = connect_or_absent(&endpoint, CTX_SHARED.app_name, info)
        .await
        .unwrap()
        .unwrap();
    let _: serde_json::Value = client
        .call(sapphire_ipc::SHUTDOWN_METHOD, serde_json::json!({}))
        .await
        .unwrap();
    task.await.unwrap().unwrap();
}
