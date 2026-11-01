# Bridge Package Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sapphire Bridge ships as a Velopack package (Windows Setup, macOS `.pkg`, Linux AppImage) that asks to keep itself running, puts `sapphire-bridge` on `PATH`, and updates itself.

**Architecture:** `bridge/desktop` gains a `velopack` feature: `PACKAGE` (corundum `PackageSpec`), a start sequence (`package::start`, lock, updater), a Windows console shim, and `update` through the package's updater. `sapphire-framework-gui` gains a `package` feature: the bridge window's first-run flow (welcome + `ServicePage`) and About's `ServiceRow` + `UpdatesPanel`, fed by `BridgeWindowOptions::package`. `release-bridge.yml` packs and uploads with `vpk`.

**Tech Stack:** Rust 2024, corundum-framework 0.6 (`package` with `service`, `gui` features `package`/`service`/`update`/`serde`), Velopack `vpk` 1.2.161 (pinned as corundum's demo), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-01-bridge-package-design.md`

## Global Constraints

- Package id `sapphire-bridge`, title "Sapphire Bridge", authors "sapphire-turtle"; command on PATH `sapphire-bridge`.
- Update source: `Source::Github { repo: "https://github.com/sapphire-turtle/sapphire-framework", prerelease: false }`; check every 24 h.
- macOS bundle id `org.sapphire-turtle.sapphire-bridge`; Linux `--categories Network`.
- Main exe: `sapphire-bridge-desktop` (Windows `.exe`, GUI subsystem under `velopack`); shim `sapphire-bridge-console` packed as `sapphire-bridge.exe` (Windows only).
- Upload to the framework's `v<version>` release.
- First-run strings (en / ja):
  - welcome title "Welcome to Sapphire Bridge" / "Sapphire Bridge へようこそ";
  - welcome body "Sapphire Bridge syncs your apps' workspaces between your devices, and keeps the device's settings for them." / "Sapphire Bridge は、アプリのワークスペースをデバイス間で同期し、このデバイスの設定を保ちます。";
  - service question "Keep Sapphire Bridge running in the background?" / "Sapphire Bridge をバックグラウンドで動かし続けますか？";
  - reason "Sync and the apps need it while their windows are closed." / "アプリのウィンドウを閉じている間も、同期とアプリが使います。"
- Without `velopack`, `sapphire-bridge-desktop` behaves exactly as part B (no updater, no first run, `update` refused).

## Review Focus

- `sapphire-bridge-desktop serve` started by the service manager inside the package: no lock, no updater, no first run, no window.
- A Velopack hook start (`--veloapp-install` etc.) exits at once without opening a window.
- A second window start while an update is pending does not apply it (only the instance applies).
- `sapphire-bridge.exe` (shim) with no arguments prints help and returns its exit code; Ctrl+C ends the child.
- The first-run flow runs once: after finishing (Yes or "continue without"), the next start shows Settings.

---

### Task 1: `PACKAGE` and the start sequence (`bridge/desktop`, feature `velopack`)

**Files:**
- Create: `bridge/desktop/src/package.rs`
- Modify: `bridge/desktop/Cargo.toml` (feature `velopack = ["dep:corundum-framework-package", "sapphire-framework-gui/package"]`, dep `corundum-framework-package = { version = "0.6.0", features = ["service"], optional = true }`), `bridge/desktop/src/main.rs`

**Interfaces:**
- Produces (`package.rs`, `#[cfg(feature = "velopack")]`):
  - `pub static PACKAGE: PackageSpec` (values per Global Constraints; `services: Services::new(&[bridge_service])`).
  - `pub fn bridge_service() -> ServiceSpec` = `bridge_service_spec(env!("CARGO_PKG_VERSION"))`.
  - `pub const REPO: &str`, `pub const CHECK_EVERY: Duration`.
  - `pub fn start_updater() -> Option<Updater>`: `Updater::new(&PACKAGE, source)`; `apply_pending_at_start()`; `run_in_background(CHECK_EVERY)`; `None` outside a package or on error (logged).
- `main` order with `velopack`: `package::start(&PACKAGE)` → `Start::Done` exits 0; subcommand → as now; bare → `run_bridge` with `package: Some(PackageWindow { updater, service: bridge_service })` where `updater = start_updater()` is made **after** `run_bridge` takes the lock (Task 2's `BridgeWindowOptions::on_instance` hook).

- [ ] **Step 1: Failing test** in `package.rs`: `package_names_the_bridge_and_its_service` — `PACKAGE.app_name == "sapphire-bridge"`, `PACKAGE.cli.unwrap().command == "sapphire-bridge"`, `bridge_service().args == ["serve"]`, `bridge_service().app_name == "sapphire-bridge"`.
- [ ] **Step 2: Run** `cargo test -p sapphire-bridge-desktop --features velopack package` — FAIL.
- [ ] **Step 3: Implement** `package.rs` and the `main` order.
- [ ] **Step 4: Run** — PASS; `cargo test -p sapphire-bridge-desktop` (no feature) — PASS.
- [ ] **Step 5: Commit** `feat(bridge-desktop): the Velopack package and its start`

### Task 2: The window's package parts (`sapphire-framework-gui`, feature `package`)

**Files:**
- Create: `crates/sapphire-framework-gui/src/bridge_window/first_run.rs`, `src/bridge_window/about.rs`
- Modify: `crates/sapphire-framework-gui/Cargo.toml` (feature `package = ["eframe", "corundum-framework-gui/package", "corundum-framework-gui/service", "corundum-framework-gui/update"]`), `src/bridge_window/{mod.rs,window.rs}`, `src/settings.rs` (About in the bridge window shows the package parts), locales.

**Interfaces:**
- Produces:
  - `pub struct PackageWindow { pub updater: Option<Updater>, pub service: fn() -> ServiceSpec }` (feature `package`).
  - `BridgeWindowOptions { …, pub package: Option<PackageWindow> }` (field present only with `package`).
  - `pub fn first_run_pages(service: fn() -> ServiceSpec) -> Vec<Box<dyn FirstRunPage>>` → `[WelcomePage, ServicePage::new(vec![service], question, reason)]`.
  - `Settings::with_package(self, package: PackageWindow) -> Settings` (bridge window About: `ServiceRow::new(service)` and `UpdatesPanel::new(vec![Target::this_app("Sapphire Bridge".into(), updater, binary_target)], user_agent)`).
  - In `run_bridge`: when `package` is `Some` and `FirstRun::done(ctx)` is false, the window draws `FirstRun::new("Sapphire Bridge", first_run_pages(service))` until its `ui` returns true, then Settings.
- Strings: the first-run strings in Global Constraints, ids `sapphire-bridge-welcome-title`, `-welcome-body`, `sapphire-bridge-service-question`, `-service-reason`.

- [ ] **Step 1: Failing tests:**
  - `first_run_is_welcome_then_service`: `first_run_pages(svc)` has 2 pages; titles (english ctx) are "Welcome to Sapphire Bridge" and "Keep Sapphire Bridge running in the background?".
  - `about_shows_the_service_and_updates`: `Settings::for_bridge(client).with_package(PackageWindow { updater: Some(Updater::for_tests(PackageState::UpToDate)), service: svc })`; render About → drawn text contains the service row's label and the update panel's "Sapphire Bridge" row.
  - i18n check passes.
- [ ] **Step 2: Run** `cargo test -p sapphire-framework-gui --features package --lib bridge_window` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — PASS; `cargo test -p sapphire-framework-gui` (no feature) — PASS.
- [ ] **Step 5: Commit** `feat(gui): the bridge window's first run and About in a package`

### Task 3: Windows GUI subsystem and the console shim

**Files:**
- Create: `bridge/desktop/src/bin/console.rs`
- Modify: `bridge/desktop/Cargo.toml` (`[[bin]] name = "sapphire-bridge-console" path = "src/bin/console.rs" required-features = ["velopack"]`), `bridge/desktop/src/main.rs` (`#![cfg_attr(all(windows, feature = "velopack"), windows_subsystem = "windows")]`)

- [ ] **Step 1:** `console.rs`: `fn main() -> ! { corundum_framework_package::console::forward("sapphire-bridge-desktop.exe") }` on Windows; elsewhere print that the shim is Windows-only and exit 1.
- [ ] **Step 2: Verify** `cargo build -p sapphire-bridge-desktop --features velopack --bins` (Linux) — builds both bins; `cargo check -p sapphire-bridge-desktop --features velopack --target x86_64-pc-windows-gnu` if the target is installed, else rely on CI's Windows pack job.
- [ ] **Step 3: Commit** `feat(bridge-desktop): the Windows console shim`

### Task 4: `update` in a package

**Files:**
- Modify: `bridge/desktop/src/main.rs`, `bridge/desktop/src/package.rs`

**Interfaces:**
- Produces: `pub fn package_update(updater: &Updater, check_only: bool, out: &mut dyn Write, timeout: Duration) -> i32` — `check_now()`, wait while `Checking`/`Downloading` (poll 200 ms, up to `timeout`); `UpToDate` → print "Sapphire Bridge <v> is the newest", 0; `Ready { version }` → check_only: print "Sapphire Bridge <version> is ready; run `sapphire-bridge update` to install it", 0; else `apply(After::Exit)` (exits) — error → print, 1; `Failed(e)` → print, 1; timeout → print, 1.
- `main`: `BridgeCommand::Update(args)` in a package (`package::current().is_some()`) → `package_update(&updater, args.check, stdout, 5 min)`; outside → part B's refusal.

- [ ] **Step 1: Failing tests** (`Updater::for_tests`):
  - `up_to_date_says_so`: `UpToDate`, check → 0, output contains "newest".
  - `ready_with_check_reports_the_version`: `Ready { version: 9.9.9 }`, check → 0, output contains "9.9.9".
  - `ready_without_check_applies`: `Ready`, apply → 0 (for_tests' apply returns Ok).
  - `failed_is_an_error`: `Failed("x")` → 1, output contains "x".
- [ ] **Step 2: Run** `cargo test -p sapphire-bridge-desktop --features velopack update` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — PASS; part B's refusal test (no feature) still PASS.
- [ ] **Step 5: Commit** `feat(bridge-desktop): update through the package`

### Task 5: Icons and the `pack` job

**Files:**
- Create: `bridge/desktop/examples/icons.rs` (writes `icon.ico`, `icon.icns`, `icon.png` 512 px from `../headless/assets/app-icon.svg`, as corundum's demo's), `bridge/desktop/Cargo.toml` dev/regular dep `corundum-framework-platform` (feature `icon`) for the example.
- Modify: `.github/workflows/release-bridge.yml` — job `pack` (needs nothing else; builds itself), matrix linux/macOS/windows per the spec table, steps mirroring corundum's `demo-release.yml`: toolchain, Linux deps (`libwayland-dev libxkbcommon-dev`), dotnet 8, `vpk` `1.2.161`, `cargo build --release --locked -p sapphire-bridge-desktop --features velopack --bins`, icons, macOS signing when `MACOS_CERTIFICATE` is set, `vpk pack` (`--packId sapphire-bridge --packVersion ${tag#v} --packDir pack --mainExe <main> --packTitle "Sapphire Bridge" --packAuthors sapphire-turtle --icon icons/<icon> --channel <channel> --delta None --outputDir releases`), copying the shim as `sapphire-bridge.exe` on Windows; a Linux smoke step `./releases/*.AppImage --version`; artifact upload; then a `release` job: download artifacts and `vpk upload github --repoUrl https://github.com/sapphire-turtle/sapphire-framework --token ${{ secrets.GITHUB_TOKEN }} --tag ${{ inputs.tag }} --releaseName ${{ inputs.tag }} --merge` per channel.

- [ ] **Step 1:** `cargo run -p sapphire-bridge-desktop --example icons -- /tmp/x` writes three files (check sizes > 0).
- [ ] **Step 2:** YAML parses (`python3 -c 'import yaml; yaml.safe_load(open(...))'`); `actionlint` if available.
- [ ] **Step 3: Commit** `ci: pack Sapphire Bridge with Velopack on release`

### Task 6: Docs and checks

**Files:**
- Modify: `bridge/desktop/README.md` (installers, first run, updates), root `README*.md` install section (Sapphire Bridge installer first), `docs/ARCHITECTURE.md` (the package), the spec's by-hand checklist copied into `docs/release-checklist-bridge.md`.

- [ ] **Step 1:** `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `cargo test --workspace --all-features --no-fail-fast`; CI per-feature checks.
- [ ] **Step 2: Commit** `docs: Sapphire Bridge's installer`
