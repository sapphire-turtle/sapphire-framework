# Bridge Relay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The app's CLI and GUI reach the bridge only through their app server, which relays an allowlist of bridge methods unchanged.

**Architecture:** The app server registers each allowlisted bridge method on its router; the handler forwards the raw params over the app server's one bridge connection (the sync runtime's, or a lazily made one of its own) and returns the bridge's answer. A `BridgeClient::relayed` wraps an app-server connection so clients keep their typed calls and their `NotRunning` errors.

**Tech Stack:** Rust 2024, tokio, `sapphire-framework-ipc` JSON-RPC (`Router`, `Client`), clap, egui.

**Spec:** `docs/superpowers/specs/2026-10-22-bridge-relay-design.md`

## Global Constraints

- Allowlist, exactly: `bridge.status`, `bridge.peers`, `bridge.workspaces`, `bridge.workgroup_create`, `bridge.invite`, `bridge.join`, `bridge.device_retire`, `bridge.device_priority_set`, `embed.settings`, `embed.model_set`, `embed.device_set`, `embed.key_set`, `embed.key_clear`, `external_device.list`, `external_device.add`, `external_device.rotate`, `external_device.retire`, `external_device.restore`, `external_device.set_apps`, `config.schema`, `config.list`, `config.get`, `config.set`, `config.unset`, `bridge.restart`.
- Never relayed: `bridge.register`, `bridge.unregister`, `bridge.incoming`, `embed.info`, `embed.embed`, `embed.embed_images`, `external_device.authenticate` (they get `method not found`).
- `BRIDGE_NOT_RUNNING: i32 = -32001` in `sapphire-framework-bridge-api`; message `no sapphire-bridge is running`.
- Relayed params and results are never logged at any level; the relay logs only the method name and the outcome (`ok` or the error code).
- Relayed `bridge.restart` answers only once the app server holds a connection to the new bridge; after 30 s it fails with `the bridge did not come back within 30 s`.
- The app's CLI, absent app server: `no {app} server is running`, exit 1 (as `connect_app` prints today).
- No crate versions are bumped (release-plz does that). API crates (`bridge-api`, `server-api`) must not depend on 0.x framework crates.
- `sapphire-bridge`'s own CLI, `McpContext::bridge`, `keys` and `bridge_embedder` keep connecting to the bridge directly.

## Review Focus

