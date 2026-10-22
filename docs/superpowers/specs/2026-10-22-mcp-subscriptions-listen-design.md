# `subscriptions/listen` in the framework's MCP handler

- Issue: #270
- PR: (to be opened)
- Crates: `sapphire-framework-mcp`
- Out of scope: what a client does with a notification (sapphire-agent's handling of
  `tools/list_changed` is its own issue); sending any notification from the framework's
  own tools, whose list never changes at runtime.
- Status: accepted

## Why

MCP 2026-07-28 (rmcp 3.x's `ProtocolVersion::LATEST`) has no session and no `initialize`.
A server reaches a client outside a request only through a long-lived
`subscriptions/listen` request. The framework's handler does not implement it:
`server_info()` advertises tools without `listChanged`, and neither `FrameworkServer` nor
an application's handler implements `accepted_subscription_filter`, so rmcp answers
`subscriptions/listen` with -32601. sapphire-timer works around this with its own
`get_info`, `accepted_subscription_filter` and `listen`.

The framework should carry the plumbing: an application says what it offers and calls
`notify_*`, and three one-line delegations in its `ServerHandler` do the rest.

## Decisions

### 1. Notifications go through `ToolContext`

`ToolContext`'s shared `Inner` gains a `tokio::sync::broadcast::Sender<Notice>`. Every
handler built from one context shares it: on HTTP the factory clones the context for each
request, and the clones share `Inner`; on stdio there is one context per process. This is
the "state outside the handler" the stateless HTTP transport needs.

```rust
// crate-private
struct Notice { to: Option<Caller>, change: Change }
enum Change { ToolListChanged, ResourceListChanged, ResourceUpdated(String) }
```

Public methods on `ToolContext`, synchronous, never failing (no subscriber is not an
error):

- `notify_tool_list_changed()`
- `notify_resource_list_changed()`
- `notify_resource_updated(uri)` — to every subscriber of `uri`
- `notify_resource_updated_to(&Caller, uri)` — to that caller's subscriptions only

Prompts are left out: the framework has none. Only list changes and resource updates exist
because rmcp's `SubscriptionSink` sends nothing else.

### 2. `Caller`: who a request comes from

Moved from sapphire-timer's `Client::of` into the framework, as the key for addressed
notifications and for any per-client state an application keeps.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Caller {
    /// stdio: the agent that launched this process.
    Local,
    /// HTTP, with a device token: that device.
    Device(GrainId),
    /// HTTP without an authenticated device.
    Anonymous,
}

impl Caller {
    pub fn of(extensions: &rmcp::model::Extensions) -> Caller;
}
```

`of` reads `http::request::Parts` from the request's extensions (rmcp puts them there on
HTTP) and `sapphire_framework_keys::Authenticated` from those (`protect` puts it there).
No `Parts` means stdio. The crate gains a dependency on `sapphire-framework-keys`
(without its `axum` feature); `http` comes from `axum::http`.

Every anonymous caller is the same `Caller`, so a notice addressed to `Anonymous` reaches
all of them. With the HTTP route behind `protect`, there are none.

### 3. `Subscriptions`: what a handler offers

```rust
#[derive(Clone, Debug, Default)]
pub struct Subscriptions { resources: Vec<String> }

impl Subscriptions {
    pub fn new() -> Self;                                 // tools/list_changed only
    pub fn resource(self, uri: impl Into<String>) -> Self; // add a subscribable URI
    pub fn capabilities(&self) -> ServerCapabilities;
    pub fn accept(&self, requested: &SubscriptionFilter) -> SubscriptionFilter;
    pub fn server_info(&self, ctx: &ToolContext) -> ServerConfig; // server_info(ctx) with these capabilities
}
```

- `capabilities` is tools with `listChanged`; with any resource added, also resources with
  `subscribe` and `listChanged`. rmcp intersects the accepted filter with what
  `get_info` advertises, so the two must agree, and one value produces both.
- `accept` is `requested` ∩ the offer: `tools_list_changed`, `resources_list_changed` when
  resources are offered, and the requested URIs that are on the list. Never `None`, so
  `subscriptions/listen` is always implemented.
- `server_info(ctx)` is `Subscriptions::new().server_info(ctx)`, so it now advertises
  `tools.listChanged`. An application that offers resources returns
  `subs.server_info(&ctx)` instead (rmcp's `ServerConfig` has no capabilities setter; the
  field is public, and this sets it).

### 4. `listen(ctx, subscription)`

```rust
pub async fn listen(ctx: &ToolContext, subscription: SubscriptionContext) -> Result<(), ErrorData>;
```

Subscribes to the context's broadcast, takes the caller from the subscription's request
context, and until the subscription is cancelled or its sink closes, forwards each notice
that is addressed to everyone or to this caller and that the accepted filter allows.
Notices the filter does not allow are dropped here rather than sent and refused by the
sink.

When the receiver lags (`RecvError::Lagged`), which notices were lost is unknown, so it
sends every notification the accepted filter allows once: a stream that cannot be resumed
promises only "something changed, read again". The broadcast closing ends the listen with
`Ok(())`.

### 5. The application's handler

```rust
#[tool_handler]
impl ServerHandler for TimerTools {
    fn get_info(&self) -> ServerConfig {
        self.subs.server_info(&self.ctx)
    }
    fn accepted_subscription_filter(&self, r: &SubscriptionFilter) -> Option<SubscriptionFilter> {
        Some(self.subs.accept(r))
    }
    async fn listen(&self, s: SubscriptionContext) -> Result<(), ErrorData> {
        mcp::listen(&self.ctx, s).await
    }
}
```

`FrameworkServer` implements the same three with `Subscriptions::new()`. sapphire-timer
then sends `ctx.notify_resource_updated_to(&caller, ACTIVE_TIMER_URI)` for the caller whose
timer changed and drops its own `listen`. The crate docs' example shows the three
delegations.

### 6. Keep-alive

rmcp keeps a listen stream on HTTP alive with an SSE comment every 15 seconds
(`StreamableHttpServerConfig::sse_keep_alive`'s default). Common reverse-proxy idle
timeouts are longer (nginx's `proxy_read_timeout` defaults to 60 seconds), so the default
stays and `http_router`'s docs say so.

## Testing

Integration tests in `crates/sapphire-framework-mcp/tests/listen.rs`. The client is rmcp
with `serve_with_lifecycle(ClientLifecycleMode::Discover { .. })`: a plain `serve()` uses
the legacy `initialize`, under which rmcp refuses `subscriptions/listen`. The context is
`ToolContext::dial` at an endpoint nothing listens on; no test calls a tool.

- **stdio**: the handler served on one end of a `tokio::io::duplex`, the same rmcp
  async-read/write transport `serve_stdio` uses on stdin/stdout.
- **HTTP**: `McpService::http_router` on a loopback port.

On both:

- `FrameworkServer` acknowledges `tools_list_changed`, and `notify_tool_list_changed`
  reaches the subscription.
- A handler offering one URI acknowledges that URI and drops a URI it does not offer;
  `notify_resource_updated` reaches the subscription.

On HTTP, with two callers told apart by a fake `Authenticated` inserted ahead of the route:
a notice addressed to one caller reaches its subscription and not the other's.

Unit tests: `Subscriptions::accept` and `capabilities`, and `Caller::of` for no `Parts`,
`Parts` with and without `Authenticated`.

Dev-dependencies: rmcp's `client` and `transport-streamable-http-client-reqwest`
features, and `tokio`'s `net` and `io-util`.
