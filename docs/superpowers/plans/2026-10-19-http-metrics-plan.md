# Prometheus Metrics on the HTTP Listener — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Serve Prometheus metrics at `GET /metrics` on the app server's HTTP listener when `[http.metrics] enabled = true`. The same change moves `[mcp] http` to `[http.mcp] enabled`.

**Architecture:** The `metrics` facade is always a dependency of `sapphire-framework-server`, so instrumentation sites need no `#[cfg]`. Feature `metrics` adds the Prometheus recorder (installed once per process), `metrics-process`, and an unauthenticated `/metrics` route. `http::start` serves a public router first and hands everything else to the token-protected router. State gauges are refreshed on each scrape. Event metrics are recorded where the events happen. Applications add their own metrics through a re-exported facade and `AppServer::metrics_collector`.

**Tech Stack:** Rust 2024, axum 0.8, `metrics` 0.24, `metrics-exporter-prometheus` 0.18 (`default-features = false`), `metrics-process` 2.4, tokio, reqwest (tests).

**Spec:** `docs/superpowers/specs/2026-10-19-http-metrics-design.md`

## Global Constraints

- Work in the worktree `/home/fluo10/Documents/Dev/project-sapphire/sapphire-framework-wt-metrics` on branch `feat/http-metrics`. Build with `CARGO_TARGET_DIR=/home/fluo10/Documents/Dev/project-sapphire/sapphire-framework-wt-metrics/target`, because `/tmp` is RAM-backed.
- Stage files by name. Never use `git add -A` or `git stash`.
- Every framework metric name starts with `sapphire_`.
- Labels take only `app`, `version` and `result` (`"ok"` / `"error"`). No metric or label carries a workspace id, device id, path, workspace or device name, or error text.
- `/metrics` is never behind the token and never behind `allowed_hosts`.
- Response content type: `text/plain; version=0.0.4`.
- `[http.mcp] enabled` and `[http.metrics] enabled` both default to `false`. `listen` and `allowed_hosts` keep their names.
- An old `[mcp]` table fails to load with `<path>: [mcp] http moved to [http.mcp] enabled`.
- A metrics failure (recorder conflict, bind failure) never stops the server. Log it and carry on.
- `metrics-exporter-prometheus` uses `default-features = false`. No hyper server, no push gateway.
- Exactly one `metrics` version in the tree (`cargo tree -d` shows no duplicate `metrics`).
- Comments and rustdoc match the surrounding code: full sentences, explaining why, with British spellings as the code already uses.

## Review Focus

1. **`/metrics` with an `Authorization` header or a wrong token** should still answer 200: the public route ignores tokens. Task 2 adds `metrics_ignores_a_bad_token`.
2. **A protected route reached through the fallback** must still need a token after the router split: `/mcp` and app routes answer 401 without one. Task 2 adds `app_routes_stay_behind_the_token`, and `tests/mcp.rs` keeps passing.
3. **Metrics on with nothing else served and no address** should log the existing "nothing to bind HTTP to" error, not panic. Task 2 adds `metrics_alone_starts_the_listener` for the positive case. The no-address case is covered by the existing `match` arm, so the reviewer checks that arm.
4. **A scrape before any workspace is selected** must render, with `sapphire_workspace_selected 0` and no embedding gauges. Task 3 adds `state_before_a_workspace_is_selected`.
5. **Identifiers leaking through a label** (an app collector excepted): Task 3 adds `output_names_no_path_or_id`, which looks for the root path and the workspace id after a sync-less init.

---

### Task 1: Move `[mcp] http` to `[http.mcp] enabled` and add `[http.metrics]`

**Files:**
- Modify: `crates/sapphire-framework-server/src/config.rs` (types, module doc, `load`, tests)
- Modify: `crates/sapphire-framework-server/src/lib.rs:60` (re-exports), `:451`, `:567`, `:592` (uses)
- Modify: `crates/sapphire-framework-server/tests/mcp.rs:17`, `:283-290`
- Modify: `docs/ARCHITECTURE.md:334`, `CHANGELOG.md` (`## [Unreleased]` → `### Changed`)

