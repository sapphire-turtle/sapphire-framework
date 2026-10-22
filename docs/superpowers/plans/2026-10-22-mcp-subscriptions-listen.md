# `subscriptions/listen` in the framework's MCP handler — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A 2026-07-28 MCP client can `subscriptions/listen` on any framework app, over stdio and HTTP, and receive the changes the app announces through `ToolContext`.

**Architecture:** `ToolContext`'s shared `Inner` gains a broadcast of changes; `Subscriptions` says what a handler offers (capabilities + accepted filter); `listen(ctx, subscription)` forwards accepted changes to rmcp's `SubscriptionSink`. `FrameworkServer` delegates to these, and an app's handler does the same in three lines.

**Tech Stack:** Rust, rmcp 3.5.1, tokio broadcast, axum.

**Spec:** `docs/superpowers/specs/2026-10-22-mcp-subscriptions-listen-design.md`

## Global Constraints

- Crate: `crates/sapphire-framework-mcp` only. No other crate's API changes; no new normal dependency.
- No per-client addressing, no `Caller` (spec: out of scope; stays in sapphire-timer).
- Dev-dependencies: rmcp with `client` and `transport-streamable-http-client-reqwest` (add `reqwest` too if the build asks for it); tokio `net`, `io-util`, `time`. Dev-deps on sibling crates stay path-only.
- Do not bump any crate's version; release-plz does that.
- Changes are only `tools/list_changed`, `resources/list_changed`, `resources/updated` — nothing for prompts.
- Never return `None` from `accepted_subscription_filter`.
- Keep rmcp's default `sse_keep_alive` (15 s); document it, don't change it.
- `#![warn(missing_docs)]` holds: every new pub item has a doc comment, in the crate's voice.
- Every cargo command in the worktree runs with `CARGO_TARGET_DIR=$HOME/.cache/claude-cargo-targets/sapphire-framework-270`.
- Stage files by explicit path; never `git add -A`.

## Review Focus

1. A change announced before a client listens must not be replayed when it does (Task 3: `a_change_before_listening_is_not_replayed`).
2. A client that cancels its listen must end the server's `listen` task, not leak a receiver (Task 3: `cancelling_the_subscription_ends_listen`).
3. A listen asking only for something not offered is acknowledged with an empty filter and stays open without error (Task 3: `a_listen_for_nothing_offered_is_acknowledged_empty`).
4. Notifying with nobody listening is silent — no error, no panic (Task 1: `notifying_with_no_listener_is_fine`).
5. A lagging subscriber is told everything it accepted once, not dropped (Task 3: `lag_sends_everything_accepted_once`, on the pure helper).

---

### Task 1: Changes on `ToolContext`

**Files:**
- Create: `crates/sapphire-framework-mcp/src/change.rs` (crate-private)
- Modify: `crates/sapphire-framework-mcp/src/context.rs`
- Modify: `crates/sapphire-framework-mcp/src/lib.rs` (`mod change;`)

**Interfaces:**
- Produces (crate-private, `change.rs`):
  ```rust
  pub(crate) const CHANGE_CAPACITY: usize = 64;
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub(crate) enum Change { ToolListChanged, ResourceListChanged, ResourceUpdated(String) }
  ```
- Produces (on `ToolContext`):
  ```rust
  pub fn notify_tool_list_changed(&self);
  pub fn notify_resource_list_changed(&self);
  pub fn notify_resource_updated(&self, uri: impl Into<String>);
  pub(crate) fn changes(&self) -> tokio::sync::broadcast::Receiver<Change>;
  #[cfg(test)] pub(crate) fn listeners(&self) -> usize; // the broadcast's receiver count
  ```

- [ ] **Step 1: Write the failing tests** in `context.rs`'s new `#[cfg(test)] mod tests` (context from `ToolContext::dial("sapphire-test", "0.0.0", None, Endpoint::in_dir("sapphire-test", std::env::temp_dir()))`):
  - `notifying_with_no_listener_is_fine`: call all three `notify_*` with no receiver; no panic.
  - `a_clone_shares_the_changes`: `rx = ctx.changes()`; `ctx.clone().notify_resource_updated("x://a")`; `rx.try_recv() == Ok(Change::ResourceUpdated("x://a".into()))`; `ctx.listeners() == 1`.
- [ ] **Step 2:** `cargo test -p sapphire-framework-mcp context` — fails to compile (methods missing).
- [ ] **Step 3: Implement.** `Inner` gets `changes: broadcast::Sender<Change>` from `broadcast::channel(CHANGE_CAPACITY)` in both `connected` and `dial`; each `notify_*` does `let _ = send(..)` (an `Err` only means nobody listens). Docs: a change is a signal to read again, not the change itself; the struct doc says every handler made from one context shares them, which is how HTTP's per-request handlers reach a subscription another request opened.
- [ ] **Step 4:** same command — passes.
- [ ] **Step 5: Commit** `feat(mcp): announce changes through ToolContext (#270)`.

