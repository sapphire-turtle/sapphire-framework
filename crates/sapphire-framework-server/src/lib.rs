//! The app server skeleton.
//!
//! An application builds one of these, adds its own methods, and runs it. Everything a
//! sapphire app needs on the server side — owning the workspaces, answering `workspace.*`,
//! pushing events — is here.
//!
//! See `docs/superpowers/specs/2026-09-16-process-architecture-design.md` §4.
//!
//! ```rust,ignore
//! static CTX: AppContext = AppContext::new("sapphire-journal");
//!
//! AppServer::new(&CTX, env!("CARGO_PKG_VERSION"))
//!     .extend(|router| router.method("journal.create_entry", create_entry))
//!     .run()
//!     .await?;
//! ```
//!
//! Application methods are named `<app>.<method>`. The framework owns `workspace.*` and
//! `server.*`; anything else is the application's.

#![warn(missing_docs)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use sapphire_backend::protocol as proto;
use sapphire_framework_service::{RunAs, ServiceSpec};
use sapphire_ipc::{Endpoint, ManagedBy, Router, ServerInfo, serve};
use sapphire_workspace::AppContext;

mod command;
mod error;
mod events;
mod handlers;
mod host;
pub mod privilege;
pub mod sync;
#[cfg(test)]
mod test_support;

pub use command::FrameworkCommand;
pub use error::{Error, Result};
pub use events::subscribe_method;
pub use handlers::{workspace_router, workspace_router_with_sync};
pub use host::{DEFAULT_IDLE, DEFAULT_MAX_OPEN, WorkspaceHost};
pub use privilege::{HelperSpec, PrivilegeConfig, UserSpec};
pub use sync::{SyncRuntime, SyncStatus, sync_router};

/// An application's server.
pub struct AppServer {
    ctx: &'static AppContext,
    version: &'static str,
    endpoint: Option<Endpoint>,
    max_open: usize,
    workspace_idle: Duration,
    host: Arc<WorkspaceHost>,
    sync: Option<Arc<SyncRuntime>>,
    extend: Option<Box<dyn FnOnce(Router) -> Router + Send>>,
    /// What the service this application installs carries, when it separates privileges.
    ///
    /// Stored, never applied: [`privilege::apply`] is `main`'s job, because the drop has to
    /// happen before the socket is bound and this builder is merely describing the server.
    privileges: Option<PrivilegeConfig>,
}

impl AppServer {
    /// A server for `ctx`'s application, reporting `version` in its handshake.
    pub fn new(ctx: &'static AppContext, version: &'static str) -> AppServer {
        AppServer {
            ctx,
            version,
            endpoint: None,
            max_open: DEFAULT_MAX_OPEN,
            workspace_idle: DEFAULT_IDLE,
            host: Arc::new(WorkspaceHost::new(ctx)),
            sync: None,
            extend: None,
            privileges: None,
        }
    }

