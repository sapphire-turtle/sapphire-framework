# Bridge Desktop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `sapphire-bridge-desktop` opens the bridge's own window; desktop apps show a bridge summary with buttons that open that window on a page; the apps' `workgroup` / `device` / `embedding` subcommands go.

**Architecture:** `bridge/` splits into `bridge/headless` (CLI) and `bridge/desktop` (window + the same CLI). The window is `sapphire-framework-gui` in a bridge mode: `FrameworkClient` connects to the bridge directly, `Settings` shows the bridge's pages. Apps' `Settings` link instead of hosting the pages (`BridgePages::Linked`), finding the desktop binary through corundum's `registered_program`.

**Tech Stack:** Rust 2024, egui/eframe via corundum-framework-gui (single-instance with activation message), clap, sapphire-framework-bridge(-client).

**Spec:** `docs/superpowers/specs/2026-10-31-bridge-desktop-design.md`

## Global Constraints

- Packages/binaries: `bridge/headless` = package `sapphire-bridge-headless`, bin `sapphire-bridge`; `bridge/desktop` = package `sapphire-bridge-desktop`, bin `sapphire-bridge-desktop`, `publish = false`.
- Release files: `sapphire-bridge-<os>-<arch>` unchanged; add `sapphire-bridge-desktop-<os>-<arch>`.
- `--page` values: `status|workgroup|devices|embedding|appearance|logs|about`, default `status`.
- Single-instance name: `sapphire-bridge-desktop`; the activation message is `BridgePage::as_str()`.
- Logs page: last 500 lines of the newest file in `BridgeDir::log_dir()`, re-read every 2 s while shown.
- Depends on the corundum-framework release with `activate_with` / `WindowOptions::on_activate` / `registered_program` (plan `2026-10-31-activation-message.md` in corundum-framework); use its version.
- The relay (`RELAYED`) is unchanged. bridge-api's version is not bumped here.

## Review Focus

- `sapphire-bridge-desktop serve` started by the service manager must not take the single-instance lock or open a window.
- A desktop app whose bridge service was registered from the headless binary: `bridge_desktop_exe` finds `sapphire-bridge-desktop` next to it, else falls through.
- Bridge not running when the bridge window opens: banner + Start, no panic, pages show "not running".
- An app's summary when the bridge is incompatible (old API): shows "incompatible" with the update hint, buttons still offered.
- Clicking a button while no executable exists after the snapshot said it did: error line, no panic.

---

### Task 1: Split `bridge/` into `bridge/headless`; share the embed factory

**Files:**
- Move: `bridge/{Cargo.toml,src/main.rs,README*.md,CHANGELOG.md,assets,docs}` → `bridge/headless/…`
- Move: `bridge/src/embed.rs` → `crates/sapphire-framework-bridge-embed/src/factory.rs` (pub `factory() -> sapphire_bridge::EmbedFactory`), so both binaries use it.
- Modify: root `Cargo.toml` members (`bridge/headless`), `release-plz.toml` (package `sapphire-bridge-headless` in the framework group, replacing `sapphire-bridge`), `.github/workflows/release-bridge.yml` (`package: sapphire-bridge-headless`, binary unchanged), `.github/workflows/release-plz.yml` (the `contains(... 'sapphire-bridge')` check → `'sapphire-bridge-headless'`)
- Modify: `bridge/headless/Cargo.toml` `name = "sapphire-bridge-headless"`, `[[bin]] name = "sapphire-bridge" path = "src/main.rs"`; README top note: "Published as `sapphire-bridge-headless`; the crate `sapphire-bridge` stopped at 0.23.x."

- [ ] **Step 1:** Move with `git mv`; adjust paths (`include_str!`/asset paths in the manifest/README).
- [ ] **Step 2: Verify** `cargo build -p sapphire-bridge-headless` → `target/debug/sapphire-bridge` exists; `cargo test -p sapphire-bridge-headless` → PASS; `cargo test -p sapphire-framework-bridge-embed` → PASS.
- [ ] **Step 3: Commit** `refactor!: bridge/ → bridge/headless, package sapphire-bridge-headless`

### Task 2: corundum release and `FrameworkClient` bridge-only mode

