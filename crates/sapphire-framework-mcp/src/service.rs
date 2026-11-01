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
    /// A request whose key may only read
    /// ([`Access::Read`](sapphire_framework_keys::Access::Read)) may call only the tools
    /// the handler's `get_tool` marks read-only (`readOnlyHint`); a call to any other is
    /// refused with 403 before the handler sees it.
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
        // One handler to ask which tools are read-only.
        let probe = (self.0)(ctx.clone());
        let read_only: ReadOnly = Arc::new(move |name: &str| {
            probe
                .get_tool(name)
                .and_then(|tool| tool.annotations)
                .and_then(|a| a.read_only_hint)
                == Some(true)
        });
        let factory = Arc::clone(&self.0);
        let service = StreamableHttpService::new(
            move || Ok(factory(ctx.clone())),
            Arc::new(LocalSessionManager::default()),
            config,
        );
        axum::Router::new()
            .route_service(crate::HTTP_PATH, service)
            .layer(axum::middleware::from_fn_with_state(read_only, by_tool))
    }
}

/// Whether the tool of this name only reads.
type ReadOnly = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The largest `/mcp` request a read-only key's is read into memory to be checked.
const MAX_CHECKED_BODY: usize = 16 * 1024 * 1024;

/// Refuse a read-only key's `tools/call` of a tool that is not read-only.
async fn by_tool(
    axum::extract::State(read_only): axum::extract::State<ReadOnly>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let reader = request
        .extensions()
        .get::<sapphire_framework_keys::Authenticated>()
        .is_some_and(|who| !who.access.may_write());
    if !reader {
        return next.run(request).await;
    }
    let (parts, body) = request.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, MAX_CHECKED_BODY).await else {
        return axum::http::StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    if let Some(tool) = writing_call(&bytes, &*read_only) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            format!("this API key may only read: {tool} is not a read-only tool"),
        )
            .into_response();
    }
    let request = axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes));
    next.run(request).await
}

/// The first tool a JSON-RPC message (or batch) in `body` calls that `read_only` does not
/// pass. A body that is not JSON-RPC calls nothing; rmcp answers it.
fn writing_call(body: &[u8], read_only: &dyn Fn(&str) -> bool) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let messages = match value {
        serde_json::Value::Array(batch) => batch,
        message => vec![message],
    };
    messages
        .iter()
        .filter(|m| m.get("method").and_then(|v| v.as_str()) == Some("tools/call"))
        .map(|m| {
            m.pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned()
        })
        .find(|name| !read_only(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reads(name: &str) -> bool {
        name.starts_with("read")
    }

    fn call(name: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": name, "arguments": {}}
        }))
        .unwrap()
    }

    #[test]
    fn a_call_of_a_tool_that_is_not_read_only_is_found() {
        assert_eq!(
            writing_call(&call("write"), &reads).as_deref(),
            Some("write")
        );
        assert_eq!(writing_call(&call("read"), &reads), None);
    }

    #[test]
    fn other_methods_and_batches() {
        let list = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        assert_eq!(writing_call(list, &reads), None);
        let batch = format!(
            "[{},{}]",
            String::from_utf8(call("read")).unwrap(),
            String::from_utf8(call("write")).unwrap()
        );
        assert_eq!(
            writing_call(batch.as_bytes(), &reads).as_deref(),
            Some("write")
        );
        assert_eq!(writing_call(b"not json", &reads), None);
        // A call without a name is no read-only tool's.
        let nameless = br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#;
        assert_eq!(writing_call(nameless, &reads).as_deref(), Some(""));
    }
}
