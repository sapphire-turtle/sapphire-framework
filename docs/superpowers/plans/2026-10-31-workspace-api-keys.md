# Workspace API Keys Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** HTTP clients authenticate against API keys kept in the served workspace (`<root>/.<app>/api_keys/`), with no bridge involved; external devices are imported once and removed from the bridge.

**Architecture:** `WorkspaceKeyVerifier` (server crate) implements the existing `sapphire_framework_keys::Verifier` over corundum's `KeyVerifier`, finding the served root from the app's selection file. The CLI and GUI open corundum's `KeyStore` on the same directory. A one-time import copies workgroup-ledger records into the store. The bridge's external-device surface is deleted.

**Tech Stack:** Rust 2024, corundum-framework-keys (+ corundum-framework-gui `keys`), axum 0.8, clap 4, egui 0.37.

**Spec:** `docs/superpowers/specs/2026-10-31-workspace-api-keys-design.md`

## Global Constraints

- Store directory: `<workspace root>/.<app>/api_keys`, only through `sapphire_framework_keys::store_dir(app_name, root)` (in the keys crate, not the server, because the GUI's server dependency is optional).
- Token prefix for new keys: `sapphire-ak-`.
- Imported hash: `"sha256:" + token_sha256`; ids kept as their grain-id text.
- Import marker: `<app data dir>/api_keys_imported`.
- CLI: `api-key`; `external-device` is a hidden alias for one release that warns on stderr; `--app` on the alias is refused.
- With no verifier, `protect` answers 401 (was 503). No 503 path remains in auth.
- bridge-api's major version goes up (3 → 4); every crate depending on it moves.
- Depends on the corundum release that ships `corundum-framework-keys` (its version from that release).

## Review Focus

- The selection file is rewritten during a switch while a request is verified: the verifier reads either the old or the new root, never fails open (partial read → refuse, retry next request).
- The served workspace's root directory is gone (moved outside the app): 401, not a panic or 500.
- Two hosts import at once and the workspace then syncs: both write the same `<id>.toml` content; the sync's conflict copy (if any) is not retired, so the key still works.
- A key retired in the old workgroup ledger after the import ran: not propagated (the ledger is no longer read) — the CHANGELOG says to retire through `api-key` from now on.
- `api-key` run while a server is running: changes are visible to the server's verifier on the next miss with no restart.

---

### Task 1: Depend on corundum-framework-keys; reshape `sapphire-framework-keys`

**Files:**
- Modify: `Cargo.toml` (workspace deps: `corundum-framework-keys = "<released version>"`, bump `corundum-framework-platform` / `-gui` to the same), `crates/sapphire-framework-keys/{Cargo.toml,src/lib.rs,src/verify.rs,src/auth.rs,src/config.rs}`, delete `crates/sapphire-framework-keys/tests/bridge.rs`
- Modify: `crates/sapphire-framework/src/lib.rs:130-140` (re-exports)

**Interfaces:**
- Produces: `Authenticated { id: corundum_framework_keys::KeyId, name: String }`; `enum Verdict { Accepted(Authenticated), Refused }`; trait `Verifier` unchanged otherwise; `BridgeVerifier` and `CACHE_TTL` removed; `sapphire_framework::http::{AuthConfig, Authenticated, Verdict, Verifier, protect, KeyId}` (drop `BridgeVerifier`, `GrainId` re-export from keys).

- [ ] **Step 1: Write failing test** in `src/auth.rs` tests: `unconfigured_refuses_with_401` (was 503).
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-keys` — Expected: FAIL (503).
- [ ] **Step 3: Implement**: remove `BridgeVerifier`, `Unavailable` and its 503 branch; `refuse` returns `UNAUTHORIZED`; update docs; drop the bridge-client dependency.
- [ ] **Step 4: Run** `cargo test -p sapphire-framework-keys` and `cargo build --workspace` — Expected: keys PASS; the server fails to build only at its `BridgeVerifier` use (fixed in Task 2).
- [ ] **Step 5: Commit** `feat(keys)!: drop BridgeVerifier; refuse unconfigured with 401`

### Task 2: `WorkspaceKeyVerifier` and the HTTP default

**Files:**
- Create: `crates/sapphire-framework-server/src/api_keys.rs`, `crates/sapphire-framework-keys/src/store.rs`
- Modify: `crates/sapphire-framework-server/src/lib.rs:879-882` (default auth), `crates/sapphire-framework-server/tests/http_endpoints.rs` (replace the bridge/503 tests)

**Interfaces:**
- Consumes: `Verifier`, `Verdict`, `Authenticated` (Task 1); `SelectionFile::for_app(ctx).load(app)` → `Option<Selection { root, .. }>`.
- Produces:
  - In `sapphire-framework-keys` (no `axum` feature needed): `pub fn store_dir(app_name: &str, root: &Path) -> PathBuf` → `root.join(format!(".{app_name}")).join("api_keys")`; `pub const TOKEN_PREFIX: &str = "sapphire-ak-";`; `pub fn open_store(app_name: &str, root: &Path) -> corundum_framework_keys::Result<KeyStore>` (with the prefix).
  - `pub struct WorkspaceKeyVerifier`; `WorkspaceKeyVerifier::for_app(ctx: &'static AppContext) -> Self`; `.cache_ttl(Duration) -> Self`; `impl Verifier`.
  - Re-exported at `sapphire_framework_server::api_keys`.

- [ ] **Step 1: Write failing tests** in `tests/http_endpoints.rs` (no bridge started anywhere):
  - `key_in_workspace_is_accepted`: select a temp workspace; `open_store(..).add("p")`; request with the token → 200; wrong token → 401.
  - `retire_takes_effect_after_ttl`: server built with a verifier of ttl 100 ms (via `http_auth`), retire, sleep 200 ms → 401.
  - `copied_key_file_is_accepted`: write the `<id>.toml` produced in another temp dir into the served store directory after the server started → 200.
  - `switch_changes_the_keys`: key in workspace A accepted; select workspace B → A's token 401, B's key 200.
  - `missing_root_refuses`: selection points at a removed directory → 401.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-server --test http_endpoints` — Expected: FAIL.
- [ ] **Step 3: Implement.** The verifier holds `Mutex<Option<(SystemTime /*mtime*/, PathBuf /*root*/, Arc<KeyVerifier>)>>`; per request, `metadata(selection file).modified()`; on change re-load the selection and, if the root differs, build a new `KeyVerifier::new(&dir(..)).token_prefix(TOKEN_PREFIX).cache_ttl(ttl)`; no selection or read error → `Refused`. Map corundum `Verdict::Accepted(info)` to `Authenticated { id: info.id, name: info.name }`. Run the corundum call inside `tokio::task::spawn_blocking`. The default in `lib.rs` becomes `AuthConfig::new(Arc::new(WorkspaceKeyVerifier::for_app(ctx)))`.
- [ ] **Step 4: Run** the same — Expected: PASS; then `cargo test -p sapphire-framework-server` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(server)!: check HTTP tokens against the workspace's API keys`

### Task 3: One-time import of external devices

**Files:**
- Create: `crates/sapphire-framework-server/src/api_keys/import.rs` (make `api_keys` a directory module)
- Modify: `crates/sapphire-framework-server/src/current.rs` (call after a workspace becomes current, including at start)

**Interfaces:**
- Consumes: `sapphire_registry::ExternalDevices::open(dir)`, `ExternalDevice { id, name, description, apps, token_sha256, created_at, rotated_at, retired_at }`; `open_store` (Task 2); `KeyStore::import(Key)`.
- Produces: `pub fn import_external_devices(ctx: &AppContext, bridge_dir: &Path, root: &Path) -> Result<ImportReport>`; `pub struct ImportReport { pub imported: usize, pub skipped: usize }`. The caller passes `bridge_dir` from `SAPPHIRE_BRIDGE_DIR` or `dirs::data_dir()/BRIDGE_DIR_NAME` without creating it (missing → nothing to import, marker written).

- [ ] **Step 1: Write failing tests** (temp bridge dir with `workgroups/<id>/root/external_devices/` made by `ExternalDevices::add`):
  - `imports_unretired_records_for_this_app`: three records — for this app, for another app, retired for this app → `imported == 1`; its old token authenticates in `open_store`; id text equal to the grain id.
  - `second_run_does_nothing`: marker present → `imported == 0`, store unchanged.
  - `existing_id_is_skipped`: store already holds the id (simulating sync) → `skipped == 1`, no error, marker written.
  - `missing_bridge_dir_writes_marker`.
  - `failure_leaves_no_marker`: store dir not writable (unix only, `#[cfg(unix)]`) → `Err`, marker absent.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-server import` — Expected: FAIL.
- [ ] **Step 3: Implement**; errors are logged by the caller with `tracing::warn!` and never stop the server.
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** `feat(server): import the workgroup's external devices into the workspace once`

### Task 4: `api-key` CLI and the `external-device` alias

**Files:**
- Modify: `crates/sapphire-framework-server/src/command.rs:76-82,166-169,595-…` (replace `ExternalDevice` variant and `external_device()`)
- Test: `crates/sapphire-framework-server/tests/cli_config.rs` (or a new `tests/cli_api_key.rs`)

**Interfaces:**
- Consumes: `corundum_framework_keys::KeyCommand` (`run_to`), `open_store`.
- Produces: `FrameworkCommand::ApiKey(KeyCommand)` as `api-key`; `FrameworkCommand::ExternalDevice(LegacyExternalDevice)` `#[command(hide = true)]`, where `LegacyExternalDevice` wraps `KeyCommand` plus a hidden `--app` arg (`Vec<String>`).

- [ ] **Step 1: Write failing tests:**
  - `api_key_add_without_server`: workspace selected, no server → exit 0, stdout is one `sapphire-ak-` token, which authenticates in `open_store`.
  - `no_workspace_is_exit_1`: no selection → exit 1, stderr names `workspace select`.
  - `external_device_alias_warns`: `external-device add p` → stderr contains "renamed to `api-key`", key added.
  - `external_device_app_is_refused`: `external-device add p --app x` → exit 2-like error saying keys are per workspace.
- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement.** Root from `SelectionFile::for_app(ctx).load(app)`.
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** `feat(cli)!: api-key replaces external-device`

### Task 5: GUI: Settings › API keys

**Files:**
- Modify: `crates/sapphire-framework-gui/Cargo.toml` (`corundum-framework-gui` feature `keys`; `corundum-framework-keys`), `src/settings.rs` (Screen `ExternalDevices` → `ApiKeys`, message id `sapphire-settings-api-keys`), `src/views/mod.rs`, `src/client/{types.rs,conn.rs,exec.rs}` (remove `external_devices` snapshot field and external-device commands/outputs), `locales/{en,ja}/sapphire.ftl` (`sapphire-settings-api-keys` = "API keys" / "API キー"; `sapphire-api-keys-no-workspace` = "Choose a workspace to manage its API keys." / "API キーを管理するには、ワークスペースを選んでください。"; remove the external-device strings)
- Delete: `src/views/external.rs`
- Modify: `src/views/embedding.rs` (its external-device reference)

**Interfaces:**
- Consumes: `corundum_framework_gui::keys::{KeysPage, LocalKeys}`, `sapphire_framework_keys::open_store` (add `sapphire-framework-keys` to the GUI's dependencies, default features).
- Produces: the page holds `Option<(PathBuf, KeysPage)>`, rebuilt when `Snapshot`'s `workspace.current` root changes.

- [ ] **Step 1: Write failing tests** in `settings.rs` tests (existing render helpers):
  - `api_keys_page_lists_workspace_keys`: snapshot with a current workspace whose store has key "p" → rendering the API keys screen shows "p".
  - `api_keys_page_without_workspace_says_so`: no current workspace → the `sapphire-api-keys-no-workspace` text.
  - i18n check passes.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** `feat(gui)!: Settings › API keys over the workspace's keys`

### Task 6: Remove the bridge's external-device surface

**Files:**
- Modify: `crates/sapphire-framework-bridge-api/{Cargo.toml (version 4.0.0),src/lib.rs (consts, RELAYED, NOT_RELAYED, re-exports),src/cli.rs}`; delete `src/external.rs`
- Modify: `crates/sapphire-framework-bridge/src/{control.rs,command.rs,workgroup.rs,lib.rs}`; delete `src/external.rs`
- Modify: `crates/sapphire-framework-bridge-client/src/client.rs` (remove external-device calls)
- Modify: `crates/sapphire-framework-server/src/relay.rs`, `tests/mcp.rs`, `tests/metrics.rs` (drop references)
- Modify: `crates/sapphire-framework-registry/src/external_devices.rs` (`#[deprecated(note = "read only by the one-time import (#364)")]` on `ExternalDevices`; allow it in the import)
- Modify: every `sapphire-framework-bridge-api = { version = "3…" }` to `"4.0.0"` (workspace and `bridge/Cargo.toml`)

- [ ] **Step 1:** Delete and fix until `cargo build --workspace --all-targets` passes.
- [ ] **Step 2:** `cargo test -p sapphire-framework-bridge-api` — Expected: PASS, including the relay-list test.
- [ ] **Step 3:** `cargo test --workspace` (memory: a failing binary stops cargo test; read the whole output) — Expected: PASS.
- [ ] **Step 4: Commit** `feat(bridge)!: remove external devices from the bridge`

### Task 7: Docs and release notes

**Files:**
- Modify: `README.md`, `README.ja.md`, `bridge/README*.md` (external-device → api-key), `docs/superpowers/specs/2026-10-11-external-devices-design.md` (a "Superseded by 2026-10-31-workspace-api-keys-design.md" line at the top)
- Modify: `CHANGELOG.md` is release-plz's; put the Breaking notes in the PR body for the release PR to carry (memory: hand-write Breaking): keys are per workspace; tokens keep working (imported once); `external-device` → `api-key`; bridge-api 4; 503 → 401; `BridgeVerifier` gone, use `WorkspaceKeyVerifier::for_app`.

- [ ] **Step 1:** `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo doc --workspace --no-deps` — Expected: clean.
- [ ] **Step 2: Commit** `docs: API keys replace external devices`
