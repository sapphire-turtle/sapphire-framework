//! `serve` runs until signalled, and shuts down cleanly.
#![cfg(unix)]

use std::time::Duration;

use sapphire_framework_server::AppServer;
use sapphire_ipc::Endpoint;
use sapphire_workspace::{AppContext, AppKind};

static CTX: AppContext = AppContext::new("sapphire-signals-test");

#[tokio::test(flavor = "multi_thread")]
async fn sigterm_ends_a_running_server() {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: single-threaded test-process setup, before any other thread reads these.
    unsafe {
        std::env::set_var("SAPPHIRE_SIGNALSTEST_CACHE_DIR", tmp.path().join("cache"));
        std::env::set_var("SAPPHIRE_SIGNALSTEST_DATA_DIR", tmp.path().join("data"));
        std::env::set_var("SAPPHIRE_SIGNALSTEST_CONFIG_DIR", tmp.path().join("config"));
    }
    CTX.init(AppKind::Server);

    let endpoint = Endpoint::in_dir(CTX.app_name, tmp.path().to_path_buf());
    let server = AppServer::new(&CTX, env!("CARGO_PKG_VERSION")).endpoint(endpoint.clone());
    let handle = tokio::spawn(async move { server.run().await });

    // The signal streams are registered before the bind, so by the time the socket is
    // up, SIGTERM cannot fall through to the default handler and kill the harness.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !sapphire_ipc::probe(&endpoint).await.unwrap() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the server never started"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // SAFETY: this process *is* the test subject; the handler is tokio's.
    unsafe { libc::kill(std::process::id() as libc::pid_t, libc::SIGTERM) };

    tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("SIGTERM must stop the server")
        .unwrap()
        .unwrap();
    assert!(
        !sapphire_ipc::probe(&endpoint).await.unwrap(),
        "the socket file must be gone after shutdown"
    );
}
