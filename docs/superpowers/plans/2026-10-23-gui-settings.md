# GUI Settings Pages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The GUI shows and edits every `config.schema` key on pages built from the schema, with field-level errors and a per-process restart banner.

**Architecture:** The servers attach the keys of a rejected batch to the `RpcError`. The GUI client gains three commands (load, write, restart) whose failures keep those field errors. A pure model (`views/config/model.rs`) turns the schema and entries into pages, tabs, drafts and batches; widgets and pages render it; `Settings` places the pages and hosts the banner; the Embedding screen's hand-made forms give way to its section's page.

**Tech Stack:** Rust 2024, egui/eframe, tokio, `sapphire-framework-bridge-client`, `sapphire-framework-ipc`.

**Spec:** `docs/superpowers/specs/2026-10-23-gui-settings-design.md`

## Global Constraints

- Copy, verbatim: `No workspace is selected`; `sapphire-bridge is not running`; `from device` / `from workgroup` / `from workspace` / `default`; `sapphire-bridge needs a restart to apply: {keys}` + button `Restart bridge`; `the {app} server needs a restart to apply: {keys}` + button `Restart {app} server`; `the {app} server did not come back within 30 s`; `Revert`, `Apply`; heading `Configuration`.
- Tab order Device, Workgroup, Workspace; pages in schema order; a section joins an existing screen when its name equals the screen's label ignoring case (today only "Embedding").
- A section is the app's only when every key in it has `process == App` and starts with `<app_name>.`.
- Secret values are never shown, logged or kept after a successful Apply; a secret is in the draft only once typed into.
- Setting values are never logged; error messages never include values.
- A reload never discards a draft.
- No crate version bumps. bridge-api additions are new types only (it is already headed for 3.0 from #324).
- Nothing moves to `corundum-gui`.

## Review Focus

1. A key whose server value changes while it has a draft (another device edits the workgroup file): the draft stays and the row says "changed elsewhere" — Task 3 test `a_reload_keeps_a_draft_and_marks_a_changed_value`.
2. A list value with blank lines or trailing spaces: blank lines are dropped, items trimmed, each item checked as the item kind — Task 3 test `list_text_drops_blank_lines_and_checks_each_item`.
3. Apply while the bridge is down with only app keys in the draft: it succeeds; with a bridge key it fails with the server's message, draft kept — Task 2 tests `a_write_of_app_keys_works_without_a_bridge` and `a_write_with_a_bridge_key_fails_without_a_bridge`.
4. A Workspace-tab draft when the workspace is switched away: the draft is kept but the tab disables until a workspace is selected; Apply is impossible meanwhile — Task 3 test `a_workspace_draft_waits_for_a_workspace`.
5. Restart of the app server from the GUI: the GUI's own connection drops and comes back; the snapshot recovers and the settings reload — Task 2 test `restart_app_comes_back_and_the_client_reconnects`.

---

### Task 1: Field errors on the wire

**Files:**
- Modify: `crates/sapphire-framework-bridge-api/src/config.rs` (new types), `src/lib.rs` (re-export)
- Modify: `crates/sapphire-framework-settings/src/error.rs` (`Error::field_errors`)
- Modify: `crates/sapphire-framework-bridge/src/control.rs` (`config_write` errors), `src/command.rs` (`config` CLI printing)
- Modify: `crates/sapphire-framework-server/src/config_methods.rs` (own errors; bridge errors pass through), `src/command/config.rs` (printing)
- Test: bridge's config IPC tests (`tests/settings_ipc.rs`), `crates/sapphire-framework-server/tests/config_methods.rs`, `tests/cli_config.rs`

**Interfaces:**
- Produces:
  - bridge-api: `#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)] pub struct ConfigFieldError { pub key: String, pub message: String }` and `pub struct ConfigFieldErrors { pub fields: Vec<ConfigFieldError> }` with `pub fn from_rpc(err: &RpcError) -> Option<ConfigFieldErrors>` (reads `err.data`) — named `ConfigFieldError` (not the spec's `FieldError`) so it does not collide with the settings crate's `FieldError` the server re-exports.
  - settings crate: `impl Error { pub fn field_errors(&self) -> Vec<ConfigFieldError> }` — one entry for `UnknownKey`, `ScopeNotAllowed`, `InvalidValue`, `SecretOnCommandLine`, and each keyed `FieldError` of `Rejected`; empty otherwise. Messages never include values.
  - A shared helper in the server and bridge: a settings `Error` becomes `RpcError::internal(err.to_string())` with `data = Some(json(ConfigFieldErrors))` when `field_errors()` is non-empty.

- [ ] **Step 1: Failing tests:** settings `field_errors_name_the_keys` (each variant above → its key; `Io` → empty); bridge `a_rejected_set_carries_its_field_errors` (`config.set` of `net.discovery` with a string → `RpcError.data` decodes to one `ConfigFieldError` with key `net.discovery`), also with `dry_run`; server `an_app_key_error_carries_its_field_errors` and `a_bridge_key_error_passes_its_field_errors_through` (StubBridge answers `config.set` with an `RpcError` carrying data → the client sees the same data); CLI `config_set_prints_each_field_error` (`{key}: {message}` on stderr).
- [ ] **Step 2: Run** the touched crates' tests — expected: compile errors.
- [ ] **Step 3: Implement** as above; both CLIs print one `{key}: {message}` line per field after the error message.
- [ ] **Step 4: Run** the touched crates' suites, workspace check, clippy `-D warnings` — expected: pass.
- [ ] **Step 5: Commit** `feat(settings): a rejected batch names its keys on the wire (#314)`.

### Task 2: GUI client — load, write, restart

**Files:**
- Modify: `crates/sapphire-framework-gui/src/client/{types.rs,exec.rs,conn.rs,mod.rs}`, every user of `Outcome.result` (`src/settings.rs`, `src/views/*.rs` `on_outcome`)
- Test: `crates/sapphire-framework-gui/tests/client.rs`, unit tests in `exec.rs`

**Interfaces:**
- Consumes: Task 1 `ConfigFieldErrors::from_rpc`.
- Produces (in `client::types`, re-exported where `Command` is):
  - `#[derive(Clone, Debug, Default, PartialEq)] pub struct CommandError { pub message: String, pub fields: Vec<ConfigFieldError> }` with `impl From<String>`; `Outcome.result: Result<CommandOutput, CommandError>` (views that showed the `String` show `.message`).
  - `#[derive(Clone, Debug, Default, PartialEq)] pub struct ConfigState { pub schema: Vec<SettingDef>, pub effective: Vec<Entry>, pub by_scope: BTreeMap<Scope, Vec<Entry>> }` — `effective` is `config.list(None)`; `by_scope` holds `Device`, `Workgroup`, and `Workspace` only when a workspace is selected; a scope the bridge cannot answer (bridge down) is simply missing its bridge keys.
  - `Command::ConfigLoad`, `Command::ConfigWrite { scope: Scope, set: Vec<ConfigEntry>, unset: Vec<String> }`, `Command::Restart(Process)`.
  - `CommandOutput::Config(ConfigState)`, `CommandOutput::Written(ConfigWriteResult)`.
  - `ConfigWrite` order: `config_check_set` (if `set` non-empty), `config_check_unset` (if `unset` non-empty), `config_set`, `config_unset`; first failure returns.
  - `Restart(App)`: `server.restart`, wait for the client's `closed()`, drop the cached connection, then reconnect and poll `server.info` every 200 ms until it answers or 30 s pass (error `the {app} server did not come back within 30 s`). `Restart(Bridge)`: relayed `BridgeClient::restart()`.
  - Commands keep the existing timeouts (`command_timeout`), except `Restart` which uses 45 s.

- [ ] **Step 1: Failing tests** (`tests/client.rs` fixture: real bridge + app server through the relay): `config_load_returns_schema_and_scopes` (schema has `net.discovery` and `http.listen`; `by_scope` has Device and Workgroup, not Workspace before one is selected); `a_mixed_write_lands_in_both_processes`; `a_rejected_write_keeps_its_field_errors` (`fields[0].key == "net.discovery"`); `a_write_of_app_keys_works_without_a_bridge`; `a_write_with_a_bridge_key_fails_without_a_bridge`; `restart_bridge_comes_back`; `restart_app_comes_back_and_the_client_reconnects` (server started with `serve_restarting`; next `ConfigLoad` succeeds and the snapshot's server is `Up`).
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui --all-features --test client` — expected: compile errors.
- [ ] **Step 3: Implement** as above.
- [ ] **Step 4: Run** the GUI suite and clippy `-D warnings` — expected: pass.
- [ ] **Step 5: Commit** `feat(gui): load, write and restart settings through the client (#314)`.

### Task 3: The settings model

**Files:**
- Create: `crates/sapphire-framework-gui/src/views/config/mod.rs`, `src/views/config/model.rs`
- Modify: `src/views/mod.rs` (module)
- Test: unit tests in `model.rs`

**Interfaces:**
- Consumes: Task 2 `ConfigState`, `CommandError`.
- Produces (`pub(crate)`):
  - `enum Placement { App, Screen(String), Configuration }`; `struct PageSpec { section: String, placement: Placement, scopes: Vec<Scope>, keys: Vec<String> }`; `fn pages(schema: &[SettingDef], app_name: &str, screens: &[&str]) -> Vec<PageSpec>` (rules in Global Constraints).
  - `enum Edit { Set { text: String, parsed: Result<Value, String> }, Unset }`; `fn parse_text(kind: &Kind, text: &str) -> Result<Value, String>` (List: one item per line, blank lines dropped, items trimmed, each `Kind::parse`d as the item kind; others `Kind::parse`; Secret: non-empty string).
  - `struct Draft { edits: BTreeMap<String, Edit>, base: BTreeMap<String, Option<EntryValue>> }` with `set(key, kind, text)`, `unset(key)`, `revert()`, `is_empty()`, `valid() -> bool`, `on_reload(&ConfigState, scope) -> BTreeSet<String>` (the drafted keys whose server value changed since the edit — "changed elsewhere"), `batch(scope) -> Option<(Vec<ConfigEntry>, Vec<String>)>` (None while invalid).
  - `struct Drafts(HashMap<(String /*section*/, Scope), Draft>)` with `get_mut`, `dirty_sections() -> BTreeSet<String>`.
  - `fn origin_label(origin: Origin, tab: Scope) -> Option<&'static str>` (`None` when the origin is the tab's scope; else the Global Constraints copy).
  - `fn route_errors(err: &CommandError, keys: &[String]) -> (BTreeMap<String, String>, Vec<String>)` (field errors on the page by key; everything else, and the message, as lines).
  - `fn pending_restarts(state: &ConfigState) -> BTreeMap<Process, Vec<String>>` (from `effective` entries with `pending_restart`, via the schema's `process`).
  - `fn tab_enabled(scope: Scope, workspace_selected: bool) -> bool`.

- [ ] **Step 1: Failing tests:** `pages_follow_the_schema_order_and_placement` (an app-only section → App; "embedding" → Screen("Embedding"); a mixed section → Configuration); `tabs_are_the_scopes_the_keys_allow_in_order`; `list_text_drops_blank_lines_and_checks_each_item`; `a_secret_is_in_the_draft_only_once_typed`; `a_reload_keeps_a_draft_and_marks_a_changed_value`; `batch_splits_set_and_unset_and_refuses_an_invalid_draft`; `origin_labels`; `field_errors_go_to_their_fields_and_the_rest_to_lines`; `pending_restarts_by_process`; `a_workspace_draft_waits_for_a_workspace` (tab disabled without a selection, draft kept).
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui --all-features config::model` — expected: fail.
- [ ] **Step 3: Implement** as above.
- [ ] **Step 4: Run** the GUI suite and clippy — expected: pass.
- [ ] **Step 5: Commit** `feat(gui): the model behind the settings pages (#314)`.

### Task 4: Widgets, the page view and the restart banner

**Files:**
- Create: `crates/sapphire-framework-gui/src/views/config/{widgets.rs,page.rs,banner.rs}`
- Test: render tests in those files (`test_support::render`)

**Interfaces:**
- Consumes: Task 3 model; `ViewCtx`, `error_line`, `unavailable` from `views/mod.rs`.
- Produces:
  - `fn value_widget(ui, def: &SettingDef, shown: Option<&EntryValue>, draft: Option<&Edit>, enabled: bool) -> Option<Edit>` (one widget per kind per the spec table; returns a new edit when the user changed it).
  - `struct ConfigPage { section: String, tab: Scope, errors: (BTreeMap<String,String>, Vec<String>), changed: BTreeSet<String> }` with `fn ui(&mut self, ui, cx: &ViewCtx, spec: &PageSpec, state: &ConfigState, drafts: &mut Drafts) -> Option<Command>` (emits `ConfigWrite` on Apply, `ConfigLoad` on "↻") and `fn on_outcome(&mut self, result: &Result<CommandOutput, CommandError>, drafts: &mut Drafts)`.
  - `struct RestartBanner { busy: Option<Process>, error: Option<String> }` with `fn ui(&mut self, ui, cx: &ViewCtx, pending: &BTreeMap<Process, Vec<String>>) -> Option<Command>` (emits `Restart(p)`), `on_outcome`.
  - Bridge keys read-only with the note when `cx.snapshot.bridge` is not `Up`; the whole page `unavailable` when the server is not `Up`.

- [ ] **Step 1: Failing tests:** `every_kind_renders` (a schema with one key of each kind, rendered on each tab, no command emitted); `a_bridge_key_is_read_only_without_a_bridge` (its widget returns no edit); `no_workspace_disables_the_workspace_tab`; `apply_emits_one_write_with_the_tab_scope` (a draft set, then `ui` with Apply simulated through the page's `apply()` helper → `ConfigWrite { scope: Device, .. }`); `a_failed_write_shows_field_errors_and_keeps_the_draft`; `the_banner_lists_each_process_with_its_button` (rendered; `restart_command(Process::App)` helper returns `Restart(App)`); `a_secret_field_never_shows_the_value`.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui --all-features config::` — expected: fail.
- [ ] **Step 3: Implement** as above.
- [ ] **Step 4: Run** the GUI suite and clippy — expected: pass.
- [ ] **Step 5: Commit** `feat(gui): settings widgets, pages and the restart banner (#314)`.

### Task 5: Settings hosts the pages; Embedding joins its section

**Files:**
- Modify: `crates/sapphire-framework-gui/src/settings.rs` (`Page::Config(String)`, list placement, load triggers, outcome routing, dirty marks, banner), `src/views/embedding.rs` (forms removed; section page below the status), `src/views/model.rs` (`LocalForm`/`RemoteForm` removed), `src/client/{types.rs,exec.rs}` (`Command::Embedding` and `EmbedRequest` removed)
- Test: `settings.rs` tests, `embedding.rs` tests

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces: `Settings` holds `config: Option<ConfigState>`, `drafts: Drafts`, `pages: Vec<PageSpec>`, one `ConfigPage` per section, a `RestartBanner`; sends `ConfigLoad` when a config page or Embedding is shown with nothing loaded, after a successful write or restart, and when `snapshot.server.current`'s workspace changes.

- [ ] **Step 1: Failing tests:** `config_pages_are_listed_by_placement` (app heading, "Configuration" heading, Embedding not listed twice); `a_dirty_page_is_marked_in_the_list`; `showing_a_config_page_loads_once`; `a_workspace_switch_reloads`; `a_successful_write_reloads`; `embedding_shows_status_and_its_section_page` (no `LocalForm`); existing `render_views` tests updated (no command emitted).
- [ ] **Step 2: Run** the GUI suite — expected: fail.
- [ ] **Step 3: Implement** as above.
- [ ] **Step 4: Run** `cargo test -p sapphire-framework-gui --all-features`, `cargo check --workspace --all-targets --all-features`, clippy `-D warnings` — expected: pass.
- [ ] **Step 5: Commit** `feat(gui)!: settings pages from the schema; Embedding edits through its section (#314)`.

### Task 6: Docs and changelog

**Files:**
- Modify: `CHANGELOG.md` (`## [Unreleased]`), `docs/ARCHITECTURE.md` (GUI section), `docs/superpowers/specs/2026-10-21-settings-registry-design.md` (Part 4 outline → pointer)

- [ ] **Step 1: Write** Added (settings pages, restart banner, `ConfigFieldErrors` / `ConfigFieldError`, CLIs print field errors) and Changed **Breaking** (`Outcome.result` error is `CommandError`; `Command::Embedding` / `EmbedRequest` removed; Embedding screen edits through its section).
- [ ] **Step 2: Run** `CARGO_INCREMENTAL=0 cargo test --workspace --all-features --no-fail-fast` and workspace clippy `-D warnings` — expected: pass.
- [ ] **Step 3: Commit** `docs: the GUI's settings pages (#314)`.
