# `subscriptions/listen` in the framework's MCP handler

- Issue: #270
- PR: (to be opened)
- Crates: `sapphire-framework-mcp`
- Out of scope:
  - What a client does with a notification (sapphire-agent's handling of
    `tools/list_changed` is its own issue).
  - Notifications addressed to one client, and telling clients apart. Only sapphire-timer
    needs them (each client has its own timer), and no other app is expected to; they stay
    in sapphire-timer, which overrides `listen` in its own handler (§5).
  - Sending any notification from the framework's own tools, whose list never changes at
    runtime.
- Status: accepted

## Why

MCP 2026-07-28 (rmcp 3.x's `ProtocolVersion::LATEST`) has no session and no `initialize`.
A server reaches a client outside a request only through a long-lived
`subscriptions/listen` request. The framework's handler does not implement it:
`server_info()` advertises tools without `listChanged`, and neither `FrameworkServer` nor
an application's handler implements `accepted_subscription_filter`, so rmcp answers
`subscriptions/listen` with -32601. sapphire-timer works around this with its own
`get_info` capabilities, `accepted_subscription_filter` and `listen`.

The framework should carry the standard part: an application says what it offers and calls
`notify_*`, and three one-line delegations in its `ServerHandler` do the rest. An
application with needs of its own — sapphire-timer's per-client timer — overrides `listen`
in the handler it already writes; rmcp's `ServerHandler` trait is the injection point, and
the framework adds none.

## Decisions

### 1. Notifications go through `ToolContext`

`ToolContext`'s shared `Inner` gains a `tokio::sync::broadcast::Sender<Change>`. Every
handler built from one context shares it: on HTTP the factory clones the context for each
request, and the clones share `Inner`; on stdio there is one context per process. This is
the "state outside the handler" the stateless HTTP transport needs.

```rust
// crate-private
enum Change { ToolListChanged, ResourceListChanged, ResourceUpdated(String) }
```

Public methods on `ToolContext`, synchronous, never failing (no subscriber is not an
error), each reaching every subscription that accepted that kind of change:

- `notify_tool_list_changed()`
- `notify_resource_list_changed()`
- `notify_resource_updated(uri)`

Prompts are left out: the framework has none. Only list changes and resource updates exist
because rmcp's `SubscriptionSink` sends nothing else.

### 2. `Subscriptions`: what a handler offers

```rust
#[derive(Clone, Debug, Default)]
pub struct Subscriptions { resources: Vec<String> }

impl Subscriptions {
    pub fn new() -> Self;                                  // tools/list_changed only
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
- `server_info(ctx)` keeps advertising tools without `listChanged`: an application whose
  handler returns it and does not yet delegate `listen` would otherwise promise changes it
  answers with -32601. A handler that answers `subscriptions/listen` returns
  `subs.server_info(&ctx)` (rmcp's `ServerConfig` has no capabilities setter; the field is
  public, and this sets it). `FrameworkServer` does.

### 3. `listen(ctx, subscription)`

```rust
pub async fn listen(ctx: &ToolContext, subscription: SubscriptionContext) -> Result<(), ErrorData>;
```

Subscribes to the context's broadcast, and until the subscription is cancelled or its sink
closes, forwards each change the accepted filter allows. Changes the filter does not allow
are dropped here rather than sent and refused by the sink.

When the receiver lags (`RecvError::Lagged`), which changes were lost is unknown, so it
sends every notification the accepted filter allows once: a stream that cannot be resumed
promises only "something changed, read again". The broadcast closing ends the listen with
`Ok(())`.

### 4. The application's handler

```rust
#[tool_handler]
impl ServerHandler for LedgerTools {
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

`FrameworkServer` implements the same three with `Subscriptions::new()`. The crate docs'
example shows the three delegations.

### 5. sapphire-timer

Each client has its own timer, so a change must reach only its owner's subscription. That
stays in sapphire-timer: its `Client` / `Clients` and its own `listen`, which watches the
caller's timer. From the framework it can take `Subscriptions::new().resource(ACTIVE_TIMER_URI)`
for `get_info` and `accepted_subscription_filter`, in place of setting the capabilities by
hand. That change is sapphire-timer's, after this releases.

### 6. Keep-alive

rmcp keeps a listen stream on HTTP alive with an SSE comment every 15 seconds
(`StreamableHttpServerConfig::sse_keep_alive`'s default). Common reverse-proxy idle
timeouts are longer (nginx's `proxy_read_timeout` defaults to 60 seconds), so the default
stays and `http_router`'s docs say so.

## Testing

The client is rmcp with `serve_with_lifecycle(ClientLifecycleMode::Discover { .. })`: a
plain `serve()` uses the legacy `initialize`, under which rmcp refuses
`subscriptions/listen`. The context is `ToolContext::dial` at an endpoint nothing listens
on; no test calls a tool.

- **stdio**: the handler served on one end of a `tokio::io::duplex`, the same rmcp
  async-read/write transport `serve_stdio` uses on stdin/stdout. In-crate tests, so they
  can see the broadcast's receiver count.
- **HTTP**: `McpService::http_router` on a loopback port, in
  `crates/sapphire-framework-mcp/tests/listen_http.rs`.

On both:

- `FrameworkServer` acknowledges `tools_list_changed`, and `notify_tool_list_changed`
  reaches the subscription.
- A handler offering one URI acknowledges that URI and drops a URI it does not offer;
  `notify_resource_updated` reaches the subscription.

On HTTP, two clients listening at once both hear one notification: the per-request
handlers share the context.

Unit tests: `Subscriptions::accept` and `capabilities`; the notify methods.

Dev-dependencies: rmcp's `client` and `transport-streamable-http-client-reqwest`
features, and `tokio`'s `net`, `io-util` and `time`.
