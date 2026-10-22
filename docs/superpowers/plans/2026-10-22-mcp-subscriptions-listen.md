# `subscriptions/listen` in the framework's MCP handler — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A 2026-07-28 MCP client can `subscriptions/listen` on any framework app, over stdio and HTTP, and receive the notices the app sends through `ToolContext` — addressed to everyone or to one caller.

**Architecture:** `ToolContext`'s shared `Inner` gains a broadcast of notices; `Caller` identifies who a request comes from; `Subscriptions` says what a handler offers (capabilities + accepted filter); `listen(ctx, subscription)` forwards matching notices to rmcp's `SubscriptionSink`. `FrameworkServer` delegates to these, and an app's handler does the same in three lines.

**Tech Stack:** Rust, rmcp 3.5.1, tokio broadcast, axum.

**Spec:** `docs/superpowers/specs/2026-10-22-mcp-subscriptions-listen-design.md`

## Global Constraints

- Crate: `crates/sapphire-framework-mcp` only. No other crate's API changes.
- New dependency: `sapphire-framework-keys` (normal dep, `version = "0.21.0"` and `path`, **without** its `axum` feature). `http` types come from `axum::http`.
- Dev-dependencies: rmcp with `client` and `transport-streamable-http-client-reqwest` (add `reqwest` too if the build asks for it); tokio `net`, `io-util`, `time`. Workspace dev-deps stay path-only (no `version`) — only relevant if a sibling crate is added.
- Do not bump any crate's version; release-plz does that.
- Notices carry only `tools/list_changed`, `resources/list_changed`, `resources/updated` — nothing for prompts.
- Never return `None` from `accepted_subscription_filter`.
- Keep rmcp's default `sse_keep_alive` (15 s); document it, don't change it.
- `#![warn(missing_docs)]` holds: every new pub item has a doc comment, in the crate's voice (short, says what it is and why).
- Every cargo command in the worktree runs with `CARGO_TARGET_DIR=$HOME/.cache/claude-cargo-targets/sapphire-framework-270`.
- Stage files by explicit path; never `git add -A`.

## Review Focus

1. A notice sent before a client listens must not be replayed when it does — listen reports only changes after the acknowledgment (Task 4: `a_notice_sent_before_listening_is_not_replayed`).
2. A client that cancels its listen (or drops the connection) must end the server's `listen` task, not leak a receiver (Task 4: `cancelling_the_subscription_ends_listen`).
3. A listen with an empty filter, or one asking only for a URI the handler does not offer, is acknowledged with what is accepted (possibly nothing) and stays open without error (Task 4: `a_listen_for_nothing_offered_is_acknowledged_empty`).
4. Notifying with nobody listening is silent — no error, no panic, no log at warn level (Task 2: `notifying_with_no_listener_is_fine`).
5. A lagging subscriber is told everything it accepted once, not dropped (Task 4: `lag_sends_everything_accepted_once`, on the pure helper).

---

### Task 1: `Caller`

**Files:**
- Create: `crates/sapphire-framework-mcp/src/caller.rs`
- Modify: `crates/sapphire-framework-mcp/Cargo.toml` (add `sapphire-framework-keys`)
- Modify: `crates/sapphire-framework-mcp/src/lib.rs` (`mod caller; pub use caller::Caller;`, mention in the module-docs piece list)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq, Hash)]
  #[non_exhaustive]
  pub enum Caller { Local, Device(sapphire_framework_keys::GrainId), Anonymous }
  impl Caller { pub fn of(extensions: &rmcp::model::Extensions) -> Caller; }
  ```

- [ ] **Step 1: Write the failing tests** in `caller.rs`'s `#[cfg(test)] mod tests`:
  - `no_request_parts_is_local`: `Caller::of(&Extensions::new()) == Caller::Local`.
  - `parts_without_a_device_is_anonymous`: insert `axum::http::Request::new(()).into_parts().0` into `Extensions`; expect `Caller::Anonymous`.
  - `parts_with_a_device_is_that_device`: as above, with `Authenticated { id, name: "agent".into() }` (`id = GrainId::random()`) inserted into the parts' extensions; expect `Caller::Device(id)`.