**Files:**
- Modify: root `Cargo.toml` (corundum-framework-* to the new release; add `corundum-framework-service` same version)
- Modify: `crates/sapphire-framework-gui/src/client/{types.rs,conn.rs,exec.rs,mod.rs}`

**Interfaces:**
- Produces:
  - `pub enum ClientMode { App, BridgeOnly }` and `ClientConfig { …, pub mode: ClientMode, pub bridge_pages: BridgePages }`; `ClientConfig::new` → `mode: App`, `bridge_pages: InApp`.
  - `ClientConfig::bridge_only(version: &'static str) -> std::io::Result<ClientConfig>` (app identity `sapphire-bridge-desktop`).
  - `pub enum BridgePages { InApp, Linked }` (in `client/types.rs`, re-exported at crate root).
  - In `BridgeOnly`, `fetch_bridge` uses `BridgeClient::connect_running("gui", version)`; `snapshot.server = Conn::Absent`; bridge commands use that client; any app-server command returns `CommandError` "no app server in the bridge's window".
  - In `App` mode with `bridge_pages == Linked`, `fetch_bridge` skips `embed_settings` and `workspaces` (fields `None` / empty).

- [ ] **Step 1: Failing tests** in `client/` tests (in-process test bridge, as existing client tests):
  - `bridge_only_fetches_the_bridge_without_a_server`: snapshot has `bridge: Conn::Up` with status, `server: Conn::Absent`.
  - `bridge_only_commands_reach_the_bridge`: `Command::WorkgroupCreate{..}` succeeds with no app server.
  - `bridge_only_without_a_bridge_is_absent`.
  - `linked_fetches_only_the_summary`: App mode, Linked → `embedding == None`, `ledger.is_empty()`.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui --lib client` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — Expected: PASS; `cargo test -p sapphire-framework-gui` PASS.
- [ ] **Step 5: Commit** `feat(gui): FrameworkClient's bridge-only mode; corundum <version>`

### Task 3: The bridge window (`BridgePage`, Settings in bridge mode, `run_bridge`)

**Files:**
- Create: `crates/sapphire-framework-gui/src/bridge_window/{mod.rs,status.rs,logs.rs}`
- Modify: `src/settings.rs` (bridge mode: `Settings::for_bridge(client) -> Settings`, `open_page(BridgePage)`), `src/lib.rs` (`pub mod bridge_window` behind `eframe` for `run_bridge`; `BridgePage` always), `locales/{en,ja}/sapphire.ftl`

**Interfaces:**
- Consumes: `ClientConfig::bridge_only` (Task 2); corundum `instance::acquire_or_activate_with`, `WindowOptions::{instance, on_activate}`.
- Produces:
  - `pub enum BridgePage { Status, Workgroup, Devices, Embedding, Appearance, Logs, About }` with `parse(&str) -> Option<BridgePage>`, `as_str(self) -> &'static str` (values as Global Constraints).
  - `pub struct BridgeWindowOptions { pub title: String, pub version: &'static str, pub page: BridgePage }`; `pub fn run_bridge(o: BridgeWindowOptions) -> eframe::Result<()>` — takes the lock `sapphire-bridge-desktop` with `acquire_or_activate_with(.., Some(page.as_str()))`; `None` → return `Ok(())`.
  - `StatusView` (status.rs): version/state, device name, workgroup name, devices (connected marked), workspaces + roles, Restart → `Command::Restart(Process::Bridge)`.
  - `LogsView::new(dir: PathBuf)` (logs.rs): `fn tail(&mut self) -> Result<&[String], String>` reads the newest file's last 500 lines, at most every 2 s.
  - Not-running banner Start: the existing `Command::ServiceInstall(ServiceTarget::Bridge)` (install & start), with `ClientConfig::bridge_only` setting `service_exes.bridge` to the running `sapphire-bridge-desktop`, so the service it registers runs `sapphire-bridge-desktop serve`.
