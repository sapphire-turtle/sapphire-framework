# App Settings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apps register settings with the framework; the app server answers a merged `config.*` (its keys and the bridge's), restarts itself in place with `server.restart`, and `<app> config` edits everything, offline included.

**Architecture:** A `settings` module in `sapphire-framework-server` builds one `Registry` from a framework owner (`http.*`, `update.check`) and the app's owner, with files under `<config_dir>/config.toml` and `{root}/.{app}/config.toml`. A `config` module answers `config.*`, splitting batches between itself and the bridge (checked first with `dry_run`). `serve_restarting(owner, build)` binds the IPC listener once and runs a fresh `AppServer` from `build` on each restart.

**Tech Stack:** Rust 2024, tokio, `sapphire-framework-settings` (toml_edit), `sapphire-framework-ipc`, clap.

**Spec:** `docs/superpowers/specs/2026-10-22-app-settings-design.md`

## Global Constraints

- Framework keys, all `Apply::Restart`, scopes `[Device]`, section `"Server"`: `http.listen` (SocketAddr), `http.allowed_hosts` (List of String), `http.mcp.enabled` (Bool, default false), `http.metrics.enabled` (Bool, default false), `update.check` (Bool, default true).
- App keys start with `<app_name>.`; scopes only `Device` and `Workspace`; secrets `Device` only.
- Reserved top-level names: `http`, `update`, `sync`, `retrieve`, `workspace` (framework), `<app_name>` (app), `net`, `embed` (bridge; no owner here).
- Files: device `<config_dir>/config.toml`; workspace `{root}/.{app}/config.toml`; secrets `<config_dir>/secrets/<key>` (0600).
- Messages, verbatim: `no workspace is selected`; `unknown setting {key} (or sapphire-bridge, which may know it, is not running)`; `this sapphire-bridge is too old to check settings without writing them; update it`; `this server cannot restart itself; restart its service`; `sapphire-bridge is not running; its settings are not shown`; `the {app} server is not running; start it, or use sapphire-bridge config for {key}`; `restart required: sapphire-bridge`; `restart required: {app} server`.
- Migration: `server.toml` → `config.toml` (text unchanged), old file renamed `server.toml.v1`, only when `config.toml` does not exist. The `[mcp]` table keeps today's error.
- Setting values (and secrets) are never logged, never accepted on the command line for secrets, never returned for secrets.
- No crate version bumps (release-plz). API crates (`bridge-api`, `server-api`) depend on no 0.x framework crate.
- `serde(default)` on every new wire field, so older peers interoperate.

## Review Focus

1. A restart while the sync runtime holds the replica store: the new server must open it, not fail with "Database already open" — Task 5 test `a_restart_with_sync_reopens_the_replica_store`.
2. A `config.toml` that does not parse: the server refuses to start and `<app> config` offline refuses to edit it, both with an error naming the file, so the user knows which file to fix by hand — Task 3 test `an_unparsable_config_names_the_file` and Task 6 test `offline_config_reports_an_unparsable_file`.
3. Two `config.set` calls racing on the app server: each is all-or-nothing and neither loses the other's keys — Task 4 test `concurrent_writes_both_land` (writes serialized by a mutex).
4. A workspace-scope write while the selection changes: the file of the workspace selected at the time of the call is written — Task 3 test `the_workspace_scope_follows_the_selection`.
5. `server.restart` while a client holds a connection: the client sees its connection close and can reconnect to the same endpoint — Task 5 test `a_client_reconnects_after_a_restart`.

---

### Task 1: Wire and crate additions (bridge-api, settings)

**Files:**
- Modify: `crates/sapphire-framework-bridge-api/src/settings.rs` (`Process`, `SettingDef.process`), `src/config.rs` (`dry_run`), `src/lib.rs` (`StatusResult.config_dry_run`, lists), `src/cli.rs` (`SettingDef` literals if any)
- Modify: `crates/sapphire-framework-settings/src/store.rs` (`check`), `src/lib.rs` (export), `src/owner.rs` (test helper literal)
- Modify: every `SettingDef { .. }` literal (`crates/sapphire-framework-bridge/src/settings.rs`)