**Interfaces:**
- Produces:
  - `pub struct HttpConfig { listen: Option<SocketAddr>, allowed_hosts: Vec<String>, #[serde(default)] mcp: HttpMcpConfig, #[serde(default)] metrics: HttpMetricsConfig }`
  - `pub struct HttpMcpConfig { #[serde(default)] pub enabled: bool }`
  - `pub struct HttpMetricsConfig { #[serde(default)] pub enabled: bool }`

  All three derive `Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize` with `deny_unknown_fields`. `mcp` and `metrics` use `skip_serializing_if` on their default. `ServerConfig` loses `mcp`, and `McpConfig` is deleted. `lib.rs` re-exports `HttpMcpConfig` and `HttpMetricsConfig` in its place.

- [ ] **Step 1: Write the failing tests in `config.rs`'s `mod tests`**

  Rewrite the existing `a_missing_file_is_all_defaults_and_mcp_is_off` and `the_file_turns_mcp_on_and_names_the_listener` for the new layout, and add:

```rust
#[test]
fn the_file_turns_metrics_on_under_http() {
    // "[http]\nlisten = \"127.0.0.1:3173\"\n\n[http.metrics]\nenabled = true\n"
    assert!(config.http.metrics.enabled);
    assert!(!config.http.mcp.enabled);
}

#[test]
fn an_old_mcp_table_says_where_the_setting_went() {
    // "[mcp]\nhttp = true\n"
    let err = ServerConfig::load(&path).unwrap_err().to_string();
    assert!(err.contains("[mcp] http moved to [http.mcp] enabled"), "{err}");
}
```

  The existing typo test (`htpp`) is rewritten to put the typo under `[http.mcp]` (`"[http.mcp]\nenabeld = true\n"`) and must still fail.

- [ ] **Step 2: Run them and confirm they fail**

  Run: `cargo test -p sapphire-framework-server --lib config::`
  Expected: compile errors (`no field mcp on HttpConfig`).

- [ ] **Step 3: Implement the types, and the `[mcp]` check in `ServerConfig::load`**

  Before `toml::from_str::<ServerConfig>`, parse the text as `toml::Table`. If it has a top-level `mcp` key, return `Error::Config(format!("{}: [mcp] http moved to [http.mcp] enabled", path.display()))`. A text that does not parse as a table falls through to the existing error. Update the module doc's TOML example to the spec's layout. In `lib.rs`, replace `config.mcp.http` with `config.http.mcp.enabled`. In `tests/mcp.rs`, build the config as `HttpConfig { listen, allowed_hosts, mcp: HttpMcpConfig { enabled: true }, ..HttpConfig::default() }`.

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p sapphire-framework-server --lib config:: && cargo test -p sapphire-framework-server --features mcp --test mcp`
  Expected: all pass.

- [ ] **Step 5: Update the docs**

  In `ARCHITECTURE.md:334`, `[mcp] http = true` becomes `[http.mcp] enabled = true`. Under `### Changed` in `CHANGELOG.md`, add one bullet: "**Breaking:** `server.toml`'s `[mcp] http` moved to `[http.mcp] enabled`; a file that still has `[mcp]` fails to load and says where the setting went. `ServerConfig::mcp` and `McpConfig` are gone; `HttpConfig` gains `mcp: HttpMcpConfig` and `metrics: HttpMetricsConfig`."

- [ ] **Step 6: Commit**

```bash
git add crates/sapphire-framework-server/src/config.rs crates/sapphire-framework-server/src/lib.rs crates/sapphire-framework-server/tests/mcp.rs docs/ARCHITECTURE.md CHANGELOG.md
git commit -m "feat!: move [mcp] http to [http.mcp] enabled in server.toml"
```

---

### Task 2: Serve `/metrics` outside the token (feature `metrics`)

**Files:**
- Modify: `Cargo.toml` (workspace deps: `metrics = "0.24"`, `metrics-exporter-prometheus = { version = "0.18", default-features = false }`, `metrics-process = "2.4"`)
- Modify: `crates/sapphire-framework-server/Cargo.toml` (deps, feature `metrics`, `[[test]] name = "metrics" required-features = ["metrics"]`)
- Modify: `crates/sapphire-framework-server/src/http.rs` (`start` signature)
- Create: `crates/sapphire-framework-server/src/exporter.rs` (`#[cfg(feature = "metrics")]`)
- Modify: `crates/sapphire-framework-server/src/lib.rs` (module decl, wiring in `run`, warning without the feature)
- Modify: `crates/sapphire-framework/Cargo.toml` (feature `metrics = ["http", "sapphire-framework-server/metrics"]`), `crates/sapphire-framework/src/lib.rs` (features table row)
- Create: `crates/sapphire-framework-server/tests/metrics.rs`

