# Mobile (Android first) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** sapphire-sync runs on Android (emulator first) with its bridge and server in-process over a socket-free IPC carrier, and browses its synced workspace with a shared file browser.

**Architecture:** An in-process carrier under `sapphire_ipc::{bind, connect, connect_raw, probe}` replaces Unix sockets on mobile; `gui::mobile` hosts bridge + `AppServer` + `FrameworkClient` in one process and opens an eframe window through GameActivity. The UI (Shell, Settings, FileBrowser) is the desktop's.

**Tech Stack:** Rust 1.98, tokio, egui/eframe 0.36 (`android-game-activity`), iroh, `jni`/`ndk-context`, cargo-ndk, Gradle + AGP, Android SDK 35 / NDK 28.

**Spec:** `docs/superpowers/specs/2026-10-16-mobile-android-design.md`

## Global Constraints

- Framework work on branch `feat/mobile-android` in `sapphire-framework-wt-mobile`; sync work on `feat/mobile-android` in `sapphire-sync-wt-mobile`, depending on the framework through `[patch.crates-io]` path entries until a framework release.
- No Unix socket, named pipe or socket file is created on mobile: `set_carrier(Carrier::InProcess)` before anything binds.
- Default carrier stays `Carrier::Os`; desktop behaviour unchanged.
- egui/eframe 0.36; the gui crate's `mobile` feature is the only one pulling Android crates, and they are `cfg(target_os = "android")`.
- Preview limit 1 MiB (`PREVIEW_LIMIT = 1024 * 1024`); hidden entries are names starting with `.`.
- Mobile workspaces live in `<base_dir>/workspaces/<name>`.
- Env vars set by the mobile host: `SAPPHIRE_RUNTIME_DIR`, `SAPPHIRE_BRIDGE_DIR`, `SAPPHIRE_<APP>_{DATA,CACHE,CONFIG}_DIR` (via `app_dir_env_var`).
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` clean in each touched workspace before every commit.

## Review Focus

- A client connecting before the in-process listener is registered (app start race) must get `NotRunning`/absent, and the GUI's next refresh must connect — tested in Task 1 (`connect_before_bind_is_not_running`).
- An in-process listener dropped while clients hold streams: clients see EOF/`Closed`, a re-bind succeeds — Task 1 (`rebind_after_drop`).
- A file that is not UTF-8, empty, or larger than 1 MiB opened in the preview — Task 3 tests.
- A symlink inside the workspace pointing outside it must not be followed — Task 3 (`symlinks_are_listed_not_followed`).
- A workspace name typed on mobile containing `/`, `..` or empty must be refused rather than escaping `<base_dir>/workspaces` — Task 4 (`workspace_names_are_single_components`).

---

### Task 1: In-process IPC carrier

**Files:**
- Create: `crates/sapphire-framework-ipc/src/inproc.rs`
- Modify: `crates/sapphire-framework-ipc/src/{lib.rs,spawn.rs,unix.rs,windows.rs}`

**Interfaces:**
- Produces: `pub enum Carrier { Os, InProcess }`, `pub fn set_carrier(Carrier)`, `pub fn carrier() -> Carrier`; `pub struct Listener` (enum inside: OS listener | in-process) with `async fn accept(&mut self) -> Result<Connection>` and `async fn accept_raw(&mut self) -> Result<RawStream>`; `bind` returns `Listener` (async on unix, sync on windows as today). `UnixListenerHandle`/`PipeListener` stay exported for the OS arm.

- [ ] Step 1: tests in `inproc.rs` (all set no global; they call `inproc::bind` directly, and `connect`/`connect_raw`/`probe` must find in-process endpoints regardless of the global carrier): `a_client_reaches_the_in_process_listener` (Notification round-trip), `raw_bytes_pass_unframed` (`b"hello"`), `a_second_bind_is_address_in_use` (`io::ErrorKind::AddrInUse`), `probe_is_false_after_the_listener_drops`, `connect_before_bind_is_not_running` (with `Carrier::InProcess` semantics via `inproc::connect` → `Error::NotRunning`), `rebind_after_drop`. Plus `binding_in_process_creates_no_file` (tempdir stays empty).
- [ ] Step 2: run `cargo test -p sapphire-framework-ipc inproc` → fail to compile.
- [ ] Step 3: implement. Registry `static REGISTRY: LazyLock<Mutex<HashMap<(PathBuf, String), mpsc::Sender<DuplexStream>>>>`; duplex buffer 64 KiB; listener holds the `Receiver` and removes its key on `Drop` (only if the stored sender is its own — compare with `Sender::same_channel`). `spawn.rs` routes: in-process key present → in-process; else `Carrier::InProcess` → `NotRunning`/`false`; else OS. Update call sites in bridge (`control.rs`, `data.rs`), server (`lib.rs`, `sync/testing.rs`), workspace (`bridge_embedder.rs` tests), gui tests, `ipc-test-server` to the `Listener` type where they named the platform type (most use `let mut listener = bind(..)`, so only `mut` changes).
- [ ] Step 4: `cargo test -p sapphire-framework-ipc` and `cargo build --workspace --all-targets` pass.
- [ ] Step 5: commit `feat(ipc): an in-process carrier, chosen per process`.

### Task 2: Bridge and server hostable in-process

**Files:**
- Modify: `crates/sapphire-framework-bridge/src/command.rs` (extract), `src/lib.rs` (export)
- Modify: `crates/sapphire-framework-server/src/lib.rs` (signals option)
- Create: `crates/sapphire-framework-gui/tests/in_process.rs`

**Interfaces:**
- Produces: `sapphire_bridge::serve_in_process(version: &'static str) -> Result<tokio::task::JoinHandle<Result<()>>>` — opens `BridgeDir::open()`, takes `InstanceLock` (held by the task), installs the bridge log, builds the iroh bridge as `build_bridge` does, spawns `run()`. `command::run` calls the same internals. `AppServer::signals(self, on: bool) -> AppServer` (default `true`); with `false`, `run` waits on neither SIGTERM nor ctrl-c.

- [ ] Step 1: test `in_process.rs::a_gui_client_sees_bridge_and_server_without_sockets`: tempdir; `set_carrier(InProcess)`; endpoints `Endpoint::in_dir(BRIDGE_NAME, tmp)` / `in_dir(BRIDGE_DATA_NAME, tmp)` / `in_dir("sapphire-inproc", tmp)`; a test bridge (`Bridge::new` with `LoopbackTransport` from test-util, `.control_endpoint/.data_endpoint`, `.net(NetConfig::default())`) and an `AppServer` (`.endpoint`, `.signals(false)`, sync via `SyncRuntime` connecting to the in-process bridge endpoint) spawned; `FrameworkClient::spawn` with `ClientConfig` whose `endpoints` are those; within 10 s the snapshot has `bridge: Conn::Up(_)` and `server: Conn::Up(_)`; assert `tmp` contains no `*.sock`. Integration tests run in their own process, so the global carrier is safe.
- [ ] Step 2: run → fails (no `signals`, no in-process wiring).
- [ ] Step 3: implement `signals` and `serve_in_process`.
- [ ] Step 4: `cargo test -p sapphire-framework-gui --test in_process` and the bridge/server suites pass.
- [ ] Step 5: commit `feat: host the bridge and an app server in-process`.

### Task 3: FileBrowser view

**Files:**
- Create: `crates/sapphire-framework-gui/src/views/files/{mod.rs,model.rs,view.rs}`
- Modify: `crates/sapphire-framework-gui/src/views/mod.rs`, `src/lib.rs` (re-export `FileBrowser`)

**Interfaces:**
- Produces (`model.rs`, egui-free): `pub struct Entry { name: String, is_dir: bool, is_symlink: bool, size: u64 }`; `pub fn list_dir(root: &Path, rel: &Path) -> io::Result<Vec<Entry>>` (sorted dirs first, case-insensitive; hidden filtered; `symlink_metadata`, never followed; `rel` with `..`/absolute components → `InvalidInput`); `pub enum Preview { Text { text: String, truncated: bool }, Binary { size: u64 } }`; `pub fn read_preview(root: &Path, rel: &Path) -> io::Result<Preview>`; `pub fn breadcrumbs(rel: &Path) -> Vec<(String, PathBuf)>` (first is `("/", "")`).
- Produces (`view.rs`): `pub struct FileBrowser` with `FileBrowser::new(runtime: tokio::runtime::Handle, repaint: Arc<dyn Fn() + Send + Sync>)` and `fn ui(&mut self, ui: &mut egui::Ui, root: Option<&Path>)`; reads run in `spawn_blocking`; compact (`< COMPACT_WIDTH`) = list or preview page with a back button, wide = `SidePanel` list + central preview; Refresh button; no root → hint "No workspace yet — choose one in Settings → Workspace."

- [ ] Step 1: model tests: `dirs_come_first_then_names_case_insensitively`, `dot_entries_are_hidden`, `symlinks_are_listed_not_followed` (unix-only; link to a dir outside root: `is_symlink == true`, `is_dir == false`, and `list_dir(root, "link")` → error), `parent_components_are_refused`, `utf8_text_previews`, `empty_file_is_empty_text`, `invalid_utf8_is_binary_with_size`, `large_text_is_truncated_at_the_limit` (1 MiB + 10 bytes → `truncated`, `text.len() <= PREVIEW_LIMIT`, cut on a char boundary — use a multibyte char straddling the limit), `breadcrumbs_of_a_nested_path`.
- [ ] Step 2: run → fail.
- [ ] Step 3: implement model, then view.
- [ ] Step 4: `cargo test -p sapphire-framework-gui files` passes; clippy clean.
- [ ] Step 5: commit `feat(gui): a read-only file browser with text preview`.

### Task 4: gui `mobile` feature — platform seams

**Files:**
- Create: `crates/sapphire-framework-gui/src/platform/{mod.rs,android.rs}`, `src/mobile/{mod.rs,dirs.rs,host.rs,android.rs}`
- Modify: `crates/sapphire-framework-gui/Cargo.toml`, `src/views/workspaces.rs`, `src/manager.rs`, `src/fonts.rs`, `src/lib.rs`, `src/app/gpu.rs` if needed; facade `crates/sapphire-framework/Cargo.toml` (`mobile = ["eframe", "sapphire-framework-gui/mobile", "server", "bridge", "ipc"]`), `src/lib.rs` docs table.

**Interfaces:**
- Produces: `platform::folder_picker_available() -> bool` (false on android/ios); `rfd` under `[target.'cfg(not(any(target_os = "android", target_os = "ios")))'.dependencies]`. Workspace views: where a picker is unavailable, a name field + "Create" sends the existing create/init command with path `mobile::workspaces_dir().join(name)`; `pub fn valid_workspace_name(&str) -> bool` (non-empty, no `/`, `\`, not `.`/`..`).
- Produces: `platform::Clipboard` — `fn get_text(&self) -> Option<String>`; `platform::clipboard()` returns the Android JNI one (`ClipboardManager.getPrimaryClip().getItemAt(0).coerceToText(ctx)` via `ndk_context`) or one returning `None` elsewhere. The ticket/join field gets a **Paste** button when `ui.ctx()` has no native clipboard (android).
- Produces: `mobile::prepare_dirs(base: &Path, app_name: &str) -> io::Result<()>` (creates and `set_var`s the env vars; `unsafe` with the SAFETY note "called before any thread is spawned"); `mobile::workspaces_dir() -> PathBuf`; `mobile::MobileOptions::new(app: AppIdentity, title)`; `mobile::start(base: &Path, app: AppIdentity, server: BuildServer, runtime: &Handle) -> Result<HostHandles>` (prepare_dirs, `CTX`-free `AppContext::new(app).init(AppKind::Server)`, `set_carrier(InProcess)`, `serve_in_process`, server `.signals(false).run()` spawned); `#[cfg(target_os = "android")] mobile::run_android(app: AndroidApp, options, server, create)` building `NativeOptions { android_app: Some(app), .. }` and the same `AppSetup` as desktop. Logging: `tracing-android`-style layer → logcat (`android_log-sys` is acceptable).
- Fonts: on android, `install_system_cjk_fallback` loads `/system/fonts` with `fontdb::Database::load_fonts_dir`.

- [ ] Step 1: tests: `workspace_names_are_single_components` (`"notes"` ok; `""`, `"a/b"`, `".."`, `"."`, `"a\\b"` refused); `prepare_dirs_sets_every_variable` (tempdir; check the five vars point under it and exist).
- [ ] Step 2: run → fail.
- [ ] Step 3: implement.
- [ ] Step 4: host tests pass; `cargo check -p sapphire-framework --features mobile --target x86_64-linux-android` (with NDK env from cargo-ndk: `cargo ndk -t x86_64 check -p sapphire-framework --features mobile`) succeeds.
- [ ] Step 5: commit `feat(gui): host an app on Android`.

### Task 5: sapphire-sync — shared server builder and UI crate

**Files (sapphire-sync-wt-mobile):**
- Modify: `Cargo.toml` (members, `[patch.crates-io]` → `../sapphire-framework-wt-mobile/crates/*` for every framework crate used), `crates/sapphire-sync-core/{Cargo.toml,src/lib.rs}` + `src/serve.rs` (moved `build_serve`, `status_rows`), `cli/src/{server.rs,sync.rs,main.rs}`
- Create: `crates/sapphire-sync-ui/{Cargo.toml,src/lib.rs}`; modify `desktop/src/main.rs`.

**Interfaces:**
- Produces: `sapphire_sync_core::serve::build_serve() -> anyhow::Result<AppServer>` (feature `server` on core); `sapphire_sync_ui::App::new(setup: &AppSetup) -> App` implementing `eframe::App`, Shell with Files (→ `FileBrowser` over `framework.snapshot().server` current root) and Settings; `sapphire_sync_ui::APP: AppIdentity`.

- [ ] Step 1: existing CLI tests (`cargo test --workspace`) are the regression net; add none beyond moving.
- [ ] Step 2: move, wire desktop to `sapphire_sync_ui`, drop the placeholder Files screen and the `shell.open_settings()` default (Files is now real).
- [ ] Step 3: `cargo test --workspace`, clippy, and run the desktop against a local `sapphire-sync serve` to see Files list a workspace (screenshot).
- [ ] Step 4: commit `feat: share the server builder and the UI; Files browses the workspace`.

### Task 6: sapphire-sync Android app

**Files:**
- Create: `mobile/Cargo.toml` (`sapphire-sync-mobile`, `crate-type = ["cdylib", "staticlib"]`, lib name `sapphire_sync_mobile`), `mobile/src/lib.rs` (`#[cfg(target_os = "android")] #[unsafe(no_mangle)] fn android_main(app: AndroidApp)` → `gui::mobile::run_android(app, MobileOptions::new(APP, "Sapphire Sync"), Box::new(build_serve), |s| Box::new(App::new(&s)))`), `mobile/android/` (Gradle wrapper, `settings.gradle.kts`, `app/build.gradle.kts` with `androidx.games:games-activity`, `minSdk 26`, `targetSdk 35`, ABIs `arm64-v8a`,`x86_64`; `AndroidManifest.xml` with `GameActivity` subclass-free `android.app.lib_name = sapphire_sync_mobile`, `INTERNET` + `ACCESS_NETWORK_STATE`), `mobile/build-android.sh` (`cargo ndk -t arm64-v8a -t x86_64 -o android/app/src/main/jniLibs build -p sapphire-sync-mobile --release` then `./gradlew assembleDebug`), `mobile/README.md` (toolchain, build, run, iOS wrapper location).
- Modify: workspace `Cargo.toml` members (not default-members).

- [ ] Step 1: `mobile/build-android.sh` produces `app-debug.apk`.
- [ ] Step 2: boot emulator (`android-35 google_apis x86_64`, `-gpu swiftshader_indirect` if host GL fails), `adb install`, launch; logcat shows bridge and server started, no `.sock` under `/data/data/<pkg>/` (`adb shell run-as <pkg> find . -name '*.sock'` empty); screenshot the Shell.
- [ ] Step 3: commit `feat(mobile): an Android build of sapphire-sync`.

### Task 7: End-to-end on the emulator

- [ ] Step 1: on the host, a workgroup with a desktop `sapphire-sync serve` + bridge (scratch dirs via env vars, not the user's real ones); `device invite` → ticket; `adb shell` clipboard or type it; join from the phone; create workspace `notes` on the phone or map the desktop's; write a text file on the desktop; it appears in Files and previews. Japanese text renders.
- [ ] Step 2: record findings (iroh on Android, IME, performance) in `mobile/README.md` "Known issues"; fix what blocks the flow, each fix with its own test where testable.
- [ ] Step 3: commit; open PRs for both repos (framework first; sync's `[patch]` noted as to-be-replaced on release).