- Strings (en/ja): `sapphire-bridge-window-title` "Sapphire Bridge"/"Sapphire Bridge"; `sapphire-bridge-page-status` "Status"/"状態"; `-logs` "Logs"/"ログ"; `sapphire-bridge-logs-open-folder` "Open the folder"/"フォルダーを開く"; `sapphire-bridge-logs-unreadable` "Could not read { $path }: { $error }"/"{ $path } を読めませんでした: { $error }"; `sapphire-bridge-status-restart` "Restart"/"再起動"; `sapphire-bridge-start` "Start"/"起動".

- [ ] **Step 1: Failing tests:**
  - `bridge_page_round_trips`: every variant `parse(as_str())`; `parse("nope") == None`.
  - `bridge_settings_lists_the_bridge_pages`: `Settings::for_bridge` over a test client; its page list (test accessor) == Status, Workgroup, Devices, Embedding, Appearance, Logs, About.
  - `open_page_from_activation`: `open_page(BridgePage::Devices)` then render → drawn page Devices; an unparsable message leaves the page.
  - `logs_tail_reads_the_newest_file`: tempdir with `a.log` (old) and `b.log` (newer, 600 lines) → 500 lines, last one is b's last; unreadable dir → `Err` containing the path.
  - `status_renders_without_a_bridge`: `StatusView` over `Snapshot::default()` renders, emits no command.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui --lib bridge_window settings` — Expected: FAIL.
- [ ] **Step 3: Implement.** `on_activate` callback: parse the message, send it over an `mpsc` the app polls each frame, then `open_page`.
- [ ] **Step 4: Run** `cargo test -p sapphire-framework-gui` — Expected: PASS (i18n check included).
- [ ] **Step 5: Commit** `feat(gui): the bridge's window`

### Task 4: `bridge/desktop`

**Files:**
- Create: `bridge/desktop/{Cargo.toml,src/main.rs,README.md}`; root `Cargo.toml` members; `release-plz.toml` (desktop in the group, `publish = false` in manifest).
- Test: `bridge/desktop/tests/cli.rs`

**Interfaces:**
- Consumes: `BridgeCommand::dispatch_with(version, Some(sapphire_framework_bridge_embed::factory()))`, `run_bridge` (Task 3).
- Produces: CLI `Desktop { #[command(subcommand)] command: Option<BridgeCommand>, #[arg(long, value_parser = parse_page, default_value = "status")] page: BridgePage }`. With a subcommand: dispatch (no window, no lock). Without: `run_bridge`; on `Err`, print `sapphire-bridge-desktop: cannot open the window: {err}` and the help to stderr, exit 1.

- [ ] **Step 1: Failing tests** (`tests/cli.rs`, process tests with `env!("CARGO_BIN_EXE_sapphire-bridge-desktop")`):
  - `a_subcommand_runs_the_cli`: `sapphire-bridge-desktop --version` exit 0 printing the version; `status` with `SAPPHIRE_RUNTIME_DIR` → empty tempdir → the same output and exit code as `sapphire-bridge status`.
  - `page_is_parsed`: unit test `Desktop::try_parse_from(["x","--page","devices"])` → `BridgePage::Devices`; `--page nope` → error.
- [ ] **Step 2: Run** `cargo test -p sapphire-bridge-desktop` — Expected: FAIL.
- [ ] **Step 3: Implement.** Console subsystem on Windows (no `windows_subsystem` attribute), as `sapphire-journal`'s `desktop/src/main.rs`; the Velopack console shim is part C.
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** `feat: sapphire-bridge-desktop`

### Task 5: Apps link to the bridge (`BridgePages::Linked`)

**Files:**
- Create: `crates/sapphire-framework-gui/src/views/bridge_summary.rs`, `src/platform/bridge_exe.rs`
- Modify: `src/settings.rs` (Linked hides Devices/Workgroup/Embedding, shows the summary), `src/app/mod.rs` (`bridge_pages: Linked`), `src/mobile/mod.rs` (`InApp`), locales.

**Interfaces:**
- Consumes: `ClientConfig.bridge_pages` (Task 2), `BridgePage` (Task 3), corundum `service::registered_program`.
- Produces:
  - `pub fn bridge_desktop_exe() -> Option<PathBuf>`; `pub fn bridge_desktop_exe_with(service_exe: Option<&Path>, own_exe: Option<&Path>, path_var: Option<&OsStr>) -> Option<PathBuf>` (order per spec; name `sapphire-bridge-desktop{EXE_SUFFIX}`).
  - `BridgeSummary { launcher: Box<dyn Fn(&Path, BridgePage) -> std::io::Result<()>>, exe: Option<PathBuf>, error: Option<String> }`; default launcher spawns `exe --page <page>` detached (stdio null).