**Interfaces:**
- Produces:
  - `pub enum Process { #[default] Bridge, App }` (serde `snake_case`, `Default`), `SettingDef.process: Process` with `#[serde(default)]`.
  - `ConfigSetParams.dry_run: bool`, `ConfigUnsetParams.dry_run: bool`, both `#[serde(default)]`.
  - `StatusResult.config_dry_run: bool` `#[serde(default)]`.
  - `RELAYED` loses the five `config.*`; new `pub const ANSWERED_BY_APP_SERVER: &[&str]` holds them (the partition test asserts the three lists split every method).
  - `sapphire_framework_settings::check(reg: &Registry, scope: Option<Scope>, ops: Vec<Op>) -> Result<Applied>` — everything `apply` does up to and including the owners' `validate`, writing nothing; `apply` = `check` then write (one shared private function, no duplicated validation).

- [ ] **Step 1: Failing tests.** settings crate: `check_writes_nothing_and_reports_what_apply_would` (an `Op::Set` on a `TestOwner`: `check` returns the same `Applied` as a later `apply`, and the file does not exist after `check`); `check_reports_the_same_errors` (a wrong kind fails in both with equal `Error` display). bridge-api: `an_older_def_reads_as_the_bridges` (JSON without `process` → `Process::Bridge`); `dry_run_defaults_to_false`; update `every_method_is_either_relayed_or_not` to the three-way split.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-settings -p sapphire-framework-bridge-api` — expected: compile errors.
- [ ] **Step 3: Implement** as in Interfaces; set `process: Process::Bridge` in the bridge's defs.
- [ ] **Step 4: Run** the same plus `cargo check --workspace --all-targets --all-features` and clippy `-D warnings` on the touched crates — expected: pass.
- [ ] **Step 5: Commit** `feat(settings): check-only batches, a setting's process, and dry_run on the wire (#314)`.

### Task 2: The bridge honours `dry_run`

**Files:**
- Modify: `crates/sapphire-framework-bridge/src/control.rs` (`config_write`, the `config.set`/`config.unset` handlers, `bridge.status`)
- Test: the bridge's existing config tests module (where `config.set` is tested today)

**Interfaces:**
- Consumes: Task 1's `check`, `dry_run`, `config_dry_run`.

