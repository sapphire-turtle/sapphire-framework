//! An application's MCP handler, and the transports that serve it.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ServerHandler, ServiceExt as _};
use tokio_util::sync::CancellationToken;

use crate::context::ToolContext;
use crate::error::{Error, Result};
use crate::server::FrameworkServer;

/// The `Host` names `/mcp` always accepts: loopback.
///
/// rmcp refuses a request whose `Host` header is not on its allowlist — its defence
/// against DNS rebinding. A server reached by any other name (a LAN name, a Tailscale name,
/// the address it binds) needs that name added; loopback is kept on top of whatever is
/// added, so widening the list never costs local use, and the list is never empty, which
/// rmcp would read as "allow every host".
pub const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// How an application answers MCP: a factory of its handler, one per session.
///
/// Type-erased so the app server can hold it without naming the handler, and cheap to
/// clone. [`McpService::framework`] is the default, with the framework's tools alone.
#[derive(Clone)]
pub struct McpService {
    factory: Arc<dyn Factory>,
}

impl std::fmt::Debug for McpService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpService").finish_non_exhaustive()
    }
}

impl Default for McpService {
    fn default() -> Self {
        McpService::framework()
    }
}

impl McpService {
    /// A service that builds a handler with `factory` for every session.
    pub fn new<S, F>(factory: F) -> Self
    where
        S: ServerHandler + Send + Sync + 'static,
        F: Fn(ToolContext) -> S + Send + Sync + 'static,
    {
        McpService {
            factory: Arc::new(Typed(Arc::new(factory))),
        }
    }

    /// The framework's tools and nothing else: [`FrameworkServer`].
    pub fn framework() -> Self {
        McpService::new(FrameworkServer::new)
    }

    /// Serve one session on this process's stdin and stdout until the agent closes it.
    ///
    /// Stdout carries the protocol, so nothing else may print there while this runs; the
    /// framework's log goes to stderr.
    pub async fn serve_stdio(&self, ctx: ToolContext) -> Result<()> {
        self.factory.serve_stdio(ctx).await
    }

    /// The `/mcp` route over MCP's Streamable HTTP transport, for the app server's HTTP
    /// listener.
    ///
    /// `allowed_hosts` are the `Host` names clients use beyond [`LOOPBACK_HOSTS`]. The
    /// route is **not authenticated**: the caller puts it behind
    /// `sapphire_framework_keys::protect`. Its sessions end when `cancel` fires.
    ///
    /// A `subscriptions/listen` holds its response open as an event stream, kept alive by an
    /// SSE comment every 15 seconds (rmcp's default). That is under common reverse-proxy idle
    /// timeouts, such as nginx's 60-second `proxy_read_timeout`; a proxy with a shorter one
    /// cuts the stream, and the client listens again.
    pub fn http_router(
        &self,
        ctx: ToolContext,
        allowed_hosts: &[String],
        cancel: CancellationToken,
    ) -> axum::Router {
        let hosts: Vec<String> = LOOPBACK_HOSTS
            .iter()
            .map(|host| (*host).to_owned())
            .chain(
                allowed_hosts
                    .iter()
                    .map(|host| host.trim())
                    .filter(|host| !host.is_empty())
                    .map(str::to_owned),
            )
            .collect();
        let config = StreamableHttpServerConfig::default()
            .with_cancellation_token(cancel)
            .with_allowed_hosts(hosts);
        self.factory.http_router(ctx, config)
    }
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// [`McpService`]'s factory, with the handler type erased.
trait Factory: Send + Sync {
    fn serve_stdio(&self, ctx: ToolContext) -> BoxFuture<Result<()>>;
    fn http_router(&self, ctx: ToolContext, config: StreamableHttpServerConfig) -> axum::Router;
}

/// The factory, with the handler type still known. Shared, because every HTTP session
/// calls it.
struct Typed<F>(Arc<F>);

impl<S, F> Factory for Typed<F>
where
    S: ServerHandler + Send + Sync + 'static,
    F: Fn(ToolContext) -> S + Send + Sync + 'static,
{
    fn serve_stdio(&self, ctx: ToolContext) -> BoxFuture<Result<()>> {
        let handler = (self.0)(ctx);
        Box::pin(async move {
            let running = handler
                .serve(rmcp::transport::stdio())
                .await
                .map_err(|err| Error::Mcp(err.to_string()))?;
            running
                .waiting()
                .await
                .map_err(|err| Error::Mcp(err.to_string()))?;
            Ok(())
        })
    }

    fn http_router(&self, ctx: ToolContext, config: StreamableHttpServerConfig) -> axum::Router {
        let factory = Arc::clone(&self.0);
        let service = StreamableHttpService::new(
            move || Ok(factory(ctx.clone())),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        axum::Router::new().route_service(crate::HTTP_PATH, service)
    }
}