- [ ] **Step 2:** `cargo test -p sapphire-framework-mcp caller` — fails to compile (`Caller` missing).
- [ ] **Step 3: Implement.** `of` reads `extensions.get::<axum::http::request::Parts>()`; none → `Local`; then `parts.extensions.get::<Authenticated>()` → `Device(who.id)` or `Anonymous`. Doc the enum: the key for addressed notices and per-client state; every anonymous caller is the same `Caller`.
- [ ] **Step 4:** same command — 3 pass.
- [ ] **Step 5: Commit** `feat(mcp): Caller, who an MCP request comes from (#270)`.

### Task 2: Notices on `ToolContext`

**Files:**
- Create: `crates/sapphire-framework-mcp/src/notice.rs` (crate-private types)
- Modify: `crates/sapphire-framework-mcp/src/context.rs`
- Modify: `crates/sapphire-framework-mcp/src/lib.rs` (`mod notice;`)

**Interfaces:**
- Consumes: `Caller` (Task 1).
- Produces (crate-private, in `notice.rs`):
  ```rust
  pub(crate) const NOTICE_CAPACITY: usize = 64;
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub(crate) enum Change { ToolListChanged, ResourceListChanged, ResourceUpdated(String) }
  #[derive(Clone, Debug)]
  pub(crate) struct Notice { pub(crate) to: Option<Caller>, pub(crate) change: Change }
  ```
- Produces (on `ToolContext`):
  ```rust
  pub fn notify_tool_list_changed(&self);
  pub fn notify_resource_list_changed(&self);
  pub fn notify_resource_updated(&self, uri: impl Into<String>);
  pub fn notify_resource_updated_to(&self, caller: &Caller, uri: impl Into<String>);
  pub(crate) fn notices(&self) -> tokio::sync::broadcast::Receiver<Notice>;
  ```

- [ ] **Step 1: Write the failing tests** in `context.rs`'s new `#[cfg(test)] mod tests` (context from `ToolContext::dial("sapphire-test", "0.0.0", None, Endpoint::in_dir("sapphire-test", std::env::temp_dir()))`):
  - `notifying_with_no_listener_is_fine`: call all four `notify_*` with no receiver; no panic.
  - `a_clone_shares_the_notices`: `rx = ctx.notices()`; `ctx.clone().notify_resource_updated("x://a")`; `rx.try_recv()` gives `Notice { to: None, change: ResourceUpdated("x://a") }`.
  - `an_addressed_notice_names_its_caller`: `notify_resource_updated_to(&Caller::Local, "x://a")` gives `to: Some(Caller::Local)`.
- [ ] **Step 2:** `cargo test -p sapphire-framework-mcp context` — fails to compile.
- [ ] **Step 3: Implement.** `Inner` gets `notices: broadcast::Sender<Notice>` made with `broadcast::channel(NOTICE_CAPACITY)` in both `connected` and `dial`; each `notify_*` does `let _ = self.inner.notices.send(..)` (an `Err` only means nobody listens). Doc each method: a notice is a signal to read again, not the change itself.
- [ ] **Step 4:** same command — passes.
- [ ] **Step 5: Commit** `feat(mcp): send notices through ToolContext (#270)`.

### Task 3: `Subscriptions`

**Files:**
- Create: `crates/sapphire-framework-mcp/src/subscriptions.rs`
- Modify: `crates/sapphire-framework-mcp/src/server.rs` (`server_info` delegates)
- Modify: `crates/sapphire-framework-mcp/src/lib.rs` (`mod subscriptions; pub use subscriptions::Subscriptions;`)

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
  - `accept_agrees_with_what_rmcp_lets_through`: for the request above, `requested.intersection(&accept).intersection(&requested.supported_by(&capabilities()))` equals `accept` — the advertised capabilities never cut the accepted filter.
