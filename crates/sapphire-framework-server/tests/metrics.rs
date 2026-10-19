//! `/metrics` on the server's HTTP listener: off unless `server.toml` says so, open to a
//! scraper without a token, and no way around the token for anything else.
//!
//! Each test owns its `AppContext` (the directories are first-writer-wins per context). The
//! recorder is installed once per process, so the tests share it; each looks only for its own
//! application's `sapphire_build_info` line.

use std::sync::Arc;
use std::time::Duration;

use axum::routing::get;
use sapphire_backend::protocol as proto;
use sapphire_framework_keys::{AuthConfig, Authenticated, GrainId, Verdict, Verifier};
use sapphire_framework_server::{AppServer, HttpConfig, HttpMetricsConfig, ServerConfig};
use sapphire_ipc::{Client, ClientInfo, Endpoint, connect_or_absent};
use sapphire_workspace::{AppContext, AppKind};
use serde_json::{Value, json};

static CTX_ALONE: AppContext = AppContext::new("sapphire-metricstest-alone");
static CTX_TOKEN: AppContext = AppContext::new("sapphire-metricstest-token");
static CTX_APP: AppContext = AppContext::new("sapphire-metricstest-app");
static CTX_OFF: AppContext = AppContext::new("sapphire-metricstest-off");

/// The one token the fake verifier accepts.
const TOKEN: &str = "sapphire-ed-good";

/// Accepts [`TOKEN`] and nothing else.
struct Fake;

#[async_trait::async_trait]
impl Verifier for Fake {
    async fn verify(&self, token: &str) -> Verdict {
        if token == TOKEN {
            Verdict::Accepted(Authenticated {
                id: GrainId::random(),
                name: "agent".into(),
            })
        } else {
            Verdict::Refused
        }
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

async fn start(server: AppServer, endpoint: &Endpoint) -> tokio::task::JoinHandle<()> {
    let server = server.endpoint(endpoint.clone());
    let task = tokio::spawn(async move { server.run().await.unwrap() });
    wait_until_listening(endpoint).await;
    task
}

async fn client(ctx: &'static AppContext, endpoint: &Endpoint) -> Client {
    let info = ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    connect_or_absent(endpoint, ctx.app_name, info)
        .await
        .unwrap()
        .unwrap()
        .0
}

async fn stop(ctx: &'static AppContext, endpoint: &Endpoint, task: tokio::task::JoinHandle<()>) {
    let c = client(ctx, endpoint).await;
    let _: Value = c
        .call(sapphire_ipc::SHUTDOWN_METHOD, json!({}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("the server stopped")
        .unwrap();
}

/// A config that listens on `port` and turns metrics on or off.
fn metrics_config(port: u16, enabled: bool) -> ServerConfig {
    ServerConfig {
        http: HttpConfig {
            listen: Some(([127, 0, 0, 1], port).into()),
            metrics: HttpMetricsConfig { enabled },
            ..HttpConfig::default()
        },
        ..ServerConfig::default()
    }
}

/// `GET path` on the listener at `port`, with `token` if there is one.
async fn get_from(port: u16, path: &str, token: Option<&str>) -> reqwest::Response {
    let mut req = reqwest::Client::new().get(format!("http://127.0.0.1:{port}{path}"));
    if let Some(token) = token {
        req = req.bearer_auth(token);
    }
    req.send().await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_alone_starts_the_listener() {
    let tmp = tempfile::tempdir().unwrap();
    let _env = point_at(&CTX_ALONE, tmp.path());
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let endpoint = Endpoint::in_dir(CTX_ALONE.app_name, tmp.path().join("run"));
    let port = free_port();

    let server = AppServer::new(&CTX_ALONE, "0.0.0")
        .config(metrics_config(port, true))
        .http_auth(AuthConfig::new(Arc::new(Fake)));
    let task = start(server, &endpoint).await;

    let res = get_from(port, "/metrics", None).await;
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let content_type = res
        .headers()
        .get("content-type")
        .expect("a content type")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(content_type.starts_with("text/plain"), "{content_type}");
    let body = res.text().await.unwrap();
    assert!(
        body.contains(
            "sapphire_build_info{app=\"sapphire-metricstest-alone\",version=\"0.0.0\"} 1"
        ),
        "{body}"
    );
    #[cfg(target_os = "linux")]
    assert!(body.contains("process_"), "{body}");

    stop(&CTX_ALONE, &endpoint, task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_ignores_a_bad_token() {
    let tmp = tempfile::tempdir().unwrap();
    let _env = point_at(&CTX_TOKEN, tmp.path());
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let endpoint = Endpoint::in_dir(CTX_TOKEN.app_name, tmp.path().join("run"));
    let port = free_port();

    let server = AppServer::new(&CTX_TOKEN, "0.0.0")
        .config(metrics_config(port, true))
        .http_auth(AuthConfig::new(Arc::new(Fake)));
    let task = start(server, &endpoint).await;

    let res = get_from(port, "/metrics", Some("nope")).await;
    assert_eq!(res.status(), reqwest::StatusCode::OK);

    stop(&CTX_TOKEN, &endpoint, task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn app_routes_stay_behind_the_token() {
    let tmp = tempfile::tempdir().unwrap();
    let _env = point_at(&CTX_APP, tmp.path());
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let endpoint = Endpoint::in_dir(CTX_APP.app_name, tmp.path().join("run"));
    let port = free_port();

    let server = AppServer::new(&CTX_APP, "0.0.0")
        .config(metrics_config(port, true))
        .http(axum::Router::new().route("/app", get(|| async { "app" })))
        .http_auth(AuthConfig::new(Arc::new(Fake)));
    let task = start(server, &endpoint).await;

    let res = get_from(port, "/app", None).await;
    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);
    let res = get_from(port, "/app", Some(TOKEN)).await;
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    assert_eq!(res.text().await.unwrap(), "app");
    let res = get_from(port, "/metrics", None).await;
    assert_eq!(res.status(), reqwest::StatusCode::OK);

    stop(&CTX_APP, &endpoint, task).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_off_serves_no_route() {
    let tmp = tempfile::tempdir().unwrap();
    let _env = point_at(&CTX_OFF, tmp.path());
    std::fs::create_dir_all(tmp.path().join("run")).unwrap();
    let endpoint = Endpoint::in_dir(CTX_OFF.app_name, tmp.path().join("run"));
    let port = free_port();

    let server = AppServer::new(&CTX_OFF, "0.0.0")
        .config(metrics_config(port, false))
        .http(axum::Router::new().route("/app", get(|| async { "app" })))
        .http_auth(AuthConfig::new(Arc::new(Fake)));
    let task = start(server, &endpoint).await;

    let res = get_from(port, "/metrics", Some(TOKEN)).await;
    assert_eq!(res.status(), reqwest::StatusCode::NOT_FOUND);

    stop(&CTX_OFF, &endpoint, task).await;
}
