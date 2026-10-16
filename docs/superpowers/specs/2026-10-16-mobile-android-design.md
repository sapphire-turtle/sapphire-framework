# Mobile (Android first) — design

Status: approved in conversation 2026-10-16. Spans sapphire-framework and sapphire-sync.

## 1. Goal

Run sapphire-sync on a phone, experimentally. iOS is the real target, but it needs a Mac
to build and test, so Android comes first — built so that almost everything it adds is
what iOS runs too.

Success for this round, on an Android emulator (and a device when one is at hand):

1. The app starts, hosts its own bridge and app server in-process, and shows the
   framework's `Shell` (bottom bar on a phone).
2. It joins an existing workgroup with an invite ticket pasted from the clipboard.
3. It creates a workspace in app-private storage (or maps an existing one of the
   workgroup) and syncs it with a desktop peer.
4. A **Files** screen browses the synced workspace and previews text files read-only.
   The OS gives users no other way to see an app's private directory.

Out of scope: building for iOS, background sync, editing files, QR pairing, store
distribution and signing.

## 2. Decisions

1. **egui everywhere.** The UI the desktop runs is the UI the phone runs: one Rust crate
   per app, started by a thin platform entry (Android `cdylib` + GameActivity; later iOS
   `staticlib` + a small Xcode wrapper). No UniFFI, no native UI.
2. **No Unix sockets on mobile.** iOS container paths carry several UUIDs and overflow
   `sun_path` (104 bytes); this broke an earlier attempt. Android adopts the in-process
   carrier now, so iOS inherits a path already proven on a device. (§3)
3. **Same code path otherwise.** The bridge, the app server and the GUI's
   `FrameworkClient` run exactly as on desktop — handshakes, JSON-RPC framing, the data
   plane's raw streams — only the carrier underneath differs.
4. **Foreground only.** The bridge and the server live as long as the process. Nothing
   syncs while the OS has the app suspended or killed; the next start resumes.
5. **The file browser is the framework's.** Every app on a phone needs it, so it lives in
   `sapphire-framework-gui` and the desktop uses it too.

## 3. In-process carrier (`sapphire-framework-ipc`)

Every IPC user goes through four functions keyed by an `Endpoint` (`dir` + `name`):
`bind`, `connect`, `connect_raw`, `probe`. The carrier is added beneath them.

- `sapphire_ipc::Carrier { Os, InProcess }`, and a process-wide choice:
  `sapphire_ipc::set_carrier(Carrier::InProcess)`, called once at start-up before anything
  binds. The default stays `Os` (Unix socket / named pipe).
- An in-process registry: `Mutex<HashMap<(PathBuf, String), mpsc::Sender<DuplexStream>>>`.
  - `bind` under `InProcess` registers the endpoint and returns a listener; a second bind
    of a live endpoint is "address in use", as with sockets. Dropping the listener
    unregisters it.
  - `connect`/`connect_raw` look the endpoint up first (whatever the global choice, so a
    test can mix carriers) and, when found, create a `tokio::io::duplex` pair, hand one
    end to the listener and keep the other. Not found → the OS carrier under `Os`,
    `NotRunning` under `InProcess`.
  - `probe` is "is it registered and its listener alive".
- The listener type becomes an enum over the platform listener and the in-process one, so
  `bind(..).accept()` / `accept_raw()` keep their signatures. (On Windows `bind` is sync
  today; the in-process arm fits either shape.)
- Bytes still flow: `Connection::from_io` frames NDJSON over the duplex exactly as over a
  socket, and the data plane's `RawStream` is the duplex itself. No same-user check is
  needed — the peer is this process.
- `Endpoint.dir` is then only a namespace. `runtime_dir()` still resolves (mobile sets
  `SAPPHIRE_RUNTIME_DIR`), but no file is created for an endpoint.

Tests: the existing unix-carrier tests are mirrored for the in-process carrier (reach the
listener, raw bytes, address in use, probe after drop), plus one integration test that
runs a real `Bridge` + `AppServer` + `FrameworkClient` in one process under `InProcess`
on Linux CI — the mobile topology without a phone.

## 4. Hosting on a phone (`sapphire-framework-gui`, feature `mobile`)

`gui::mobile` is what an app's mobile crate calls:

```rust
pub struct MobileOptions { app: AppIdentity, title: String, fonts: Option<InstallFonts> }

/// What the app supplies: its sync-mounted server, built after the directories are set.
pub type BuildServer = Box<dyn FnOnce() -> anyhow::Result<AppServer> + Send>;

#[cfg(target_os = "android")]
pub fn run_android(app: AndroidApp, options: MobileOptions, server: BuildServer,
                   create: impl FnOnce(AppSetup<'_>) -> Box<dyn eframe::App>) -> eframe::Result<()>;
// later: run_ios(...) with the same shape
```

Shared core, `MobileHost::start(base_dir, app_name, server, runtime)`:

1. Directories. `dirs` resolves nothing on Android/iOS (it reads only `$HOME`), so the
   host sets, before any thread exists: `SAPPHIRE_RUNTIME_DIR`, `SAPPHIRE_BRIDGE_DIR`,
   and `SAPPHIRE_<APP>_{DATA,CACHE,CONFIG}_DIR`, all under `base_dir` (Android:
   `AndroidApp::internal_data_path()`; iOS: Application Support). `AppContext::init`
   runs with `AppKind::Server` — this process is the server.
2. `sapphire_ipc::set_carrier(Carrier::InProcess)`.
3. The bridge: `BridgeDir::open()`, `InstanceLock`, the iroh transport and `Bridge::run`,
   spawned on the runtime — the body of `sapphire-bridge serve`, exposed from
   `sapphire-framework-bridge` as `serve_in_process(version) -> JoinHandle`. No
   embedding provider.
4. The app server: the app's `BuildServer`, `.run()` spawned on the runtime. Its sync
   runtime connects to the bridge through the same endpoints.
5. The `FrameworkClient`, as on desktop.

A start failure (a bad directory, the bridge or server ending with an error) is logged
and shown by the existing health/banner views: `Settings` already reports a bridge or
server that is absent or erroring.

Logging: `tracing` to logcat on Android (`android_logger`-style layer), stderr elsewhere.

### Desktop-only pieces

- `rfd` (folder dialog) becomes a desktop-only dependency. On mobile the workspace views
  offer **create in app storage** instead: a name field; the folder is
  `<base_dir>/workspaces/<name>`. Mapping a workgroup workspace picks the same default.
- `eframe` gains `android-game-activity` behind the `mobile` feature on Android.
- Fonts: `fonts::install_system_cjk_fallback` must find `/system/fonts` CJK fonts on
  Android (fontdb's system scan skips Android, so the directory is loaded explicitly).
- Clipboard: egui-winit has none on Android. A small `gui::platform::Clipboard` trait,
  used by the ticket field's **Paste** button: Android via JNI (`ClipboardManager`), later
  iOS via `UIPasteboard`, desktop via egui's own.

## 5. File browser (`gui::views::FileBrowser`)

- Root: the current workspace's root from the server snapshot. No workspace → a hint
  pointing at Settings → Workspace.
- List: one directory at a time; folders first, then files, by name (case-insensitive).
  Hidden: names starting with `.` (the app's marker directory, `.sapphireignore`, editor
  noise). Breadcrumbs to go up.
- Preview: read off the UI thread. UTF-8 up to 1 MiB → scrollable monospace text,
  selectable; larger → the first 1 MiB and a note; not UTF-8 → "binary file, N bytes".
- Layout: below `COMPACT_WIDTH` the list and the preview are separate pages (back
  returns to the list); above it, two panes.
- Refresh: a button re-reads the directory and the open file. No watching.
- Paths never leave the root: entries come from `read_dir` of a directory under the
  root, and symlinks are shown but not followed.

Tests: the pure parts — sorting, hiding, text/binary classification and truncation,
breadcrumbs — are unit-tested; listing and reading run against a temp directory.

## 6. sapphire-sync

- `sapphire-sync-core` takes `build_serve` (and `sync::status_rows`) from the CLI, so the
  CLI and the phone mount the same server.
- `crates/sapphire-sync-ui`: the app (`Shell` with **Files** → `FileBrowser`), used by
  `desktop/` and `mobile/`. The desktop's placeholder screen goes.
- `mobile/`: crate `sapphire-sync-mobile` (`cdylib`, `staticlib`), `android_main`
  calling `gui::mobile::run_android`. `mobile/android/` is a Gradle project with
  GameActivity and no app code of its own; `cargo ndk` builds the `.so` into
  `jniLibs`. `mobile/README.md` documents the toolchain and where the iOS wrapper goes.
- Not a workspace default member, so desktop CI does not need the NDK. A CI job for the
  Android build is a follow-up.

## 7. Toolchain (developer machine)

Installed per-user under `~/Android` (no root): a JDK 17, Android SDK command-line tools,
platform-tools, a platform, build-tools, the NDK, and an x86_64 system image + emulator
(KVM). Rust targets `aarch64-linux-android` and `x86_64-linux-android`; `cargo-ndk`.

## 8. Risks

- iroh (netwatch) on Android: interface enumeration has needed workarounds before. If the
  node does not come up, that is the first thing to look at.
- IME: winit's GameActivity backend may deliver Japanese input imperfectly. The ticket
  field is covered by Paste; workspace names by the soft keyboard's direct input.
- `AppServer::run` and `Bridge` assume a process they own (signals, `current_exe` for
  routes, update checks). The host must not install signal handlers on mobile, and
  `exe_path` in routes is meaningless there — wake-on-sync never fires, since the server
  is always up while the process is.
- Build time and APK size (iroh + redb + egui + wgpu).