- [ ] **Step 2:** `cargo test -p sapphire-framework-mcp subscriptions` — fails to compile.
- [ ] **Step 3: Implement.** `capabilities` via `ServerCapabilities::builder().enable_tools().enable_tool_list_changed()`, plus `.enable_resources().enable_resources_subscribe().enable_resources_list_changed()` when `resources` is non-empty. `accept` = `requested.intersection(&offer)` where `offer` is built with `SubscriptionFilter::builder()` from the same rule. `server_info` sets the `capabilities` field of what `ServerConfig::new` builds.
- [ ] **Step 4:** same command, and `cargo test -p sapphire-framework-mcp` — all pass.
- [ ] **Step 5: Commit** `feat(mcp): Subscriptions, what a handler offers on subscriptions/listen (#270)`.

### Task 4: `listen`, `FrameworkServer`, stdio tests

**Files:**
- Create: `crates/sapphire-framework-mcp/src/listen.rs`
- Modify: `crates/sapphire-framework-mcp/src/server.rs` (`FrameworkServer` gets `subs: Subscriptions` and the three methods)
- Modify: `crates/sapphire-framework-mcp/src/lib.rs` (`mod listen; pub use listen::listen;`; update the module-docs example to the three delegations from spec §5, with `subs` a field set to `Subscriptions::new()`)
- Modify: `crates/sapphire-framework-mcp/Cargo.toml` (dev-deps)
- Test: `crates/sapphire-framework-mcp/src/listen.rs` (`#[cfg(test)] mod tests`, so tests can see `notices()`'s receiver count)

**Interfaces:**
- Consumes: `Caller::of`, `ToolContext::notices`, `Notice`/`Change`, `Subscriptions`.
- Produces:
  ```rust
  pub async fn listen(ctx: &ToolContext, subscription: SubscriptionContext) -> Result<(), ErrorData>;
  // crate-private, unit-tested:
  fn allowed(accepted: &SubscriptionFilter, change: &Change) -> bool;
  fn everything(accepted: &SubscriptionFilter) -> Vec<Change>;
  ```

`listen`'s loop: subscribe **before** anything else (`ctx.notices()`), `caller = Caller::of(&subscription.request_context().extensions)`; `tokio::select!` on `subscription.cancelled()` (→ `Ok(())`) and `rx.recv()`:
- `Ok(notice)` where `notice.to` is `None` or `Some(caller)`, and `allowed(...)`: send through `subscription.sink()` (`notify_tool_list_changed` / `notify_resource_list_changed` / `notify_resource_updated(uri)`); a send error → `tracing::debug!` and `Ok(())`.
- `Err(Lagged(_))`: send each of `everything(accepted)`.
- `Err(Closed)`: `Ok(())`.

- [ ] **Step 1: Write the failing pure tests**: `allowed` true/false per category and per URI; `lag_sends_everything_accepted_once`: for accepted = tools_list_changed + `["x://a","x://b"]`, `everything == [ToolListChanged, ResourceUpdated("x://a"), ResourceUpdated("x://b")]`.
- [ ] **Step 2: Write the failing stdio tests.** Helper `connect(handler) -> RunningService<RoleClient, ()>`: `let (a, b) = tokio::io::duplex(64 * 1024)`; spawn `handler.serve(b)`; client `().serve_with_lifecycle(a, ClientLifecycleMode::Discover { preferred_versions: vec![ProtocolVersion::LATEST] })`. A test handler `Offering` = `FrameworkServer`-like handler with `Subscriptions::new().resource("x://a")` (write it in the test module with the three delegations plus an empty `ToolRouter`). Each wait wrapped in `tokio::time::timeout(Duration::from_secs(5), ..)`:
  - `the_framework_server_tells_a_listener_its_tools_changed`: listen with tools_list_changed; `acknowledged()` has `tools_list_changed == Some(true)`; `ctx.notify_tool_list_changed()`; `next()` is `ToolListChangedNotification`.
  - `an_offered_resource_is_accepted_and_others_dropped`: `Offering`, request `["x://a","x://b"]`; acknowledged URIs == `["x://a"]`; `notify_resource_updated("x://a")` arrives.
  - `a_notice_for_another_caller_does_not_arrive`: `notify_resource_updated_to(&Caller::Device(GrainId::random()), "x://a")`, then `notify_resource_updated_to(&Caller::Local, "x://a")`; the first `next()` is the second notice (`x://a`, and no other arrives within 200 ms after it).
  - `a_notice_sent_before_listening_is_not_replayed`: `notify_tool_list_changed()` before `listen`; after the ack, nothing within 200 ms.
  - `a_listen_for_nothing_offered_is_acknowledged_empty`: `FrameworkServer`, request only `["x://b"]`; ack == `SubscriptionFilter::new()`; `peer().list_tools` still answers.
  - `cancelling_the_subscription_ends_listen`: listen, then `subscription.cancel().await`; poll until `ctx` has no notice receivers (`inner.notices.receiver_count() == 0`, via a `#[cfg(test)] pub(crate) fn listeners(&self) -> usize` on `ToolContext`) within 5 s.
- [ ] **Step 3:** `cargo test -p sapphire-framework-mcp listen` — fails to compile.
- [ ] **Step 4: Implement** `listen`, `allowed`, `everything`, and `FrameworkServer`'s `get_info` (→ `self.subs.server_info(&self.ctx)`), `accepted_subscription_filter`, `listen`.
- [ ] **Step 5:** same command, then `cargo test -p sapphire-framework-mcp` and `cargo test -p sapphire-framework-server --test mcp` — all pass.
- [ ] **Step 6: Commit** `feat(mcp): answer subscriptions/listen in the framework's handler (#270)`.

### Task 5: HTTP tests, keep-alive docs

**Files:**
- Create: `crates/sapphire-framework-mcp/tests/listen_http.rs`
- Modify: `crates/sapphire-framework-mcp/src/service.rs` (`http_router` doc: listen streams kept alive by an SSE comment every 15 s, rmcp's default, under common proxy idle timeouts such as nginx's 60 s)

**Interfaces:**
- Consumes: `McpService::{framework, new, http_router}`, `ToolContext::notify_*`, `Caller`, `Subscriptions`, `listen`.

Server helper: `McpService::framework().http_router(ctx.clone(), &[], CancellationToken::new())`, layered with an `axum::middleware::from_fn` that, for an `Authorization: Bearer <name>` header, inserts `Authenticated { id, name }` with the `id` the test assigned to that name (no header → no insertion; this stands in for `sapphire_framework_keys::protect`), served by `axum::serve` on `127.0.0.1:0`. Client: `StreamableHttpClientTransport::from_uri(format!("http://127.0.0.1:{port}/mcp"))` built from `StreamableHttpClientTransportConfig::with_uri(..)` plus `.auth_header(name)` for a device, `serve_with_lifecycle(Discover)` as in Task 4.

- [ ] **Step 1: Write the failing tests:**
  - `http_listen_hears_tool_list_changes`: listen with tools_list_changed; `notify_tool_list_changed()` arrives.
  - `http_shares_notices_across_requests`: two separate clients both listening; one `notify_tool_list_changed()` reaches both (the factory's per-request handlers share the context).
  - `http_addressed_notice_reaches_only_its_device`: an `Offering` handler (via `McpService::new`) offering `x://a`; client A as device `a`, client B as device `b`, both listening on `x://a`; `notify_resource_updated_to(&Caller::Device(a), "x://a")` reaches A; B gets nothing within 200 ms.
- [ ] **Step 2:** `cargo test -p sapphire-framework-mcp --test listen_http` — all pass (the code exists since Task 4; these pin the HTTP path). To prove `http_addressed_notice_reaches_only_its_device` can fail, temporarily make `listen` ignore `notice.to`, see it fail, and restore it. `Offering` is redefined in this file: integration tests cannot reach Task 4's test module.
- [ ] **Step 3:** Write the `http_router` doc paragraph.
- [ ] **Step 4:** `cargo test -p sapphire-framework-mcp`, `cargo clippy -p sapphire-framework-mcp --all-targets -- -D warnings`, `cargo doc -p sapphire-framework-mcp --no-deps` (no warnings), `cargo fmt --check` — all clean.
- [ ] **Step 5: Commit** `test(mcp): subscriptions/listen over HTTP (#270)`.