### Task 2: `Subscriptions`

**Files:**
- Create: `crates/sapphire-framework-mcp/src/subscriptions.rs`
- Modify: `crates/sapphire-framework-mcp/src/server.rs` (`server_info` delegates)
- Modify: `crates/sapphire-framework-mcp/src/lib.rs` (`mod subscriptions; pub use subscriptions::Subscriptions;`, a line in the module-docs piece list)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug, Default)]
  pub struct Subscriptions { resources: Vec<String> }
  impl Subscriptions {
      pub fn new() -> Self;
      pub fn resource(self, uri: impl Into<String>) -> Self;
      pub fn capabilities(&self) -> ServerCapabilities;
      pub fn accept(&self, requested: &SubscriptionFilter) -> SubscriptionFilter;
      pub fn server_info(&self, ctx: &ToolContext) -> ServerConfig;
  }
  ```
  `server::server_info(ctx)` becomes `Subscriptions::new().server_info(ctx)`; the name/version/instructions code moves with it unchanged.

- [ ] **Step 1: Write the failing tests** in `subscriptions.rs`:
  - `the_default_offers_tool_list_changes_only`: `Subscriptions::new().capabilities()` has `tools.list_changed == Some(true)` and `resources == None`.
  - `a_resource_adds_subscribe_and_list_changed`: `.resource("x://a").capabilities().resources` has `subscribe == Some(true)` and `list_changed == Some(true)`.
  - `accept_keeps_only_what_is_offered`: offer `x://a`; requested = tools_list_changed + resources_list_changed + `["x://a", "x://b"]`; accepted == tools_list_changed + resources_list_changed + `["x://a"]`.
  - `accept_without_resources_drops_resource_requests`: `Subscriptions::new().accept(requested above)` == tools_list_changed only.
  - `accept_grants_nothing_unrequested`: offer `x://a`; `accept(&SubscriptionFilter::new()) == SubscriptionFilter::new()`.
  - `accept_agrees_with_what_rmcp_lets_through`: for the request above, `requested.intersection(&accept).intersection(&requested.supported_by(&capabilities()))` equals `accept`.
- [ ] **Step 2:** `cargo test -p sapphire-framework-mcp subscriptions` — fails to compile.
- [ ] **Step 3: Implement.** `capabilities` via `ServerCapabilities::builder().enable_tools().enable_tool_list_changed()`, plus `.enable_resources().enable_resources_subscribe().enable_resources_list_changed()` when `resources` is non-empty. `accept` = `requested.intersection(&offer)`, `offer` built with `SubscriptionFilter::builder()` from the same rule. `server_info` sets the `capabilities` field of what `ServerConfig::new` builds.
- [ ] **Step 4:** same command, and `cargo test -p sapphire-framework-mcp` — all pass.
- [ ] **Step 5: Commit** `feat(mcp): Subscriptions, what a handler offers on subscriptions/listen (#270)`.

### Task 3: `listen`, `FrameworkServer`, stdio tests

**Files:**
- Create: `crates/sapphire-framework-mcp/src/listen.rs` (with `#[cfg(test)] mod tests`)
- Modify: `crates/sapphire-framework-mcp/src/server.rs` (`FrameworkServer` gets `subs: Subscriptions` and the three methods)
- Modify: `crates/sapphire-framework-mcp/src/lib.rs` (`mod listen; pub use listen::listen;`; module-docs example gets the three delegations from spec §4, with a `subs: Subscriptions` field)
- Modify: `crates/sapphire-framework-mcp/Cargo.toml` (dev-deps)

**Interfaces:**
- Consumes: `ToolContext::{changes, listeners, notify_*}`, `Change`, `Subscriptions`.
- Produces:
  ```rust
  pub async fn listen(ctx: &ToolContext, subscription: SubscriptionContext) -> Result<(), ErrorData>;
  // crate-private, unit-tested:
  fn allowed(accepted: &SubscriptionFilter, change: &Change) -> bool;
  fn everything(accepted: &SubscriptionFilter) -> Vec<Change>;
  ```

`listen`'s loop: `rx = ctx.changes()` first; `tokio::select!` on `subscription.cancelled()` (→ `Ok(())`) and `rx.recv()`:
- `Ok(change)` that is `allowed`: send it through `subscription.sink()`; a send error → `tracing::debug!` and `Ok(())`.
- `Err(Lagged(_))`: send each of `everything(accepted)`.
- `Err(Closed)`: `Ok(())`.