- [ ] **Step 1: Failing tests:** `a_dry_run_set_validates_and_writes_nothing` (valid batch → `Ok`, `changed` lists the keys, files unchanged, no embed reload); `a_dry_run_reports_the_error_a_write_would` (bad value → same error as without `dry_run`); `the_status_says_dry_run_is_supported` (`config_dry_run == true`).
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-bridge --all-features` — expected: fail.
- [ ] **Step 3: Implement:** `config_write` takes `dry_run: bool`; when set, call `settings::check` (inside `write_settings`, so it sees a consistent state) and return `ConfigWriteResult { changed: applied.keys, pending_restart: <current list> }` without reloading.
- [ ] **Step 4: Run** the bridge suite and clippy — expected: pass.
- [ ] **Step 5: Commit** `feat(bridge): config.set and config.unset can check without writing (#314)`.

### Task 3: The app server's settings (owners, files, migration, `Settings`)

**Files:**
- Create: `crates/sapphire-framework-server/src/settings.rs`
- Modify: `crates/sapphire-framework-server/Cargo.toml` (dep `sapphire-framework-settings`), `src/lib.rs` (module, re-exports, `run` builds `ServerConfig` from settings when `config` is not overridden), `src/config.rs` (`ServerConfig::from_settings`, file constant now `config.toml`; keep the `[mcp]` message)
- Test: unit tests in `settings.rs`; `tests/settings.rs`

**Interfaces:**
- Produces (all `pub` unless noted, re-exported from the crate root):
  - `pub trait AppSettings: Send + Sync + 'static { fn defs(&self) -> Vec<SettingDef>; fn validate(&self, staged: &Staged) -> Result<(), Vec<FieldError>> { Ok(()) } }`, with `impl AppSettings for ()` (no keys). (The app does not choose file paths; the framework wraps it in a `SettingsOwner`. This refines the spec's "the `SettingsOwner` the app passes".)
  - `pub(crate) struct SettingsFiles { config_dir: PathBuf, app_name: &'static str, selection: SelectionFile }` with `device() -> PathBuf`, `workspace() -> Result<PathBuf>` (the currently saved selection's `{root}/.{app}/config.toml`, read at each call; none → `Error` with `no workspace is selected`), `secret(key) -> PathBuf`.
  - `pub(crate) fn registry(ctx: &AppContext, files: &SettingsFiles, app: Arc<dyn AppSettings>) -> Result<Registry>` — framework owner index 0, app owner index 1, reserved names per Global Constraints (`net`, `embed` reserved to an index no owner has, e.g. `usize::MAX`); rejects an app key not under `<app_name>.` or with `Scope::Workgroup` (`Error::InvalidDef`). Every def from these owners has `process: Process::App`.
  - `pub(crate) fn migrate(config_dir: &Path) -> Result<()>` (server.toml → config.toml + `.v1`).
  - `#[derive(Clone)] pub struct Settings` with `pub fn get(&self, key: &str) -> Option<Value>`, `pub fn get_as<T: FromValue>(&self, key: &str) -> Option<T>` (`FromValue` implemented for `bool`, `i64`, `String`, `Vec<String>`), `pub fn changed(&self) -> tokio::sync::watch::Receiver<u64>`; `pub(crate) fn load(ctx, app) -> Result<Settings>` (migrate, build registry, read values, capture `StartState`); `pub(crate) fn refresh(&self) -> Result<()>` (re-read; bump `changed` iff a `Live` key's value changed; `Restart` keys keep their start values); `pub(crate) fn registry(&self) -> &Registry`, `pub(crate) fn start(&self) -> &StartState`.
  - `ServerConfig::from_settings(settings: &Settings) -> Result<ServerConfig>`.

- [ ] **Step 1: Failing tests:** `server_toml_becomes_config_toml` (text equal, `server.toml.v1` exists, `server.toml` gone); `an_existing_config_toml_is_not_overwritten`; `the_old_mcp_table_still_says_where_it_went`; `server_config_comes_from_the_settings` (`http.listen = "127.0.0.1:9000"`, `update.check = false` in config.toml → same `ServerConfig` fields); `an_app_key_outside_its_name_is_refused`; `an_app_key_in_the_workgroup_is_refused`; `an_app_key_named_http_is_refused`; `the_workspace_scope_follows_the_selection` (two selections in turn → two files); `no_selection_means_no_workspace_scope` (message `no workspace is selected`); `a_live_change_fires_changed_and_a_restart_change_does_not`; `an_unparsable_config_names_the_file`.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-server --all-features settings` — expected: compile errors.
- [ ] **Step 3: Implement** as in Interfaces. `run` keeps `AppServer::config(cfg)` as an override; otherwise `ServerConfig::from_settings(&Settings::load(ctx, Arc::new(()))?)` (Task 5 swaps in the real `Settings`).
- [ ] **Step 4: Run** the crate suite `--all-features` and clippy — expected: pass.
- [ ] **Step 5: Commit** `feat(server): the app server's settings, in config.toml (#314)`.

### Task 4: The app server answers `config.*`

**Files:**
- Create: `crates/sapphire-framework-server/src/config_methods.rs`
- Modify: `src/lib.rs` (register the five methods after the relay; `AppServer` gains `pub(crate) settings: Option<Settings>`), `src/relay.rs` (expose `pub(crate) async fn bridge(&self) -> Option<Arc<BridgeClient>>` for the config module, reusing `BridgeLink::client`)
- Test: `tests/config_methods.rs` (against `StubBridge`; add `StubBridge` canned answers for `config.*` and `bridge.status` with `config_dry_run`)

**Interfaces:**
- Consumes: Task 1 types, Task 3 `Settings`/`registry`, Task 2 behaviour (tests use `StubBridge`).
- Produces: `pub(crate) fn routes(router: Router, settings: Settings, relay: Arc<Relay>) -> Router` registering `config.schema|list|get|set|unset`; writes serialized by one `tokio::sync::Mutex<()>`.

Behaviour (spec "config.* on the app server"): schema = own defs then the bridge's; list merged, `Workgroup` → bridge only, `Workspace` → own only; get → owner; no bridge → own part only, a bridge-owned get → `BRIDGE_NOT_RUNNING`; writes: own `check`, then bridge `dry_run` (only if bridge part non-empty; bridge status must report `config_dry_run`, else the "too old" message), then bridge write, then own `apply`, then `settings.refresh()`; unknown key with no bridge → the "unknown setting … (or sapphire-bridge …)" message; result `changed` = both, `pending_restart` = own (via `StartState`) ∪ bridge's. Values never logged (log method + outcome only).

- [ ] **Step 1: Failing tests:** `the_schema_merges_both_sides` (own keys first, each with its `process`); `list_filters_by_scope`; `a_mixed_batch_with_a_bad_bridge_value_writes_nothing`; `a_mixed_batch_with_a_bad_app_value_never_reaches_the_bridge` (stub saw no `config.set`); `a_mixed_batch_writes_bridge_then_app`; `without_a_bridge_app_keys_still_read_and_write`; `without_a_bridge_a_batch_with_a_bridge_key_fails`; `an_old_bridge_refuses_mixed_batches` (message verbatim); `a_live_write_updates_settings`; `concurrent_writes_both_land`; `setting_values_are_never_logged` (own test binary with a capturing subscriber, as `relay_logging.rs`: `http.allowed_hosts` value and an app secret absent from the log).
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-server --all-features --test config_methods` — expected: fail.
- [ ] **Step 3: Implement** as above.
- [ ] **Step 4: Run** the crate suite and clippy — expected: pass.
- [ ] **Step 5: Commit** `feat(server): the app server answers config.*, merged with the bridge's (#314)`.

### Task 5: In-place restart

**Files:**
- Create: `crates/sapphire-framework-server/src/restart.rs`
- Modify: `src/lib.rs` (split `run` into binding and `pub(crate) async fn serve_on(self, listener: &mut sapphire_ipc::Listener, restart: Option<Arc<Notify>>) -> Result<Exit>`; register `server.restart`), `crates/sapphire-framework-server-api/src/lib.rs` (`SERVER_RESTART`)
- Test: `tests/restart.rs`

**Interfaces:**
- Consumes: Task 3 `Settings::load`, `AppSettings`; Task 4 `Settings` on `AppServer`.
- Produces:
  - `pub const SERVER_RESTART: &str = "server.restart";` (server-api, result `Ack`).
  - `pub async fn serve_restarting<A, F>(owner: A, build: F) -> Result<()> where A: AppSettings + Clone, F: Fn(&Settings) -> AppServer` — migrate + load settings, bind the endpoint once, loop { build, attach owner and settings, `serve_on` }, rebuild on `Exit::Restart`, return on `Exit::Stopped`.
  - `pub(crate) enum Exit { Stopped, Restart }`.
  - `AppServer::run(self)` unchanged in signature: binds and calls `serve_on(.., None)`; `server.restart` there answers `this server cannot restart itself; restart its service`.
  - `const RESTART_DELAY` equal to the bridge's.

`serve_on` must, before returning `Exit::Restart`, stop everything `run` stops today and await (not just abort) the sync tasks and drop every `Arc` it holds of the runtime and host, so the next `build`'s runtime can open the replica store.

- [ ] **Step 1: Failing tests:** `a_restart_serves_again_on_the_same_endpoint`; `a_restart_picks_up_a_restart_setting` (set `update.check=false` via `config.set`, restart, `config.list` shows it no longer pending); `a_client_reconnects_after_a_restart`; `a_restart_keeps_the_process_and_lock` (pid in `server.info` unchanged); `run_cannot_restart` (message verbatim); `a_restart_with_sync_reopens_the_replica_store` (server with `sync` enabled on a workspace, restart, `sync` still enabled and serving).
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-server --all-features --test restart` — expected: fail.
- [ ] **Step 3: Implement** as above; `server.restart` answers `Ack`, then after `RESTART_DELAY` notifies.
- [ ] **Step 4: Run** the crate suite and clippy — expected: pass.
- [ ] **Step 5: Commit** `feat(server): server.restart rebuilds the app server in place (#314)`.

### Task 6: `<app> config`

**Files:**
- Create: `crates/sapphire-framework-server/src/command/config.rs` (or a `config` section in `command.rs` if the module is not split; follow the file's existing layout)
- Modify: `src/command.rs` (`FrameworkCommand::Config`, `dispatch` signature), `src/bin/server-test-app.rs`, every caller of `dispatch` in the workspace
- Test: `tests/cli_config.rs`

**Interfaces:**
- Consumes: Tasks 3–5; bridge-api `parse_entries`, `read_secret`, `describe_entries`.
- Produces:
  - `FrameworkCommand::Config(AppConfigCommand)`; `AppConfigCommand` = `List { scope: AppScopeFlags, show_origin: bool }`, `Get { key }`, `Set { entries, scope, stdin, restart }`, `Unset { keys, scope, restart }`.
  - `#[derive(clap::Args)] pub struct AppScopeFlags { --device, --workgroup, --workspace }` (mutually exclusive) with `fn scope(&self) -> Option<Scope>`.
  - **Breaking:** `FrameworkCommand::dispatch<A, F>(self, owner: A, build: F, version: &'static str) -> Result<i32>` with the same bounds as `serve_restarting`; `Serve` → `serve_restarting(owner, build)`; other variants call `build(&Settings::load(..)?)` once where they need an `AppServer`.

Online: the five calls over the app connection (`BridgeClient::relayed`), then per-process restart lines; `--restart` restarts the bridge first (relayed `bridge.restart`) if any pending key's process is `Bridge`, then the app server (`server.restart`, then wait up to 30 s for `server.info`). Offline (no app server): registry from `owner`, `migrate`, read/write files directly; a key the registry does not know → the offline bridge-key message; `list` adds the bridge-not-shown line.

- [ ] **Step 1: Failing tests:** `config_set_and_get_through_the_server`; `config_list_shows_both_sides`; `config_restart_restarts_the_app_server` (pending `http.listen` → `--restart` → server back, nothing pending); `offline_set_writes_the_file` (no server; `config.toml` holds the key afterwards); `offline_a_bridge_key_is_refused` (message verbatim); `offline_config_reports_an_unparsable_file`; `a_secret_comes_from_stdin_only` (`set <app>.token=x` refused; `set <app>.token --stdin` with stdin `x` writes the 0600 file).
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-server --all-features --test cli_config` — expected: fail.
- [ ] **Step 3: Implement** as above; update `server-test-app` to `serve_restarting((), |_| AppServer::new(..)...)` via `dispatch`.
- [ ] **Step 4: Run** the crate suite, `cargo check --workspace --all-targets --all-features`, clippy — expected: pass.
- [ ] **Step 5: Commit** `feat(server)!: <app> config, and dispatch takes the app's settings and build function (#314)`.

### Task 7: Docs and changelog

**Files:**
- Modify: `CHANGELOG.md` (`## [Unreleased]`), `docs/ARCHITECTURE.md`, `docs/superpowers/specs/2026-10-21-settings-registry-design.md` (Part 3 outline → pointer to the two Part 3 specs)

- [ ] **Step 1: Write** Added (app settings, `Settings`, `serve_restarting`, `server.restart`, `<app> config`, `dry_run`, `Process`, `config_dry_run`, `check`) and Changed **Breaking** (`server.toml` → `config.toml` migration; `dispatch` signature; `config.*` answered by the app server, not relayed).
- [ ] **Step 2: Run** `cargo test --workspace --all-features` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` — expected: pass (no `cargo publish --dry-run`).
- [ ] **Step 3: Commit** `docs: the app's own settings and server.restart (#314)`.
