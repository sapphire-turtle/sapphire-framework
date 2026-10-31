//! The HTTP layer's default authentication (#364): the served workspace's API keys, with
//! no bridge running anywhere.
//!
//! Each test owns its `AppContext` (the directories are first-writer-wins per context).

use std::sync::Arc;
use std::time::Duration;

use axum::routing::get;
use sapphire_backend::protocol as proto;
use sapphire_framework_keys::open_store;
use sapphire_framework_server::{
    AppServer, AppSettings, HttpEndpoint, Selection, SelectionFile, SettingDef,
};
use sapphire_ipc::{ClientInfo, Endpoint, connect_or_absent};
use sapphire_workspace::{AppContext, AppKind};
use serde_json::{Value, json};

static CTX: AppContext = AppContext::new("sapphire-apikeytest-served");

/// Declares the endpoints `on` and `off`.
struct Declared;

impl AppSettings for Declared {
    fn defs(&self) -> Vec<SettingDef> {
        Vec::new()
    }

    fn http_endpoints(&self) -> Vec<HttpEndpoint> {
        vec![
            HttpEndpoint::new("on", "An endpoint config.toml turns on"),
            HttpEndpoint::new("off", "An endpoint left off"),
        ]
    }
}

/// The environment lock, held for a test's life: every test in this binary points its
/// context's directories through the process environment.
struct EnvLock(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);

/// Point `ctx`'s directories at `tmp`, serialised on the environment with every other test
/// in this binary.
fn point_at(ctx: &'static AppContext, tmp: &std::path::Path) -> EnvLock {
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let upper = ctx
        .app_name
        .trim_start_matches("sapphire-")
        .replace('-', "_")
        .to_uppercase();
    for cat in ["CACHE", "DATA", "CONFIG"] {
        // SAFETY: serialised by the lock above, which the caller holds for the test.
        unsafe {
            std::env::set_var(
                format!("SAPPHIRE_{upper}_{cat}_DIR"),
                tmp.join(cat.to_lowercase()),
            )
        };
    }
    ctx.init(AppKind::Server);
    EnvLock(lock)
}

/// A port nothing listens on, for the moment.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The routes each endpoint serves: `/ping` answers `pong`.
fn ping() -> axum::Router {
    axum::Router::new().route("/ping", get(|| async { "pong" }))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_key_is_accepted_without_a_bridge() {
    let tmp = tempfile::tempdir().unwrap();
    let _env = point_at(&CTX, tmp.path());
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let config_dir = CTX.try_config_dir().unwrap().to_path_buf();
    std::fs::create_dir_all(&config_dir).unwrap();
    let port = free_port();
    std::fs::write(
        config_dir.join("config.toml"),
        format!("[http]\nlisten = \"127.0.0.1:{port}\"\n\n[http.on]\nenabled = true\n"),
    )
    .unwrap();
    let root = tmp.path().join("ws");
    std::fs::create_dir_all(root.join(format!(".{}", CTX.app_name))).unwrap();
    let root = root.canonicalize().unwrap();
    SelectionFile::for_app(&CTX)
        .save(&Selection {
            root: root.clone(),
            sync: false,
        })
        .unwrap();
    let (_, token) = open_store(CTX.app_name, &root)
        .unwrap()
        .add("pendant", None, None)
        .unwrap();
    // An external device from before #364, in the bridge's workgroup ledger: the server
    // imports it at start, so its token keeps working.
    let bridge_dir = tmp.path().join("bridge");
    // SAFETY: serialised by `point_at`'s lock, held for the test.
    unsafe { std::env::set_var("SAPPHIRE_BRIDGE_DIR", &bridge_dir) };
    #[allow(deprecated)]
    let (_, old_token) = sapphire_registry::ExternalDevices::open(
        &bridge_dir
            .join("workgroups")
            .join(sapphire_registry::GrainId::random().to_string())
            .join("root")
            .join("external_devices"),
    )
    .unwrap()
    .add("old pendant", None, vec![CTX.app_name.to_owned()])
    .unwrap();
    let endpoint = Endpoint::in_dir(CTX.app_name, tmp.path().join("run"));

    // No `http_auth`: the default is the workspace's keys.
    let server = AppServer::new(&CTX, "0.0.0")
        .app_settings(Arc::new(Declared))
        .unwrap()
        .http_endpoint("on", ping())
        .endpoint(endpoint.clone());
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_err()
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "HTTP never listened"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let http = reqwest::Client::new();
    let get = |token: &str| {
        http.get(format!("http://127.0.0.1:{port}/on/ping"))
            .bearer_auth(token)
            .send()
    };
    let ok = get(token.expose()).await.unwrap();
    assert_eq!(ok.status(), reqwest::StatusCode::OK);
    assert_eq!(ok.text().await.unwrap(), "pong");
    let imported = get(old_token.expose()).await.unwrap();
    assert_eq!(
        imported.status(),
        reqwest::StatusCode::OK,
        "the imported key"
    );
    let refused = get("sapphire-ak-wrong").await.unwrap();
    assert_eq!(refused.status(), reqwest::StatusCode::UNAUTHORIZED);

    let info = ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    let (c, _) = connect_or_absent(&endpoint, CTX.app_name, info)
        .await
        .unwrap()
        .unwrap();
    let _: Value = c
        .call(sapphire_ipc::SHUTDOWN_METHOD, json!({}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the server stopped")
        .unwrap();
}
