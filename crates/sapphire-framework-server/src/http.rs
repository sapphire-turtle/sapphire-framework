//! The server's HTTP listener (feature `http`).
//!
//! One listener per server, shared by every HTTP route — the framework's `/mcp` (feature
//! `mcp`) and the application's own ([`AppServer::http`](crate::AppServer::http)). Every
//! route is behind an external device's bearer token, checked with the bridge
//! (`sapphire-framework-keys`): HTTP is how a client that does not sync reaches the
//! application, and the IPC endpoint's same-user rule does not reach across a network.
//!
//! The listener binds only when something is served over it. A listener that cannot bind
//! does not stop the server: its IPC endpoint and sync keep working, and the failure is
//! logged as an error.

use std::net::SocketAddr;

use sapphire_framework_keys::{AuthConfig, protect};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Serve `routes` behind `auth` at `addr`, until `stop` fires.
///
/// `None` when nothing could be bound — already logged.
pub(crate) async fn start(
    addr: SocketAddr,
    routes: axum::Router,
    auth: AuthConfig,
    stop: CancellationToken,
) -> Option<(SocketAddr, JoinHandle<()>)> {
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!("the HTTP listener could not bind {addr}: {err}");
            return None;
        }
    };
    let bound = listener.local_addr().unwrap_or(addr);
    tracing::info!("serving HTTP on {bound}");
    let app = protect(auth, routes);
    let task = tokio::spawn(async move {
        let served = axum::serve(listener, app)
            .with_graceful_shutdown(async move { stop.cancelled().await })
            .await;
        if let Err(err) = served {
            tracing::error!("the HTTP listener ended: {err}");
        }
    });
    Some((bound, task))
}
