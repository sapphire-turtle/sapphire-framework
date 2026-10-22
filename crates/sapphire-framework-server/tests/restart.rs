//! `server.restart` (#314): the app server rebuilds itself in place.
//!
//! A server run by [`serve_restarting`] answers `server.restart`, stops what it serves and
//! runs a fresh `AppServer` from the build function on the same endpoint, in the same
//! process, with its settings read again. One run by `AppServer::run` refuses. Each test
//! owns its `AppContext` (the directories are first-writer-wins per context).

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sapphire_backend::protocol as proto;
use sapphire_bridge_api::settings::EntryValue;
use sapphire_bridge_api::{ConfigEntry, Entry};
use sapphire_bridge_client::BridgeClient;
use sapphire_framework_server::{
    AppServer, BridgeConnector, Settings, StatusReport, Value, serve_restarting,
};
use sapphire_ipc::{Client, ClientInfo, Endpoint, Error, connect_or_absent};
use sapphire_workspace::AppContext;
use serde_json::json;

/// How long a test waits for a server to come back.
const DEADLINE: Duration = Duration::from_secs(10);

fn client_info() -> ClientInfo {
    ClientInfo {
        kind: "test".into(),
        version: "0.0.0".into(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    }
}

/// A server for one test, with no bridge running.
struct Fixture {
    tmp: tempfile::TempDir,
    ctx: &'static AppContext,
    endpoint: Endpoint,
    server: Option<tokio::task::JoinHandle<sapphire_framework_server::Result<()>>>,
}

impl Fixture {
    /// Serve `ctx`'s application with [`serve_restarting`], or with `AppServer::run` when
    /// `restarting` is off.
    async fn start(ctx: &'static AppContext, restarting: bool) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        ctx.set_cache_dir(tmp.path().join("cache"));
        ctx.set_data_dir(tmp.path().join("data"));
        ctx.set_config_dir(tmp.path().join("config"));
        std::fs::create_dir_all(tmp.path().join("config")).unwrap();
        std::fs::create_dir_all(tmp.path().join("run")).unwrap();
        let endpoint = Endpoint::in_dir(ctx.app_name, tmp.path().join("run"));
        let selection: PathBuf = tmp.path().join("selection.toml");

        let make = {
            let endpoint = endpoint.clone();
            move || {
                AppServer::new(ctx, "0.0.0")
                    .endpoint(endpoint.clone())
                    .selection_file(selection.clone())
                    .bridge_connector(BridgeConnector::new(|| async { Ok(None) }))
            }
        };
        let server = if restarting {
            tokio::spawn(serve_restarting(ctx, (), move |_: &Settings| make()))
        } else {
            tokio::spawn(make().run())
        };
        let f = Fixture {
            tmp,
            ctx,
            endpoint,
            server: Some(server),
        };
        let deadline = tokio::time::Instant::now() + DEADLINE;
        while !sapphire_ipc::probe(&f.endpoint).await.unwrap() {
            assert!(!f.finished(), "the server stopped");
            assert!(tokio::time::Instant::now() < deadline, "never listened");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        f
    }

    /// Whether the server's task has returned.
    fn finished(&self) -> bool {
        self.server.as_ref().is_none_or(|s| s.is_finished())
    }

    /// A new connection to the server.
    async fn connect(&self) -> Client {
        let (client, _) = connect_or_absent(&self.endpoint, self.ctx.app_name, client_info())
            .await
            .unwrap()
            .expect("the server is listening");
        client
    }

    /// Ask for a restart over `client`, wait until that connection has been closed by the
    /// old server, and connect to the new one.
    async fn restart(&self, client: &Client) -> Client {
        let _: proto::Ack = client
            .call(proto::SERVER_RESTART, json!({}))
            .await
            .expect("server.restart answers");
        let deadline = tokio::time::Instant::now() + DEADLINE;
        loop {
            let answer: sapphire_ipc::Result<StatusReport> =
                client.call(proto::SERVER_INFO, json!({})).await;
            if answer.is_err() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "the old server kept serving"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!self.finished(), "a restart is not a stop");
        self.connect().await
    }

    /// `config.*` over `client`.
    fn config(&self, client: Client) -> BridgeClient {
        BridgeClient::relayed(Arc::new(client), self.tmp.path().join("run"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}

fn entry<'a>(entries: &'a [Entry], key: &str) -> &'a Entry {
    entries
        .iter()
        .find(|e| e.key == key)
        .unwrap_or_else(|| panic!("{key} is listed"))
}

static CTX_SAME: AppContext = AppContext::new("rstsame");

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_serves_again_on_the_same_endpoint() {
    let f = Fixture::start(&CTX_SAME, true).await;
    let client = f.connect().await;
    let again = f.restart(&client).await;
    let report: StatusReport = again.call(proto::SERVER_INFO, json!({})).await.unwrap();
    assert!(report.running);

    // And a stop is still a stop.
    let _: proto::Ack = again
        .call(sapphire_ipc::SHUTDOWN_METHOD, json!({}))
        .await
        .unwrap();
    let mut f = f;
    tokio::time::timeout(DEADLINE, f.server.take().unwrap())
        .await
        .expect("the server stops")
        .unwrap()
        .unwrap();
}

static CTX_SETTING: AppContext = AppContext::new("rstsetting");

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_picks_up_a_restart_setting() {
    let f = Fixture::start(&CTX_SETTING, true).await;
    let client = f.connect().await;
    let config = f.config(f.connect().await);

    let written = config
        .config_set(
            None,
            vec![ConfigEntry {
                key: "update.check".into(),
                value: Value::Bool(false),
            }],
        )
        .await
        .unwrap();
    assert_eq!(written.pending_restart, vec!["update.check"]);
    let listed = config.config_list(None).await.unwrap().entries;
    assert!(entry(&listed, "update.check").pending_restart);

    let again = f.restart(&client).await;
    let listed = f.config(again).config_list(None).await.unwrap().entries;
    let check = entry(&listed, "update.check");
    assert_eq!(check.value, EntryValue::Value(Value::Bool(false)));
    assert!(!check.pending_restart, "the new server started with it");
}

static CTX_RECONNECT: AppContext = AppContext::new("rstreconnect");

#[tokio::test(flavor = "multi_thread")]
async fn a_client_reconnects_after_a_restart() {
    let f = Fixture::start(&CTX_RECONNECT, true).await;
    let restarter = f.connect().await;
    let other = f.connect().await;

    let _again = f.restart(&restarter).await;
    // Every connection of the old server is closed, not only the one that asked.
    let closed: sapphire_ipc::Result<StatusReport> =
        other.call(proto::SERVER_INFO, json!({})).await;
    assert!(closed.is_err(), "{closed:?}");

    let reconnected = f.connect().await;
    let report: StatusReport = reconnected
        .call(proto::SERVER_INFO, json!({}))
        .await
        .unwrap();
    assert!(report.running);
}

static CTX_PROCESS: AppContext = AppContext::new("rstprocess");

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_keeps_the_process_and_lock() {
    let f = Fixture::start(&CTX_PROCESS, true).await;
    let client = f.connect().await;
    let before: StatusReport = client.call(proto::SERVER_INFO, json!({})).await.unwrap();

    let again = f.restart(&client).await;
    let after: StatusReport = again.call(proto::SERVER_INFO, json!({})).await.unwrap();
    assert_eq!(before.pid, Some(std::process::id()));
    assert_eq!(after.pid, before.pid, "the same process serves");
    // The endpoint was never given up: nothing else could have bound it meanwhile, and the
    // task holding it never returned.
    assert!(!f.finished());

    // Twice in a row works too.
    let third = f.restart(&again).await;
    let report: StatusReport = third.call(proto::SERVER_INFO, json!({})).await.unwrap();
    assert_eq!(report.pid, before.pid);
}

static CTX_RUN: AppContext = AppContext::new("rstrun");

#[tokio::test(flavor = "multi_thread")]
async fn run_cannot_restart() {
    let f = Fixture::start(&CTX_RUN, false).await;
    let client = f.connect().await;
    let err = client
        .call::<_, proto::Ack>(proto::SERVER_RESTART, json!({}))
        .await
        .unwrap_err();
    match err {
        Error::Rpc(e) => assert_eq!(
            e.message,
            "this server cannot restart itself; restart its service"
        ),
        other => panic!("expected an RPC error, got {other:?}"),
    }
    // Still serving.
    let report: StatusReport = client.call(proto::SERVER_INFO, json!({})).await.unwrap();
    assert!(report.running);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_with_sync_reopens_the_replica_store() {
    let net = sapphire_framework_bridge::LoopbackNetwork::new();
    let mut host = common::start_host_restarting(&net, common::NODE_A, "host-a").await;
    common::enable_sync(&host).await;
    let status: proto::SyncStatusResult = host
        .client
        .call(proto::SYNC_STATUS, json!({}))
        .await
        .unwrap();
    assert!(status.enabled);

    host.restart_in_place().await;

    // The new server restores sync for the selected workspace, which opens the replica
    // store again: that fails for as long as anything of the old server still holds it.
    let deadline = tokio::time::Instant::now() + DEADLINE;
    loop {
        let status: proto::SyncStatusResult = host
            .client
            .call(proto::SYNC_STATUS, json!({}))
            .await
            .unwrap();
        if status.enabled {
            assert_eq!(status.last_error, None);
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "sync never came back after the restart: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // And serving: a file written now is read back.
    let _: proto::Ack = host
        .client
        .call(
            proto::WRITE_FILE,
            proto::ContentParams {
                path: PathBuf::from("after-restart.md"),
                content: "still here".into(),
            },
        )
        .await
        .unwrap();
    host.stop().await;
}