**Interfaces:**
- Produces:
  - `http::start(addr: SocketAddr, protected: Option<axum::Router>, public: Option<axum::Router>, auth: AuthConfig, stop: CancellationToken) -> Option<(SocketAddr, JoinHandle<()>)>`. The served app is `public.unwrap_or_default().fallback_service(protect(auth, protected))` when both exist. With only one of them, serve that one alone (`protect`ed if it is the protected one). Use `fallback_service`, not `merge`, so the two routers' fallbacks never collide.
  - `exporter::install() -> Option<&'static PrometheusHandle>`. A `static OnceLock<Option<PrometheusHandle>>` is initialised with `PrometheusBuilder::new().install_recorder()`. On `Err`, it logs `tracing::error!("metrics are off: another metrics recorder is installed in this process: {err}")` and stores `None`. On first success it calls `metrics_process::Collector::default().describe()` and `instrument`'s describe (Task 3 adds the latter).
  - `pub(crate) struct Scrape { process: metrics_process::Collector }`. Tasks 3 and 5 add fields.
  - `exporter::routes(handle: &'static PrometheusHandle, scrape: Arc<Scrape>) -> axum::Router`, which serves `GET /metrics`. The handler runs `scrape.process.collect()` and `handle.run_upkeep()`, then returns `handle.render()` with the content type from Global Constraints.
  - Wherever the server builds its HTTP routes, when `config.http.metrics.enabled`: call `install()`, set `gauge!("sapphire_build_info", "app" => ctx.app_name, "version" => version).set(1.0)`, and pass `routes(...)` as `public`. `None` from `install()` means no public router.
  - Without the feature: `#[cfg(not(feature = "metrics"))] if config.http.metrics.enabled { tracing::warn!("server.toml turns metrics on, but this build of {} has no metrics", ctx.app_name) }`, beside the MCP warning.
  - The bind `match` binds when either router is `Some`.

- [ ] **Step 1: Write the failing tests in `tests/metrics.rs`**

  Copy `point_at`, `free_port`, `wait_until_listening`, `start`, `client`, `stop`, `Fake` and `TOKEN` from `tests/mcp.rs`. Each test gets its own `static CTX_*: AppContext`. A helper `metrics_config(port, enabled) -> ServerConfig` sets `listen` and `http.metrics.enabled`. Tests:

```rust
// metrics on, no other routes: the listener starts for /metrics alone
async fn metrics_alone_starts_the_listener()
    // GET /metrics without a token → 200, content-type starts with "text/plain"
    // body contains "sapphire_build_info{app=\"sapphire-metricstest-alone\",version=\"0.0.0\"} 1"
    // body contains "process_" on Linux (#[cfg(target_os = "linux")] assert)

async fn metrics_ignores_a_bad_token()
    // GET /metrics with bearer "nope" → 200

async fn app_routes_stay_behind_the_token()
    // .http(Router::new().route("/app", get(|| async { "app" }))) + metrics on
    // GET /app without a token → 401; with TOKEN → 200 "app"; GET /metrics → 200

async fn metrics_off_serves_no_route()
    // .http(/app route), metrics off; GET /metrics with TOKEN → 404
```

- [ ] **Step 2: Run them and confirm they fail**

  Run: `cargo test -p sapphire-framework-server --features metrics --test metrics`
  Expected: build error (feature `metrics` unknown).

- [ ] **Step 3: Add the dependencies and the feature, then implement `exporter.rs`, the `http::start` split and the `run` wiring as in Interfaces**

  The feature line in the server's `Cargo.toml` gets a comment matching its neighbours: `# Prometheus metrics at /metrics on the HTTP listener when server.toml turns them on; the one route not behind a token.` Then `metrics = ["http", "dep:metrics-exporter-prometheus", "dep:metrics-process"]`. `metrics` (the facade) is a plain, non-optional dependency.

