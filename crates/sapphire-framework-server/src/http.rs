//! The server's HTTP listener (feature `http`).
//!
//! One listener per server, shared by every HTTP route — the framework's `/mcp` (feature
//! `mcp`) and `/metrics` (feature `metrics`), and the application's own
//! ([`AppServer::http`](crate::AppServer::http)). Every route but `/metrics` is behind an
//! external device's bearer token, checked with the bridge (`sapphire-framework-keys`):
//! HTTP is how a client that does not sync reaches the application, and the IPC endpoint's
//! same-user rule does not reach across a network. `/metrics` is for a Prometheus scraper,
//! and carries nothing worth a token.
//!
//! The listener binds only when something is served over it. A listener that cannot bind
//! does not stop the server: its IPC endpoint and sync keep working, and the failure is
//! logged as an error.

use std::net::SocketAddr;

use sapphire_framework_keys::{AuthConfig, protect};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Serve `protected` behind `auth`, and `public` without it, at `addr`, until `stop` fires.
///
/// A request goes to `public` first, and whatever `public` has no route for falls through
/// to `protected`, token check and all — an unknown path without a token is refused, not
/// answered with a 404. The two are joined with a fallback rather than merged, so their
/// fallbacks never collide (`merge` panics on two).
///
/// `None` when nothing could be bound — already logged — or when there is nothing to serve.
pub(crate) async fn start(
    addr: SocketAddr,
    protected: Option<axum::Router>,
    public: Option<axum::Router>,
    auth: AuthConfig,
    stop: CancellationToken,
) -> Option<(SocketAddr, JoinHandle<()>)> {
    let app = match (public, protected) {
        (None, None) => return None,
        (Some(public), None) => public,
        (None, Some(protected)) => protect(auth, protected),
        (Some(public), Some(protected)) => public.fallback_service(protect(auth, protected)),
    };
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::error!("the HTTP listener could not bind {addr}: {err}");
            return None;
        }
    };
    let bound = listener.local_addr().unwrap_or(addr);
    tracing::info!("serving HTTP on {bound}");
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