    /// The application this server serves, as its [`AppContext`] names it.
    ///
    /// [`FrameworkCommand`](crate::FrameworkCommand) reads it to build the CLI's endpoint,
    /// so an application never passes its own name twice.
    pub fn app_name(&self) -> &'static str {
        self.ctx.app_name
    }

    /// Listen somewhere other than the application's default endpoint. Used by tests.
    pub fn endpoint(mut self, endpoint: Endpoint) -> AppServer {
        self.endpoint = Some(endpoint);
        self
    }

    /// How many workspaces to keep open, and how long a cold one may linger.
    pub fn limits(mut self, max_open: usize, idle: Duration) -> AppServer {
        self.max_open = max_open;
        self.workspace_idle = idle;
        self.host = Arc::new(WorkspaceHost::with_limits(self.ctx, max_open, idle));
        self
    }

    /// Serve `sync.enable`, `sync.disable` and `sync.status`, and keep a replica of each
    /// synced workspace in step with its files.
    ///
    /// Without this the server has no sync at all: an application that does not call it
    /// never talks to the bridge. The runtime is built by the caller because it needs the
    /// bridge connection, which is the caller's to open and to close.
    pub fn sync(mut self, runtime: Arc<SyncRuntime>) -> AppServer {
        // The runtime re-indexes what a session writes, and the index belongs to this host:
        // tell it where the workspaces live before anything can arrive.
        runtime.set_host(Arc::clone(&self.host));
        self.sync = Some(runtime);
        self
    }

    /// The privilege separation this application runs under, when it has any.
    ///
    /// Stored for [`service_spec`](AppServer::service_spec) to describe the service with: an
    /// installed unit carries the configuration in its environment, so the server it starts
    /// knows whom to become. It is not applied here — that stays `privilege::apply`'s job in
    /// `main`, which must run before the socket is bound (spec §3.1).
    pub fn privileges(mut self, config: PrivilegeConfig) -> AppServer {
        self.privileges = Some(config);
        self
    }

    /// What this application's service is: the arguments a service manager starts it with,
    /// and the privilege separation, if any, that the server it starts needs.
    ///
    /// `["serve"]`: a service manager starts the executable directly, and the executable's
    /// bare invocation is `serve` — the subcommand-less default of
    /// [`FrameworkCommand::Serve`](crate::FrameworkCommand::Serve). A CLI that wants its
    /// service installed hands this to
    /// [`ServiceCommand`](sapphire_framework_service::ServiceCommand).
    pub fn service_spec(&self) -> ServiceSpec {
        ServiceSpec {
            app_name: self.ctx.app_name,
            description: format!("{} server {}", self.ctx.app_name, self.version),
            args: vec!["serve".to_owned()],
            // An app server's files belong to the human who uses it; one started as root
            // would put the cache, the data and the sockets under `/root`.
            system_run_as: RunAs::InvokingUser,
            privileges: self.privileges.clone(),
            post_install: None,
        }
    }

    /// Add the application's own methods.
    pub fn extend(mut self, f: impl FnOnce(Router) -> Router + Send + 'static) -> AppServer {
        self.extend = Some(Box::new(f));
        self
    }

    /// The workspaces this server has open. An application's own handlers use it to reach a
    /// workspace the same way the framework's do.
    pub fn host(&self) -> &Arc<WorkspaceHost> {
        &self.host
    }

    /// Listen until told to stop, or until a signal arrives.
    pub async fn run(self) -> Result<()> {
        let AppServer {
            ctx,
            version,
            endpoint,
            host,
            sync,
            extend,
            ..
        } = self;

        let endpoint = match endpoint {
            Some(e) => e,
            None => Endpoint::for_app(ctx.app_name)?,
        };
        // A server is always-on from here on: only `serve` and the service manager start
        // one, and the service manager starts it for keeps. There is no spawned mode left
        // to report.
        let info = ServerInfo {
            version: version.to_owned(),
            pid: std::process::id(),
            managed_by: ManagedBy::Service,
        };

        let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
        let live = Arc::new(AtomicU64::new(0));

        let mut router = subscribe_method(
            Arc::clone(&host),
            workspace_router_with_sync(Arc::clone(&host), sync.clone()),
        );
        if let Some(runtime) = &sync {
            // `sync_router` is applied *under* the framework's own methods so a later
            // `extend` can still replace one, and after `subscribe_method` so the
            // `workspace.*` namespace is complete.
            router = sync_router(Arc::clone(runtime), router);
        }
        router = router
            .method(proto::SERVER_INFO, {
                let info = info.clone();
                move |_| {
                    let info = info.clone();
                    async move {
                        serde_json::to_value(info)
                            .map_err(|e| sapphire_ipc::RpcError::internal(e.to_string()))
                    }
                }
            })
            .method(sapphire_ipc::SHUTDOWN_METHOD, {
                let stop_tx = stop_tx.clone();
                move |_| {
                    let stop_tx = stop_tx.clone();
                    async move {
                        let _ = stop_tx.send(true);
                        serde_json::to_value(proto::Ack {})
                            .map_err(|e| sapphire_ipc::RpcError::internal(e.to_string()))
                    }
                }
            });
        if let Some(extend) = extend {
            router = extend(router);
        }
        let router = Arc::new(router);

        // Registered before the bind, so a SIGTERM arriving the instant the socket is up
        // cannot fall through to the default handler and kill the process. SIGINT is
        // listened to only through `ctrl_c` (which swallows the default handler on both
        // platforms): a Unix `SignalKind::interrupt()` stream would double-fire for ^C.
        // Windows has no SIGTERM; its arm's future is a future that is never ready, so the
        // arm exists for the macro's sake and never fires.
        #[cfg(unix)]
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        #[cfg(not(unix))]
        let sigterm = std::future::pending::<()>();

        #[cfg(unix)]
        let listener = sapphire_ipc::bind(&endpoint).await?;
        #[cfg(windows)]
        let mut listener = sapphire_ipc::bind(&endpoint)?;

        // The watcher, the bridge announcement loop and the dialer, if sync is on. Started
        // only once the socket is bound, so an early `?` above cannot leave tasks running for
        // a server that never served. All three are allowed to fail: only sync stops when
        // they do, and the app server keeps serving files (spec §10). They are aborted with
        // the server.
        let _sync_tasks = match &sync {
            Some(runtime) => {
                let driver = Arc::clone(runtime);
                let announcements = tokio::spawn(async move {
                    if let Err(err) = driver.run().await {
                        tracing::warn!("the bridge announcement loop ended: {err}");
                    }
                });
                let changing = Arc::clone(runtime);
                let watching = tokio::spawn(async move {
                    if let Err(err) = changing.watch_changes().await {
                        tracing::warn!("the file watcher ended: {err}");
                    }
                });
                // The dialer is what keeps a session open after the initial exchange, so a
                // commit reaches a peer without waiting for the next one (spec §4.2).
                let dialling = Arc::clone(runtime);
                let dialling = tokio::spawn(async move {
                    if let Err(err) = dialling.dial_loop().await {
                        tracing::warn!("the live dial loop ended: {err}");
                    }
                });
                Some((announcements, watching, dialling))
            }
            None => None,
        };

        // Close cold workspaces as their idle limit passes. The server itself no longer
        // exits on idle — it is service-managed for keeps — but an open workspace still
        // holds its cache's exclusive lock, so the LRU sweep stays.
        let ticker = {
            let host = Arc::clone(&host);
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(60));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    host.close_idle();
                }
            })
        };

        loop {
            tokio::select! {
                changed = stop_rx.changed() => {
                    if changed.is_err() || *stop_rx.borrow() {
                        break;
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("interrupted; shutting down");
                    break;
                }
                _ = sigterm.recv() => {
                    #[cfg(unix)]
                    {
                        tracing::info!("terminated; shutting down");
                        break;
                    }
                    #[cfg(not(unix))]
                    {
                        std::future::pending::<()>().await
                    }
                }
                accepted = listener.accept() => {
                    let conn = accepted?;
                    live.fetch_add(1, Ordering::Relaxed);
                    let router = Arc::clone(&router);
                    let app = ctx.app_name;
                    let info = info.clone();
                    let live = Arc::clone(&live);
                    tokio::spawn(async move {
                        let _ = serve(conn, router, app, info).await;
                        live.fetch_sub(1, Ordering::Relaxed);
                    });
                }
            }
        }

        ticker.abort();
        if let Some((announcements, watching, dialling)) = _sync_tasks {
            announcements.abort();
            watching.abort();
            dialling.abort();
        }
        host.close_all();
        drop(listener); // removes the socket file on Unix
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sapphire_ipc::{ClientInfo, Endpoint, ManagedBy};
    use sapphire_workspace::{AppContext, AppKind};
    use std::ffi::OsString;

    use crate::test_support;

    static CTX: AppContext = AppContext::new("sapphire-appservertest");

    /// The env vars `init_ctx` writes, in the order the guard restores them.
    const DIR_VARS: [&str; 3] = [
        "SAPPHIRE_APPSERVERTEST_CACHE_DIR",
        "SAPPHIRE_APPSERVERTEST_DATA_DIR",
        "SAPPHIRE_APPSERVERTEST_CONFIG_DIR",
    ];

    fn client_info() -> ClientInfo {
        ClientInfo {
            kind: "test".into(),
            version: "0.0.0".into(),
            pid: std::process::id(),
        }
    }

    /// Point the context's directories at `tmp` and restore the previous values (present
    /// or absent) when dropped — including while unwinding from a panic. Without the
    /// unconditional restore, a panicking assertion would leave the variables pointing at
    /// a temp directory the test then deletes, and the context of a later test would
    /// resolve into the void.
    struct EnvGuard {
        previous: [Option<OsString>; 3],
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        /// Write the three directory vars while holding the environment lock.
        fn set(lock: std::sync::MutexGuard<'static, ()>, tmp: &std::path::Path) -> EnvGuard {
            let previous = DIR_VARS.map(std::env::var_os);
            let dirs = ["cache", "data", "config"].map(|cat| tmp.join(cat));
            // SAFETY (via `test_support::set`): `lock` serialises every read and write of
            // the process environment in this test binary — the `host`, `handlers` and
            // `events` modules' tests share the same lock — and it is held until `drop`
            // has restored the old values.
            for (name, dir) in DIR_VARS.iter().zip(dirs) {
                test_support::set(name, &dir);
            }
            EnvGuard {
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: `self._lock` still serialises the environment; it is dropped only
            // after this method returns.
            for (name, previous) in DIR_VARS.iter().zip(self.previous.iter_mut()) {
                match previous.take() {
                    Some(value) => unsafe { std::env::set_var(name, value) },
                    None => test_support::remove(name),
                }
            }
        }
    }

    /// Everything one test needs, held for the test's whole body.
    ///
    /// `_tmp` is declared before `_env` so the scratch tree is deleted while the
    /// environment lock is still held: a sibling test that wakes on the lock must never
    /// observe the tree mid-deletion, nor env vars pointing at a deleted directory.
    struct Fixture {
        _tmp: tempfile::TempDir,
        _env: EnvGuard,
        endpoint: Endpoint,
    }

    /// A listening endpoint in a scratch directory, with the context's directories
    /// pointed at the same scratch tree.
    ///
    /// The caller must keep the fixture (and with it the environment lock) alive for the
    /// whole test: the static [`CTX`] is first-writer-wins, so while one test runs, every
    /// other test that would re-point the context's directories must wait.
    fn prepared() -> Fixture {
        let lock = test_support::lock();
        let tmp = tempfile::tempdir().unwrap();
        let env = EnvGuard::set(lock, tmp.path());
        CTX.init(AppKind::Server);
        let endpoint = Endpoint::in_dir("sapphire-appservertest", tmp.path().to_path_buf());
        Fixture {
            _tmp: tmp,
            _env: env,
            endpoint,
        }
    }

    /// Wait until something is listening on `endpoint`.
    ///
    /// `tokio::spawn(server.run())` only schedules the server; without this wait a fast
    /// `connect_or_absent` probe can run before `run` has bound the socket, and with
    /// nothing listening that single unlucky probe is the whole test failing.
    async fn wait_until_listening(endpoint: &Endpoint) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !sapphire_ipc::probe(endpoint).await.unwrap() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the server never started listening"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_server_reports_itself_as_service_managed() {
        let f = prepared();
        let endpoint = f.endpoint.clone();
        let server = AppServer::new(&CTX, "0.0.0").endpoint(endpoint.clone());
        let handle = tokio::spawn(async move { server.run().await });
        wait_until_listening(&endpoint).await;

        let (client, info) =
            sapphire_ipc::connect_or_absent(&endpoint, CTX.app_name, client_info())
                .await
                .unwrap()
                .expect("the server is listening");
        assert_eq!(info.managed_by, ManagedBy::Service);

        let _: serde_json::Value = client
            .call(sapphire_ipc::SHUTDOWN_METHOD, serde_json::json!({}))
            .await
            .unwrap();
        handle.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_running_server_answers_server_info() {
        let f = prepared();
        let endpoint = f.endpoint.clone();

        let server = AppServer::new(&CTX, "1.2.3").endpoint(endpoint.clone());
        let handle = tokio::spawn(async move { server.run().await });
        wait_until_listening(&endpoint).await;

        let (client, info) = sapphire_ipc::connect_or_absent(
            &endpoint,
            CTX.app_name,
            ClientInfo {
                version: "1.2.3".into(),
                ..client_info()
            },
        )
        .await
        .unwrap()
        .expect("the server is listening");
        assert_eq!(info.version, "1.2.3");

        let reported: sapphire_ipc::ServerInfo = client
            .call(
                sapphire_backend::protocol::SERVER_INFO,
                serde_json::json!({}),
            )
            .await
            .unwrap();
        assert_eq!(reported.version, "1.2.3");

        let _: serde_json::Value = client
            .call(sapphire_ipc::SHUTDOWN_METHOD, serde_json::json!({}))
            .await
            .unwrap();
        handle.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_stops_the_server() {
        let f = prepared();
        let endpoint = f.endpoint.clone();

        let server = AppServer::new(&CTX, "0.0.0").endpoint(endpoint.clone());
        let handle = tokio::spawn(async move { server.run().await });
        wait_until_listening(&endpoint).await;

        let (client, _) = sapphire_ipc::connect_or_absent(&endpoint, CTX.app_name, client_info())
            .await
            .unwrap()
            .expect("the server is listening");
        let _: serde_json::Value = client
            .call(sapphire_ipc::SHUTDOWN_METHOD, serde_json::json!({}))
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("the server must stop")
            .unwrap()
            .unwrap();
        assert!(!sapphire_ipc::probe(&endpoint).await.unwrap());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_application_can_add_its_own_methods() {
        let f = prepared();
        let endpoint = f.endpoint.clone();

        let server = AppServer::new(&CTX, "0.0.0")
            .endpoint(endpoint.clone())
            .extend(|r| {
                r.method("sapphire-appservertest.greet", |_| async move {
                    Ok(serde_json::json!("hello"))
                })
            });
        let handle = tokio::spawn(async move { server.run().await });
        wait_until_listening(&endpoint).await;

        let (client, _) = sapphire_ipc::connect_or_absent(&endpoint, CTX.app_name, client_info())
            .await
            .unwrap()
            .expect("the server is listening");
        let greeting: String = client
            .call("sapphire-appservertest.greet", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(greeting, "hello");

        let _: serde_json::Value = client
            .call(sapphire_ipc::SHUTDOWN_METHOD, serde_json::json!({}))
            .await
            .unwrap();
        handle.await.unwrap().unwrap();
    }
}