- Strings (en/ja): `sapphire-bridge-summary-title` "Sapphire Bridge"; `-no-workgroup` "No workgroup"/"ワークグループなし"; `-connected` "{ $connected } of { $total } devices connected"/"{ $total } 台中 { $connected } 台が接続中"; `-open` "Open Sapphire Bridge"/"Sapphire Bridge を開く"; `-workgroup` "Workgroup…"/"ワークグループ…"; `-devices` "Devices…"/"デバイス…"; `-embedding` "Embedding…"/"埋め込み…"; `-get` "Get Sapphire Bridge"/"Sapphire Bridge を入手"; `-cli-hint` "Or use the command line: sapphire-bridge workgroup …"/"コマンドラインでは sapphire-bridge workgroup … で操作できます"; `-launch-failed` "Could not open Sapphire Bridge: { $error }"/"Sapphire Bridge を開けませんでした: { $error }". "Get" links to `https://github.com/sapphire-turtle/sapphire-framework/releases/latest`.

- [ ] **Step 1: Failing tests:**
  - `bridge_desktop_exe_order`: tempdirs: (a) service exe `…/x/sapphire-bridge` with `…/x/sapphire-bridge-desktop` present → that; (b) service exe is the desktop binary itself → it; (c) none at service, present beside own exe → that; (d) only on PATH → that; (e) none → `None`.
  - `linked_settings_has_no_bridge_pages`: page list lacks Devices/Workgroup/Embedding, has the summary; `InApp` has them.
  - `summary_shows_workgroup_and_connected`: snapshot with workgroup "home", 3 peers (2 connected) → summary text contains "home" and "2 of 3".
  - `buttons_launch_with_the_page`: fake launcher records `(exe, page)`; trigger Workgroup → `(exe, BridgePage::Workgroup)` (test hook `press(BridgePage)`).
  - `no_exe_offers_get`: `exe: None` → the get row (test accessor `shows_get()`).
  - `launch_error_is_shown`: launcher returning `Err` → `error` set with the message.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** `feat(gui)!: desktop Settings link to Sapphire Bridge`

### Task 6: Remove the apps' `workgroup`, `device`, `embedding`

**Files:**
- Modify: `crates/sapphire-framework-server/src/command.rs` (variants, dispatch, helpers only they used), `crates/sapphire-framework-server/src/lib.rs` and `crates/sapphire-framework/src/lib.rs` (re-exports of `WorkgroupCommand`, `DeviceCommand` from the server go; the bridge's stay), tests referencing them.

- [ ] **Step 1: Failing test** `crates/sapphire-framework-server/tests/cli_removed.rs` (test-util, as `cli_api_key`): `server-test-app workgroup show` / `device list` / `embedding show` → non-zero exit, stderr contains "unrecognized subcommand"; `config list` → exit 0.
- [ ] **Step 2: Run** — Expected: FAIL (the subcommands still exist).
- [ ] **Step 3: Implement**; `cargo check --workspace --all-targets --all-features`.
- [ ] **Step 4: Run** `cargo test -p sapphire-framework-server --all-features` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(cli)!: the apps' workgroup, device and embedding moved to sapphire-bridge`

### Task 7: Release workflow, docs

**Files:**
- Modify: `.github/workflows/release-bridge.yml` (second build: `package: sapphire-bridge-desktop`, `binary: sapphire-bridge-desktop`, linux packages for eframe), `.github/workflows/ci.yml` (no change unless a feature list names the bridge), `docs/ARCHITECTURE.md` (bridge window, Linked/InApp, CLI), root `README*.md` (bridge install lines: `sapphire-bridge-headless`, desktop binary).

- [ ] **Step 1:** `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; CI per-feature `cargo check` loop; `cargo test --workspace --all-features --no-fail-fast` — Expected: all pass.
- [ ] **Step 2: Commit** `docs: Sapphire Bridge's window`