- [ ] **Step 4: Run the tests and confirm they pass, along with the existing HTTP tests**

  Run: `cargo test -p sapphire-framework-server --features metrics,mcp --test metrics --test mcp`
  Expected: all pass.

- [ ] **Step 5: Check the builds without the feature, and the dependency tree**

  Run:
  - `cargo check -p sapphire-framework-server --no-default-features`
  - `cargo check -p sapphire-framework-server --features http`
  - `cargo check -p sapphire-framework --features metrics`
  - `cargo tree -p sapphire-framework-server --features metrics -d | grep -w metrics`
  - `cargo tree -p sapphire-framework-server --features metrics -i hyper`

  Expected: the checks pass. The duplicate grep prints nothing. `hyper` is reached only through axum or reqwest, never through `metrics-exporter-prometheus`.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/sapphire-framework-server/Cargo.toml crates/sapphire-framework-server/src/http.rs crates/sapphire-framework-server/src/exporter.rs crates/sapphire-framework-server/src/lib.rs crates/sapphire-framework-server/tests/metrics.rs crates/sapphire-framework/Cargo.toml crates/sapphire-framework/src/lib.rs
git commit -m "feat: serve Prometheus metrics at /metrics when server.toml turns them on"
```

---

### Task 3: State gauges refreshed on each scrape

**Files:**
- Create: `crates/sapphire-framework-server/src/instrument.rs` (always compiled. It uses the `metrics` facade only, and holds the name constants, `describe()` and the recording helpers for Tasks 3–4)
- Modify: `crates/sapphire-framework-server/src/exporter.rs` (`Scrape` fields, handler)
- Modify: `crates/sapphire-framework-server/src/lib.rs` (pass `current` and `update_status` into `Scrape`)
- Test: `crates/sapphire-framework-server/tests/metrics.rs`

**Interfaces:**
- Consumes: `Scrape`, `exporter::routes` (Task 2); `Current::status() -> proto::SyncStatusResult`, `Current::root() -> Option<PathBuf>`, `SyncRuntime::live_session_devices(&Path) -> Vec<GrainId>`, `update::SharedStatus` (`Arc<Mutex<UpdateStatus>>`)
- Produces:
  - `pub(crate) fn instrument::describe()`: `describe_gauge!` / `describe_counter!` / `describe_histogram!` for every name in the spec's tables, with one-line help texts taken from the spec's "Value" / "What" column. `exporter::install` calls it.
  - `pub(crate) async fn instrument::refresh_state(current: &Current, update: &update::SharedStatus)` sets the following:

    | gauge | value |
    |---|---|
    | `sapphire_workspace_selected` | `root().is_some()` |
    | `sapphire_sync_enabled` | `enabled` |
    | `sapphire_sync_bridge_available` | `bridge_available` |
    | `sapphire_sync_paused` | `paused.is_some()` |
    | `sapphire_sync_error` | `last_error.is_some()` |
    | `sapphire_sync_peers` | `peers` |
    | `sapphire_sync_live_sessions` | `live_session_devices(root).len()` when there is a sync runtime and a root, else `0` |
    | `sapphire_embedding_vectors` / `_pending` / `_running` | only when `embedding` is `Some` |
    | `sapphire_update_available` | only when `enabled && latest.is_some()` |

    Booleans are `1.0` / `0.0`. If `Current` does not expose its runtime, add `pub(crate) fn sync(&self) -> Option<&Arc<SyncRuntime>>`.
  - `Scrape` gains `current: Arc<Current>` and `update: update::SharedStatus`. The handler awaits `refresh_state` before collecting process metrics.

- [ ] **Step 1: Write the failing tests**

```rust
async fn state_before_a_workspace_is_selected()
    // body contains "sapphire_workspace_selected 0" and "sapphire_sync_enabled 0"
    // body does not contain "sapphire_embedding_"

async fn state_after_a_workspace_is_selected()
    // workspace.init on tmp/ws via IPC, then scrape: "sapphire_workspace_selected 1"
    // body contains "# HELP sapphire_sync_peers"

async fn output_names_no_path_or_id()
    // after init: body does not contain tmp.path().to_str()
    // body does not contain the marker's sync id if one exists (read <ws>/.<app>/<SYNC_ID_FILE> when present; `SYNC_ID_FILE` in current.rs)