1. A bridge restarted (or stopped) while the GUI polls: the next poll shows the bridge as not running, then up again, never a stale error — Task 5 test `a_bridge_that_stops_reads_absent_and_one_that_returns_reads_up` (reworked through the relay).
2. A relayed call whose connection drops mid-call answers `BRIDGE_NOT_RUNNING`, and the next call reconnects — Task 2 test `a_dropped_bridge_is_reconnected_on_the_next_call`.
3. `embed.key_set` / `config.set` of a secret: the key never appears in the app server's log — Task 2 test `relayed_params_are_never_logged`.
4. An older bridge without a method (e.g. `config.list` on a pre-#314 bridge): the client sees the bridge's `method not found`, not a relay error — Task 2 test `the_bridges_own_errors_come_back_unchanged`.
5. The app server absent: the GUI's bridge state says the app server is not running, not that the bridge is — Task 5 test `nothing_running_reads_absent_twice` (adjusted).

---

### Task 1: The relay's error code and a relayed `BridgeClient`

**Files:**
- Modify: `crates/sapphire-framework-bridge-api/src/lib.rs` (constants near `RESTART`)
- Modify: `crates/sapphire-framework-bridge-client/src/client.rs`
- Modify: `crates/sapphire-framework-server-api/Cargo.toml`, `crates/sapphire-framework-server-api/src/lib.rs`

**Interfaces:**
- Produces: `sapphire_bridge_api::BRIDGE_NOT_RUNNING: i32 = -32001`; `sapphire_bridge_api::RELAYED: &[&str]` (the allowlist, in the order of Global Constraints); `sapphire_bridge_api::NOT_RELAYED: &[&str]`; `BridgeClient::relayed(client: Arc<sapphire_ipc::Client>, runtime_dir: PathBuf) -> BridgeClient`; server-api `pub use sapphire_bridge_api as bridge;`.

- [ ] **Step 1: Write the failing tests** in `client.rs`'s test module (serve a `Router` over `Connection::pair()` with `sapphire_ipc::serve`):
  - `a_relayed_client_reads_bridge_not_running_as_not_running`: router answers `bridge.status` with `RpcError { code: BRIDGE_NOT_RUNNING, message: "no sapphire-bridge is running".into(), data: None }`; `BridgeClient::relayed(..).status().await` is `Err(sapphire_ipc::Error::NotRunning(m))` with `m == "no sapphire-bridge is running"`.
  - `a_direct_client_keeps_the_code_as_an_rpc_error`: same router, `BridgeClient::from_client` → `Err(Error::Rpc(e))` with `e.code == BRIDGE_NOT_RUNNING`.
  - In bridge-api's tests, `every_method_is_either_relayed_or_not`: the set of every `pub const` method name in the crate (list them literally in the test) equals `RELAYED ∪ NOT_RELAYED`, and the two are disjoint.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-bridge-client -p sapphire-framework-bridge-api` — expected: compile errors (missing items).
- [ ] **Step 3: Implement.** Add the constants (documented: `#![warn(missing_docs)]`). Add a private `relayed: bool` field to `BridgeClient`; route every typed call through one private `async fn call<P, R>(&self, method, params)` that maps `Error::Rpc(e) if self.relayed && e.code == BRIDGE_NOT_RUNNING` to `Error::NotRunning(e.message)`. In server-api add the bridge-api dependency (`version = "2.4.0"`, path) and the re-export with a doc line.
- [ ] **Step 4: Run** the same tests, plus `cargo clippy -p sapphire-framework-bridge-client -p sapphire-framework-bridge-api -p sapphire-framework-server-api --all-targets -- -D warnings` — expected: pass, clean.
- [ ] **Step 5: Commit** `feat(bridge-api): the relay's not-running code and a relayed BridgeClient (#314)`.

### Task 2: The relay in the app server

**Files:**
- Create: `crates/sapphire-framework-server/src/relay.rs`
- Modify: `crates/sapphire-framework-server/src/lib.rs` (builder field + `run` wiring after the `sync_router` block, before `extend`), `src/sync/mod.rs` (accessor), `src/sync/testing.rs` (StubBridge hooks), `Cargo.toml` (dev-dep `tracing-subscriber.workspace = true`)
- Test: `crates/sapphire-framework-server/tests/relay.rs`, `tests/relay_logging.rs`

**Interfaces:**
- Consumes: Task 1's `RELAYED`, `BRIDGE_NOT_RUNNING`.
- Produces:
  - `pub(crate) struct Relay` with `Relay::new(link: BridgeLink) -> Relay` and `fn routes(self: Arc<Self>, router: Router) -> Router` (registers every `RELAYED` method).
  - `pub(crate) enum BridgeLink { Sync(Arc<SyncRuntime>), Own(OwnLink) }`; `OwnLink { connector: BridgeConnector, current: tokio::sync::Mutex<Option<Arc<BridgeClient>>> }`; `async fn client(&self) -> Option<Arc<BridgeClient>>` (Sync: the slot's open client; Own: the cached open one, else one `connector.connect()` attempt).
  - `SyncRuntime::connected_bridge(&self) -> Option<Arc<BridgeClient>>` and `SyncRuntime::bridge_changes(&self) -> tokio::sync::watch::Receiver<Option<Arc<BridgeClient>>>` (pub(crate)).
  - `BridgeClient::call_raw(&self, method: &str, params: serde_json::Value) -> sapphire_ipc::Result<serde_json::Value>` (pub, in bridge-client; documented "for a relay").
  - `AppServer::bridge_connector(self, BridgeConnector) -> AppServer` (the Own link's connector; default `BridgeConnector::standard(ctx.app_name, version)`).
  - `StubBridge::answer(&self, method: &'static str, result: serde_json::Value)` and `StubBridge::calls(&self) -> Vec<(String, serde_json::Value)>` (methods the stub does not know answer from these canned results and are recorded).

- [ ] **Step 1: Write the failing tests** in `tests/relay.rs` (copy `sync_wiring.rs`'s `point_at`, `wait_until_listening`, `client_info`; a `BridgeConnector::new` over a `VecDeque` of `StubBridge::start_client_in` clients, like `sync/mod.rs`'s `connecting()`):
  - `an_allowlisted_method_returns_the_bridges_answer`: stub `answer("embed.settings", json!({"x": 1}))`; a client `call::<_, Value>("embed.settings", json!({"a": 2}))` returns `{"x":1}`, and `stub.calls()` contains `("embed.settings", {"a":2})`.
  - `a_method_off_the_allowlist_is_not_found`: `bridge.register` → `Error::Rpc(e)` with `e.code == codes::METHOD_NOT_FOUND`; the stub saw no call.
  - `no_bridge_answers_bridge_not_running`: connector always `Ok(None)` → `e.code == BRIDGE_NOT_RUNNING`, `e.message == "no sapphire-bridge is running"`.
  - `an_app_without_sync_relays_over_its_own_connection`: `AppServer` without `.sync(..)`, with `.bridge_connector(..)`; `bridge.status` returns the stub's status.
  - `a_dropped_bridge_is_reconnected_on_the_next_call`: first stub `disconnect()`; first call → `BRIDGE_NOT_RUNNING`; second call → the second stub's answer.
  - `the_bridges_own_errors_come_back_unchanged`: an allowlisted method the stub does not answer → `codes::METHOD_NOT_FOUND` with the stub's message.
  - `relayed_params_are_never_logged`, in its own test binary `tests/relay_logging.rs` (it installs a global subscriber): `tracing::subscriber::set_global_default` with a `tracing_subscriber::fmt` subscriber at `TRACE` writing into an `Arc<Mutex<Vec<u8>>>` (`MakeWriter`); relay `embed.key_set` with `{"key":"sk-SECRET-123"}`; the captured log contains `embed.key_set` and does not contain `sk-SECRET-123`.
  - With sync: `a_synced_server_relays_over_the_sync_connection`: `.sync(runtime)` with `StubBridge::start()`; `bridge.status` answers without any connector call.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-server --features test-util --test relay` — expected: compile failure.
- [ ] **Step 3: Implement.** Each relay handler: `link.client()`; none → `BRIDGE_NOT_RUNNING`; `call_raw(method, ctx.params)`; `Error::Rpc(e)` → `Err(e)`; `Error::Closed` (or any transport error) → drop the Own cache and answer `BRIDGE_NOT_RUNNING`; log `tracing::debug!(method, outcome)` only. Wire in `run`: `router = Arc::new(Relay::new(link)).routes(router)` after the sync block, before `server.info`/`extend`. `bridge.restart` is plain-forwarded here (Task 3 adds the wait).
- [ ] **Step 4: Run** the tests, `cargo test -p sapphire-framework-server --all-features`, clippy `-D warnings` — expected: pass.
- [ ] **Step 5: Commit** `feat(server): relay the bridge's client-facing methods (#314)`.

### Task 3: A relayed restart waits for the new bridge

**Files:**
- Modify: `crates/sapphire-framework-server/src/relay.rs`
- Test: `crates/sapphire-framework-server/tests/relay_restart.rs` (uses `tests/common/mod.rs`'s real-bridge `Host`)

**Interfaces:**
- Consumes: Task 2's `Relay`, `BridgeLink`, `SyncRuntime::bridge_changes`.
- Produces: `const RESTART_WAIT: Duration = Duration::from_secs(30)` in relay.rs.

- [ ] **Step 1: Write the failing tests:**
  - `a_relayed_restart_returns_once_the_new_bridge_is_connected`: start a host from `tests/common` (real bridge; the app server's sync runtime connects over `BridgeConnector::at`); call `bridge.restart` through the app server; it returns `Ok`, and a relayed `bridge.status` sent immediately after returns `Ok` (without the wait it answers `BRIDGE_NOT_RUNNING`).
  - `a_restart_that_never_comes_back_fails_after_the_wait`: an Own link whose connector returns one stub client, then `Ok(None)` forever; stub answers `bridge.restart` with `{}` then `disconnect()`s; with `RESTART_WAIT` overridable for tests (`#[cfg(any(test, feature = "test-util"))] Relay::restart_wait(Duration)` set to 300 ms via an `AppServer` test hook or by testing `Relay` directly), the call fails with message `the bridge did not come back within 30 s` (the message names 30 s regardless of the test override).
- [ ] **Step 2: Run** them — expected: the first fails (status right after restart answers `BRIDGE_NOT_RUNNING`), the second fails (returns Ok).
- [ ] **Step 3: Implement** in the `bridge.restart` handler: remember the client used; forward; on `Ok`, wait (`tokio::time::timeout(RESTART_WAIT, ..)`) until `old.closed()` and then a different open client is available — Sync: loop on `bridge_changes().changed()` until `Some(c)` with `!Arc::ptr_eq(&c, &old) && !c.is_closed()`; Own: after `old.closed()`, retry `connector.connect()` every 100 ms. Timeout → `RpcError::internal("the bridge did not come back within 30 s")`. Return the bridge's `Ack`.
- [ ] **Step 4: Run** the tests and the server crate's suite — expected: pass.
- [ ] **Step 5: Commit** `feat(server): a relayed bridge.restart returns once the new bridge is connected (#314)`.

### Task 4: The app's CLI goes through the app server

**Files:**
- Modify: `crates/sapphire-framework-server/src/command.rs` (`connect_running` :477, `WorkgroupCommand::dispatch` :305, `DeviceCommand::dispatch` ~:381, `embedding` :484, `external_device` :496, `workspace_show` :806, `FrameworkCommand::dispatch` :107-111)
- Test: `crates/sapphire-framework-server/tests/cli_bridge.rs`

**Interfaces:**
- Consumes: Task 1's `BridgeClient::relayed`; Task 2's relay.
- Produces: `WorkgroupCommand::dispatch(self, app: &'static str, version: &'static str) -> Result<i32>`; `DeviceCommand::dispatch(self, app: &'static str, version: &'static str) -> Result<i32>`; private `async fn relayed_bridge(app: &str, version: &str) -> Result<Option<BridgeClient>>` (replaces `connect_running`: `connect_app` then `BridgeClient::relayed(Arc::new(client), runtime_dir()?)`; `None` after `connect_app` printed `no {app} server is running`).

- [ ] **Step 1: Rework the tests** in `cli_bridge.rs`: each starts a real bridge (as now) **and** an `AppServer` for a test app at `Endpoint::for_app(APP)` under `RUNTIME_DIR_ENV`, with `.bridge_connector(BridgeConnector::at(control, "test", "0.0.0"))`; `WorkgroupCommand::Create{..}.dispatch(APP, "0.0.0")` returns 0 and the bridge has the workgroup; `device_retire_own_device_fails` still errors with "own device". Add `bridge_commands_without_an_app_server_say_so`: no app server → `WorkgroupCommand::List.dispatch(APP, ..)` returns `Ok(1)`. Add `workspace_show_lists_the_workgroup_through_the_app_server` asserting the rendered output contains the workgroup section from a single app connection (no bridge endpoint probe: run it with the bridge's control endpoint absent from the runtime dir by giving the bridge a custom control endpoint elsewhere).
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-server --all-features --test cli_bridge` — expected: compile errors (signature) / failures.
- [ ] **Step 3: Implement** as in Interfaces; `FrameworkCommand::dispatch` passes `server.app_name()`; `embedding` and `external_device` use `relayed_bridge`; `workspace_show` reuses its app connection for `BridgeClient::relayed(..).workspaces()`, keeping the "no sapphire-bridge is running" line when that returns `NotRunning`.
- [ ] **Step 4: Run** the CLI tests and the crate's suite; clippy — expected: pass.
- [ ] **Step 5: Commit** `feat(server)!: the app's bridge commands go through the app server (#314)`.

### Task 5: The GUI keeps one connection

**Files:**
- Modify: `crates/sapphire-framework-gui/src/client/{conn.rs,exec.rs,types.rs,mod.rs}`, `src/app/mod.rs` (`AppSetup`), `src/test_support.rs`, `src/views/banner.rs` (only if the copy needs the app name)
- Test: `crates/sapphire-framework-gui/tests/client.rs`, `tests/in_process.rs`, `exec.rs` unit tests

**Interfaces:**
- Consumes: Task 1's `BridgeClient::relayed`; Task 2's relay.
- Produces: `Endpoints { pub app: Endpoint }` (field `bridge` removed; `default_for` unchanged otherwise); `Connections { app: Option<Client> }` with `pub(crate) fn bridge(&self, cfg) -> Option<BridgeClient>` built from the cached app connection via `BridgeClient::relayed`.

- [ ] **Step 1: Rework the tests:** remove `bridge:` from every `Endpoints` literal; in `tests/client.rs` start the server with `.bridge_connector(BridgeConnector::at(<bridge control>, "test", "0.0.0"))` (or its sync runtime over that endpoint) and keep the bridge's control endpoint in the fixture, not in `Endpoints`. Keep `create_invite_and_retire_round_trip`, `workgroup_join_pairs_with_another_bridge`, `a_bridge_that_stops_reads_absent_and_one_that_returns_reads_up` passing through the app server. Adjust `nothing_running_reads_absent_twice`: with no app server, `snapshot.bridge` is `Conn::Error(m)` with `m` containing `server is not running`, and `snapshot.server` is `Conn::Absent`. `a_process_that_never_answers_reads_error_and_does_not_stall_the_loop` hangs the app endpoint instead of the bridge's. `in_process.rs` keeps its bridge endpoint as a local.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui --all-features` — expected: compile errors.
- [ ] **Step 3: Implement:** `fetch_bridge` runs over the app connection (absent app → `Conn::Error(format!("the {} server is not running", cfg.app.app_name))`); its `classify` keeps working because `relayed` already turns `BRIDGE_NOT_RUNNING` into `NotRunning`; `bridge_command` uses the relayed client and keeps the message `sapphire-bridge is not running` for `NotRunning`; on errors drop the app connection (one connection now). Remove `BRIDGE_NAME` from `types.rs` imports.
- [ ] **Step 4: Run** the GUI tests, `cargo clippy -p sapphire-framework-gui --all-features --all-targets -- -D warnings` — expected: pass.
- [ ] **Step 5: Commit** `feat(gui)!: reach the bridge through the app server (#314)`.

### Task 6: Docs and changelog

**Files:**
- Modify: `CHANGELOG.md` (`## [Unreleased]`), `docs/ARCHITECTURE.md` (the process architecture / client sections around :126 and :155)

- [ ] **Step 1: Write the entries.** Added: the relay (allowlist summary, `BRIDGE_NOT_RUNNING`, `BridgeClient::relayed`, `AppServer::bridge_connector`, `sapphire-framework-server-api` re-exports bridge-api as `bridge`). Changed, **Breaking**: the app's `workgroup` / `device` / `embedding` / `external-device` need the app server running; `WorkgroupCommand::dispatch` and `DeviceCommand::dispatch` take the app name; `Endpoints` loses `bridge`; server-api now depends on bridge-api (its major follows bridge-api's).
- [ ] **Step 2: Update ARCHITECTURE.md** to say clients talk only to their app server, which relays the listed bridge methods; `sapphire-bridge`'s CLI is the exception.
- [ ] **Step 3: Run** `cargo test --workspace --all-features` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` — expected: pass. (Skip `cargo publish --dry-run`: it verifies against crates.io's bridge-api, which lacks the new items until the release PR.)
- [ ] **Step 4: Commit** `docs: clients reach the bridge through their app server (#314)`.
