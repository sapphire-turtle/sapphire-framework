# corundum-gui: the Android window (`android` feature)

Issue: #326. Follows the desktop window (`2026-10-23-corundum-gui-eframe-design.md`, #325),
whose Known gaps left the Android window in sapphire-framework-gui. asteria targets phones
from the start, and would otherwise copy the soft-keyboard bridge and the insets.

## Scope

- **In:** what an eframe app's window needs on Android whatever the app is — `PhoneWindow`
  (the insets and the keyboard's text), the GameTextInput bridge (`text_input`), logcat,
  and the platform calls behind them (clipboard, insets, paste button).
- **Out:** sapphire's own start-up (the bridge and the server in-process, the directories,
  `EntryGuard`), and the workspace parts of `platform`. iOS, which has no entry yet.

## Decisions

1. **A new feature `android`** in corundum-gui:
   `["eframe", "eframe/android-game-activity", "dep:android-activity", "dep:jni",
   "dep:ndk-context", "dep:android_log-sys", "dep:tracing-subscriber"]`, the dependencies
   declared for `target_os = "android"` only. On Android, eframe's default build does not
   compile (winit needs an activity chosen), so an Android app turns this on.
2. **`EntryGuard` stays in sapphire.** It must be checked before the services start, which
   is before the window; asteria will need one too, and it is moved when it does.
3. **sapphire-framework-gui's public API does not change** (`mobile::run_android`,
   `MobileOptions`, `AndroidApp`, `phone_log_filter`, `platform::*`).
4. **corundum-gui stays at 0.1.0** (still unpublished).

## Design

### corundum-gui

**`window::run_android`** (`target_os = "android"`, feature `android`):

```rust
pub use android_activity::AndroidApp;

/// Open the app's window on a phone and run it until it closes: installs the panic hook,
/// gives `platform` the activity, opens the window on wgpu (`gpu::wgpu_options`) through
/// `open`, and wraps the app `create` builds in the phone's frame (`PhoneWindow`).
pub fn run_android(
    android_app: AndroidApp,
    options: WindowOptions,   // the title and the fonts; the sizes do not apply
    create: impl FnOnce(&eframe::CreationContext<'_>) -> Box<dyn eframe::App>,
) -> eframe::Result<()>;
```

`PhoneWindow` (private) is sapphire's, moved unchanged: it reads the insets every 250 ms
and lays the app out inside them, and feeds the keyboard's text to egui through
`TextInputBridge`, resetting the GameTextInput buffer when focus moves.

**`window/text_input.rs`**: moved unchanged with its tests. It is platform-free, so the
tests run on the desktops too (`cfg_attr(not(android), allow(dead_code))` as today).

**`platform`** (always compiled): `clipboard_text() -> Option<String>`,
`window_insets() -> Option<[i32; 4]>`, `needs_paste_button() -> bool`, and
`pub(crate) set_android_activity`. The JNI half (`platform/android.rs`) is sapphire's,
moved unchanged, for `all(target_os = "android", feature = "android")`; elsewhere `None` /
`false`.

**`logcat`** (`target_os = "android"`, feature `android`):

```rust
/// The filter a phone logs with when `RUST_LOG` says nothing: the app's, corundum-gui's
/// start-up lines, every other target's warnings — and winit's Android backend at error
/// only, whose warning for a keyboard event it drops carries the text typed.
pub fn phone_log_filter(app_filter: &str) -> String;   // always compiled, so tested anywhere
/// Send this process's tracing events to logcat under `tag`, at `RUST_LOG` or
/// `phone_log_filter(app_filter)`. The first subscriber wins.
pub fn install(tag: &str, app_filter: &str);
```

`phone_log_filter(f)` is `warn,{f},corundum_gui=info,winit::platform_impl::android=error`.
The writer is sapphire's `mobile/logcat.rs`, moved.

### sapphire-framework-gui

- `Cargo.toml`: `mobile` turns on `corundum-gui/android` in place of
  `eframe/android-game-activity`. The `[target.'cfg(target_os = "android")'.dependencies]`
  (`android-activity`, `android_log-sys`, `jni`, `ndk-context`, `tracing-subscriber`) go.
- `mobile/android.rs`: `run_android` keeps its signature and its start-up (logcat, the
  entry guard, the directories, the runtime and the services, the `ClientConfig`), then
  calls `corundum_gui::window::run_android(android_app, window_options, |cc| build_app(..))`.
  `PhoneWindow` goes. `pub use corundum_gui::window::AndroidApp`.
- `mobile/logcat.rs` and `mobile/text_input.rs` go; `run_android` calls
  `corundum_gui::logcat::install(app_name, &app_filter)`.
- `phone_log_filter(app_name)` stays public:
  `corundum_gui::logcat::phone_log_filter(&format!("{},sapphire_framework_gui=info", default_log_filter(app_name)))`.
- `app::open_window` goes (`run_android` was its only user); `build_app` becomes
  `pub(crate)`.
- `platform/android.rs` goes. `platform::{clipboard_text, window_insets,
  needs_paste_button}` call corundum-gui's; `set_android_activity` goes (corundum-gui sets
  it).

### Mobile CI

android: clippy `--no-default-features` and `--features android` (in place of
`--features eframe/android-game-activity`); the emulator's tests with `--features android`.

## Testing

- `text_input`'s tests, moved (desktop and emulator).
- `phone_log_filter`: the app's filter, `corundum_gui=info`, and winit's backend at error,
  last. sapphire's `the_phone_filter_keeps_typed_text_out_of_the_log` kept as it is.
- `platform`: off Android, `clipboard_text` and `window_insets` are `None` and
  `needs_paste_button` is `false`.
- `cargo ndk -t arm64-v8a check -p sapphire-framework-gui --features mobile` passes.
- **On the emulator, end to end:** sapphire-sync's `feat/mobile-android` (its gradle
  project in `mobile/android`), pointed at this branch with a local `[patch]` that is not
  committed, built to an APK and run on the local API 36 emulator:
  - a screenshot (`adb shell screencap`) shows the app clear of the status and navigation
    bars;
  - text typed with `adb shell input text` into a field appears in it;
  - logcat shows "wgpu renderer ready".

## Known gaps

- iOS: no entry yet in sapphire; its window goes in corundum-gui with it.
- `EntryGuard` (decision 2).
