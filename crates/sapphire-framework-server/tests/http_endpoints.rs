//! The application's switchable HTTP endpoints (`AppServer::http_endpoint`): served at
//! `/<name>` behind the token while `config.toml` turns `http.<name>.enabled` on, and not
//! at all otherwise.
//!
//! Each test owns its `AppContext` (the directories are first-writer-wins per context).

use std::sync::Arc;
use std::time::Duration;

use axum::routing::get;
use sapphire_backend::protocol as proto;
use sapphire_framework_keys::{AuthConfig, Authenticated, KeyId, Verdict, Verifier};
use sapphire_framework_server::{AppServer, AppSettings, HttpEndpoint, SettingDef};
use sapphire_ipc::{ClientInfo, Endpoint, connect_or_absent};
use sapphire_workspace::{AppContext, AppKind};
use serde_json::{Value, json};

static CTX_SERVED: AppContext = AppContext::new("sapphire-endpointtest-served");
static CTX_UNDECLARED: AppContext = AppContext::new("sapphire-endpointtest-undeclared");

/// The one token the fake verifier accepts.
const TOKEN: &str = "sapphire-ak-good";

/// Accepts [`TOKEN`] and nothing else.
struct Fake;

#[async_trait::async_trait]
impl Verifier for Fake {
    async fn verify(&self, token: &str) -> Verdict {
        if token == TOKEN {
            Verdict::Accepted(Authenticated {
                id: KeyId::generate(),
                name: "agent".into(),
            })
        } else {
            Verdict::Refused
        }
    }
}

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
async fn an_endpoint_is_served_while_its_switch_is_on() {
    let tmp = tempfile::tempdir().unwrap();
    let _env = point_at(&CTX_SERVED, tmp.path());
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let config_dir = CTX_SERVED.try_config_dir().unwrap().to_path_buf();
    std::fs::create_dir_all(&config_dir).unwrap();
    let port = free_port();
    std::fs::write(
        config_dir.join("config.toml"),
        format!("[http]\nlisten = \"127.0.0.1:{port}\"\n\n[http.on]\nenabled = true\n"),
    )
    .unwrap();
    let endpoint = Endpoint::in_dir(CTX_SERVED.app_name, tmp.path().join("run"));

    let server = AppServer::new(&CTX_SERVED, "0.0.0")
        .app_settings(Arc::new(Declared))
        .unwrap()
        .http_endpoint("on", ping())
        .http_endpoint("off", ping())
        .http_auth(AuthConfig::new(Arc::new(Fake)))
        .endpoint(endpoint.clone());
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !sapphire_ipc::probe(&endpoint).await.unwrap() {
        assert!(tokio::time::Instant::now() < deadline, "never listened");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // The HTTP listener binds alongside the IPC socket, not before it.
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
    let get = |path: &str, token: Option<&str>| {
        let mut req = http.get(format!("http://127.0.0.1:{port}{path}"));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        req.send()
    };
    let on = get("/on/ping", Some(TOKEN)).await.unwrap();
    assert_eq!(on.status(), reqwest::StatusCode::OK);
    assert_eq!(on.text().await.unwrap(), "pong");
    for token in [None, Some("sapphire-ak-bad")] {
        let refused = get("/on/ping", token).await.unwrap();
        assert_eq!(
            refused.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "{token:?}"
        );
    }
    let off = get("/off/ping", Some(TOKEN)).await.unwrap();
    assert_eq!(off.status(), reqwest::StatusCode::NOT_FOUND);

    let info = ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    let (c, _) = connect_or_absent(&endpoint, CTX_SERVED.app_name, info)
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

#[tokio::test(flavor = "multi_thread")]
async fn an_endpoint_the_settings_do_not_declare_stops_the_server() {
    let tmp = tempfile::tempdir().unwrap();
    let _env = point_at(&CTX_UNDECLARED, tmp.path());
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let endpoint = Endpoint::in_dir(CTX_UNDECLARED.app_name, tmp.path().join("run"));

    let err = AppServer::new(&CTX_UNDECLARED, "0.0.0")
        .app_settings(Arc::new(Declared))
        .unwrap()
        .http_endpoint("acp", ping())
        .endpoint(endpoint)
        .run()
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("`acp`"), "{err}");
}
