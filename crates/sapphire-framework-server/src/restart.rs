//! Restarting the app server in place (#314): `server.restart` and [`serve_restarting`].
//!
//! A restart reads the settings again and runs a fresh [`AppServer`] from the application's
//! build function, as the bridge's `bridge.restart` does: the IPC listener, the pid, the log
//! and whatever the caller holds around the call (the instance lock) stay; everything a
//! server started — HTTP, sync, the update check, open connections — is stopped first.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use sapphire_ipc::{Endpoint, RpcError};
use sapphire_workspace::AppContext;
use tokio::sync::Notify;

use crate::error::{Error, Result};
use crate::settings::{AppSettings, Settings};
use crate::{AppServer, Sigterm, selection_at};

/// How long `server.restart` waits after answering before the restart begins, so the reply
/// is flushed before the connection closes. The bridge's `bridge.restart` waits as long.
pub(crate) const RESTART_DELAY: Duration = Duration::from_millis(50);

/// What a server run by [`AppServer::run`] answers `server.restart` with: nothing would
/// bring it back, so its service manager has to.
pub(crate) const CANNOT_RESTART: &str = "this server cannot restart itself; restart its service";

/// Why [`AppServer::serve_on`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Exit {
    /// Told to stop, or a signal arrived. The server is done.
    Stopped,
    /// `server.restart` asked for a new server: read the settings, build one and run it.
    Restart,
}

/// Answer `server.restart`: with `restart`, fire it once the reply is on its way; without,
/// refuse.
pub(crate) fn request(restart: Option<Arc<Notify>>) -> std::result::Result<(), RpcError> {
    let restart = restart.ok_or_else(|| RpcError::internal(CANNOT_RESTART))?;
    tracing::info!("a restart was requested");
    // Notified after the reply is on its way: the restart closes this very connection, and
    // the caller should hear back first.
    tokio::spawn(async move {
        tokio::task::yield_now().await;
        tokio::time::sleep(RESTART_DELAY).await;
        restart.notify_one();
    });
    Ok(())
}

/// Serve `ctx`'s application with the [`AppServer`] `build` makes; on `server.restart`,
/// read the settings again, build another and serve that, on the same endpoint.
///
/// Each round reads the settings of `owner` and the framework first, so `build` can use the
/// keys that apply on a restart, and the server it returns serves them through `config.*`.
/// The first round reads the workspace scope from the default selection file; if the built
/// server keeps its selection elsewhere ([`AppServer::selection_file`]), the settings are
/// read again from there before the server runs, and later rounds read them from there
/// straight away.
///
/// The process stays: its pid, the IPC listener (bound once and never rebound — on Windows
/// a pipe name lives while any client holds an instance of it, so binding anew would fail
/// while clients of the old server are connected), the log, and whatever the caller holds
/// around this call, such as the instance lock. Returns when a server stops for good, or
/// when reading the settings, a bind or a run fails.
///
/// ```rust,ignore
/// serve_restarting(&CTX, MyOwner, |settings: &Settings| {
///     AppServer::new(&CTX, VERSION).sync_via_bridge().extend(routes)
/// })
/// .await?;
/// ```
pub async fn serve_restarting<A, F>(ctx: &'static AppContext, owner: A, build: F) -> Result<()>
where
    A: AppSettings + Clone,
    F: Fn(&Settings) -> AppServer,
{
    // Held across every round, so a restart neither drops nor reopens the log file; its
    // drop flushes the last lines when the server stops for good.
    let _log = sapphire_workspace::logging::install(ctx)?;
    let owner: Arc<dyn AppSettings> = Arc::new(owner);
    // The selection file the settings' workspace scope is read from: the default one
    // until a built server names its own.
    let mut selection: Option<PathBuf> = None;
    let mut listener: Option<(Endpoint, sapphire_ipc::Listener)> = None;
    // One SIGTERM stream for every round: a signal that arrives while one server winds down
    // is queued here and stops the next, where a stream of each round's own would miss it
    // and leave the service manager waiting to kill the process.
    let mut sigterm: Option<Sigterm> = None;
    loop {
        let load = |selection: &Option<PathBuf>| {
            Settings::load_with(
                ctx,
                Arc::clone(&owner),
                selection_at(ctx, selection.clone()),
            )
        };
        let mut settings = load(&selection)?;
        let mut server = build(&settings);
        if !std::ptr::eq(server.ctx, ctx) {
            return Err(Error::Config(format!(
                "the build function serves {}, not {}",
                server.ctx.app_name, ctx.app_name
            )));
        }
        if server.selection_file != selection {
            selection = server.selection_file.clone();
            settings = load(&selection)?;
        }
        server.settings = Some(settings);

        if server.signals && sigterm.is_none() {
            // Before the bind, so a SIGTERM arriving the instant the socket is up cannot
            // fall through to the default handler and kill the process.
            sigterm = Sigterm::register()?;
        }
        let endpoint = server.endpoint_or_default()?;
        if listener
            .as_ref()
            .is_none_or(|(bound, _)| *bound != endpoint)
        {
            // Let go of a listener bound elsewhere before binding anew.
            drop(listener.take());
            #[cfg(unix)]
            let bound = sapphire_ipc::bind(&endpoint).await?;
            #[cfg(windows)]
            let bound = sapphire_ipc::bind(&endpoint)?;
            listener = Some((endpoint, bound));
        }
        let (_, bound) = listener.as_mut().expect("bound above");

        // A fresh one each round: a second `server.restart` to one server must not leave a
        // permit that restarts the next the moment it starts.
        let restart = Arc::new(Notify::new());
        match server.serve_on(bound, Some(restart), &mut sigterm).await? {
            Exit::Stopped => return Ok(()),
            Exit::Restart => tracing::info!("restarting (requested over IPC)"),
        }
    }
}