- [ ] **Step 1: Write the failing pure tests**: `allowed` true/false per category and per URI; `lag_sends_everything_accepted_once`: for accepted = tools_list_changed + `["x://a","x://b"]`, `everything == [ToolListChanged, ResourceUpdated("x://a"), ResourceUpdated("x://b")]`.
- [ ] **Step 2: Write the failing stdio tests.** Helper: `let (a, b) = tokio::io::duplex(64 * 1024)`; spawn `handler.serve(b)`; client `().serve_with_lifecycle(a, ClientLifecycleMode::Discover { preferred_versions: vec![ProtocolVersion::LATEST] })`. A test handler `Offering` with `Subscriptions::new().resource("x://a")`, the three delegations and an empty `ToolRouter`. Every wait under `tokio::time::timeout(Duration::from_secs(5), ..)`:
  - `the_framework_server_tells_a_listener_its_tools_changed`: listen with tools_list_changed; `acknowledged().tools_list_changed == Some(true)`; `ctx.notify_tool_list_changed()`; `next()` is `ToolListChangedNotification`.
  - `an_offered_resource_is_accepted_and_others_dropped`: `Offering`, request `["x://a","x://b"]`; acknowledged URIs == `["x://a"]`; `notify_resource_updated("x://b")` then `notify_resource_updated("x://a")`; the first `next()` is the `x://a` update.
  - `a_change_before_listening_is_not_replayed`: `notify_tool_list_changed()` before `listen`; after the ack, nothing within 200 ms.
  - `a_listen_for_nothing_offered_is_acknowledged_empty`: `FrameworkServer`, request only `["x://b"]`; ack == `SubscriptionFilter::new()`; `peer().list_tools` still answers.
  - `cancelling_the_subscription_ends_listen`: listen (wait until `ctx.listeners() == 1`), then `subscription.cancel().await`; poll until `ctx.listeners() == 0` within 5 s.
- [ ] **Step 3:** `cargo test -p sapphire-framework-mcp listen` — fails to compile.
- [ ] **Step 4: Implement** `listen`, `allowed`, `everything`, and `FrameworkServer`'s `get_info` (→ `self.subs.server_info(&self.ctx)`), `accepted_subscription_filter`, `listen`.
- [ ] **Step 5:** same command, then `cargo test -p sapphire-framework-mcp` and `cargo test -p sapphire-framework-server --test mcp` — all pass.
- [ ] **Step 6: Commit** `feat(mcp): answer subscriptions/listen in the framework's handler (#270)`.

### Task 4: HTTP tests, keep-alive docs

**Files:**
- Create: `crates/sapphire-framework-mcp/tests/listen_http.rs`
- Modify: `crates/sapphire-framework-mcp/src/service.rs` (`http_router` doc: listen streams kept alive by an SSE comment every 15 s, rmcp's default, under common proxy idle timeouts such as nginx's 60 s)

**Interfaces:**
- Consumes: `McpService::{framework, new, http_router}`, `ToolContext::notify_*`, `Subscriptions`, `listen`.

Server: `service.http_router(ctx.clone(), &[], CancellationToken::new())` served by `axum::serve` on `127.0.0.1:0`. Client: `StreamableHttpClientTransport::from_uri(format!("http://127.0.0.1:{port}/mcp"))`, `serve_with_lifecycle(Discover)` as in Task 3. `Offering` is redefined here: integration tests cannot reach Task 3's test module.

- [ ] **Step 1: Write the tests:**
  - `http_listen_hears_tool_list_changes`: `McpService::framework()`; listen with tools_list_changed; `notify_tool_list_changed()` arrives.
  - `http_listen_hears_an_offered_resource`: `McpService::new(Offering::new)`; request `["x://a","x://b"]`; ack has `["x://a"]`; `notify_resource_updated("x://a")` arrives.
  - `http_shares_changes_across_requests`: two clients both listening; one `notify_tool_list_changed()` reaches both.
- [ ] **Step 2:** `cargo test -p sapphire-framework-mcp --test listen_http` — all pass (the code exists since Task 3; these pin the HTTP path). To prove `http_shares_changes_across_requests` can fail, temporarily make `McpService`'s HTTP factory hand each request a fresh `ToolContext::dial(..)` instead of `ctx.clone()`, see it fail, and restore it.
- [ ] **Step 3:** Write the `http_router` doc paragraph.
- [ ] **Step 4:** `cargo test -p sapphire-framework-mcp`, `cargo clippy -p sapphire-framework-mcp --all-targets -- -D warnings`, `cargo doc -p sapphire-framework-mcp --no-deps` (no warnings), `cargo fmt --check` — all clean.
- [ ] **Step 5: Commit** `test(mcp): subscriptions/listen over HTTP (#270)`.