```

- [ ] **Step 2: Run them and confirm they fail**

  Run: `cargo test -p sapphire-framework-server --features metrics --test metrics state_ output_`
  Expected: FAIL. The gauges are missing from the body.

- [ ] **Step 3: Implement `instrument.rs` and the `Scrape` fields as in Interfaces**

- [ ] **Step 4: Run the tests and confirm they pass**

  Run: `cargo test -p sapphire-framework-server --features metrics --test metrics`
  Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/sapphire-framework-server/src/instrument.rs crates/sapphire-framework-server/src/exporter.rs crates/sapphire-framework-server/src/lib.rs crates/sapphire-framework-server/src/current.rs crates/sapphire-framework-server/tests/metrics.rs
git commit -m "feat: report workspace, sync, embedding and update state as gauges"
```

---

### Task 4: Event metrics — IPC connections, re-indexing, sync sessions

**Files:**
- Modify: `crates/sapphire-framework-server/src/instrument.rs`
- Modify: `crates/sapphire-framework-server/src/lib.rs` (accept loop, about `:705`)
- Modify: `crates/sapphire-framework-server/src/handlers.rs` (`proto::REINDEX`, about `:173`)
- Modify: `crates/sapphire-framework-server/src/current.rs` (the moved workspace's re-index, about `:162`)
- Modify: `crates/sapphire-framework-server/src/sync/mod.rs` (`reindex` `:857`, inbound session `:1205-1228`, `adopt_live_session` `:1609`)
- Test: `crates/sapphire-framework-server/tests/metrics.rs`

**Interfaces:**
- Produces, in `instrument`:
  - `pub(crate) struct IpcConnection`. `IpcConnection::open() -> IpcConnection` increments `sapphire_ipc_connections_total` and `sapphire_ipc_connections`. `Drop` decrements `sapphire_ipc_connections`. The accept loop creates one and moves it into the spawned `serve` task. The in-process `/mcp` connection is not counted.
  - `pub(crate) fn reindexed(started: std::time::Instant, ok: bool)` increments `sapphire_reindex_total{result}` and records `sapphire_reindex_duration_seconds` with `started.elapsed().as_secs_f64()`. Call sites:
    - `SyncRuntime::reindex`: `started` is taken after the `reindexing` lock is acquired. A `backend_at` failure counts as `false`, and so do both `Err` arms.
    - The `REINDEX` handler, around `backend.sync()`.
    - The `current.rs` moved-workspace spawn, around `backend.sync()`.
  - `pub(crate) fn sync_session(ok: bool)` increments `sapphire_sync_sessions_total{result}`.
    - `ok = true` right after `table.catch_up(...)`, both inbound and in `adopt_live_session`.
    - `ok = false` where `open_live_session` fails, both inbound (the `Err` arm) and in `adopt_live_session` (before `map_err` returns).
    - A `table.insert` that returns `None` (the tie-break) is not counted.
  - `result` label values come from one `fn result(ok: bool) -> &'static str`.

- [ ] **Step 1: Write the failing tests**

```rust
async fn ipc_connections_are_counted()
    // open an IPC client, then scrape: a line starting "sapphire_ipc_connections_total " with value >= 1
    // a line starting "sapphire_ipc_connections " with value >= 1

async fn reindexing_is_counted_and_timed()
    // workspace.init, then call proto::REINDEX over IPC, then scrape:
    // "sapphire_reindex_total{result=\"ok\"}" with value >= 1
    // body contains "sapphire_reindex_duration_seconds_count"
```

  Add a helper `fn value(body: &str, prefix: &str) -> f64` that finds the line starting with `prefix` followed by a space and parses its last field. Session counting needs two bridges, so no test here. The reviewer checks the four call sites against the Interfaces list, and `# HELP sapphire_sync_sessions_total` appears in Task 3's body via `describe()`.

- [ ] **Step 2: Run them and confirm they fail**

  Run: `cargo test -p sapphire-framework-server --features metrics --test metrics counted`
  Expected: FAIL. The lines are missing.

- [ ] **Step 3: Implement the helpers and the call sites as in Interfaces**

- [ ] **Step 4: Run the server's whole suite, since the sync code changed**

  Run: `cargo test -p sapphire-framework-server --features metrics,mcp`
  Expected: all pass, including `live`, `converge`, `star` and `vectors`.

- [ ] **Step 5: Commit**

```bash
git add crates/sapphire-framework-server/src/instrument.rs crates/sapphire-framework-server/src/lib.rs crates/sapphire-framework-server/src/handlers.rs crates/sapphire-framework-server/src/current.rs crates/sapphire-framework-server/src/sync/mod.rs crates/sapphire-framework-server/tests/metrics.rs
git commit -m "feat: count IPC connections, re-index runs and sync sessions"
```

---

### Task 5: Application extension — re-exported facade and `AppServer::metrics_collector`

**Files:**
- Modify: `crates/sapphire-framework-server/src/lib.rs` (field, builder method, destructure in `run`, pass to `Scrape`, `pub use metrics;`)
- Modify: `crates/sapphire-framework-server/src/exporter.rs` (`Scrape.collectors`)
- Modify: `crates/sapphire-framework/src/lib.rs` (`#[cfg(feature = "server")] pub use sapphire_framework_server::metrics;`)
- Modify: `CHANGELOG.md` (`### Added`), `docs/ARCHITECTURE.md` (one paragraph beside the `/mcp` one at `:334`)
- Test: `crates/sapphire-framework-server/tests/metrics.rs`

**Interfaces:**
- Consumes: `Scrape` (Tasks 2–3)
- Produces:
  - `AppServer` field `metrics_collectors: Vec<Arc<dyn Fn() + Send + Sync>>`. It is not cfg-gated: a build without the feature keeps the collectors and never calls them.
  - `pub fn metrics_collector(mut self, collect: Arc<dyn Fn() + Send + Sync>) -> AppServer` pushes onto that `Vec`. Its rustdoc says:
    - it runs on every scrape of `/metrics`, in registration order;
    - set state gauges with `sapphire_framework::metrics::gauge!`;
    - prefix names with the application's own (`sapphire_journal_entries`);
    - labels carry no identifiers, paths or names.
  - `sapphire_framework_server::metrics` is the `metrics` crate re-exported (`pub use metrics;`), and `sapphire_framework::metrics` is the same.
  - `Scrape.collectors: Vec<Arc<dyn Fn() + Send + Sync>>`. The handler calls each one after `refresh_state` and before rendering.

- [ ] **Step 1: Write the failing test**

```rust
async fn an_application_collector_runs_on_each_scrape()
    // let n = Arc<AtomicU64>; collector: n += 1; metrics::gauge!("sapphire_metricstest_scrapes").set(n as f64)
    // scrape twice: second body contains "sapphire_metricstest_scrapes 2"
```

- [ ] **Step 2: Run it and confirm it fails**

  Run: `cargo test -p sapphire-framework-server --features metrics --test metrics an_application_collector`
  Expected: compile error (`no method metrics_collector`).

- [ ] **Step 3: Implement as in Interfaces, then write the docs**

  - `CHANGELOG.md`, under `### Added`: "Prometheus metrics: `[http.metrics] enabled = true` in `server.toml` serves `GET /metrics` on the HTTP listener (feature `metrics`), the one route not behind an external device's token. It reports process metrics, `sapphire_build_info`, workspace/sync/embedding/update state, IPC connections, re-index runs and sync sessions, and carries no identifiers. Applications add their own with `sapphire_framework::metrics` and `AppServer::metrics_collector`."
  - `ARCHITECTURE.md`: a short paragraph in Japanese, matching that file, covering the same points.

- [ ] **Step 4: Run the full verification**

  Run:
  - `cargo test -p sapphire-framework-server --features metrics,mcp`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo fmt --all --check`
  - `cargo deny check` (when installed)

  Expected: all pass. No new licences are flagged. `metrics*` crates are MIT.

- [ ] **Step 5: Commit**

```bash
git add crates/sapphire-framework-server/src/lib.rs crates/sapphire-framework-server/src/exporter.rs crates/sapphire-framework/src/lib.rs crates/sapphire-framework-server/tests/metrics.rs CHANGELOG.md docs/ARCHITECTURE.md
git commit -m "feat: let applications add metrics of their own"
```
