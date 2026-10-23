# corundum-gui Android window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the app-independent Android window (`PhoneWindow`, the GameTextInput bridge, logcat, clipboard/insets) from sapphire-framework-gui into corundum-gui behind a new `android` feature.

**Architecture:** corundum-gui gains `platform`, `logcat`, `window::text_input` and `window::run_android` (wrapping the app in `PhoneWindow` over `window::open`). sapphire-framework-gui's `mobile::run_android` keeps its signature and start-up (logcat, `EntryGuard`, directories, services, client config) and hands the window to corundum-gui.

**Tech Stack:** Rust, eframe 0.36 (GameActivity), android-activity 0.6, jni 0.21, ndk-context 0.1, android_log-sys 0.3, tracing-subscriber, cargo-ndk, the local API 36 emulator.

**Spec:** `docs/superpowers/specs/2026-10-23-corundum-gui-android-design.md`

## Global Constraints

- corundum-gui feature: `android = ["eframe", "eframe/android-game-activity", "dep:android-activity", "dep:jni", "dep:ndk-context", "dep:android_log-sys", "dep:tracing-subscriber"]`; those dependencies only under `[target.'cfg(target_os = "android")'.dependencies]`, `optional = true`, at the versions sapphire-framework-gui uses today (`android-activity = "0.6"`, `android_log-sys = "0.3"`, `jni = "0.21"`, `ndk-context = "0.1"`, `tracing-subscriber = { workspace = true, features = ["fmt", "env-filter"] }`).
- corundum-gui stays `0.1.0`. sapphire-framework-gui's public API unchanged: `mobile::{run_android, AndroidApp, MobileOptions, BuildServer, phone_log_filter}`, `platform::{clipboard_text, window_insets, needs_paste_button, …}`.
- `corundum_gui::logcat::phone_log_filter(f)` = `format!("warn,{f},corundum_gui=info,winit::platform_impl::android=error")`.
- Moved code is moved unchanged except paths and visibility.
- Worktree `../sapphire-framework-wt-android`, branch `feat/corundum-android`; Android SDK at `~/Android/Sdk`, AVD `corundum` in `~/.config/.android/avd`, JDK at `~/.local/jdk`, runner `.github/scripts/android-test-runner.sh`.

## Review Focus

1. **The keyboard still types on a phone.** `PhoneWindow`'s focus reset and buffer translation must survive the move; only the end-to-end check (Task 4) types into a real field.
2. **The insets still apply** (`platform::window_insets` needs the activity set before the first frame): corundum-gui's `run_android` sets it before `open`.
3. **No second tracing subscriber / lost log lines.** sapphire's `run_android` installs logcat first through corundum-gui; the filter keeps `sapphire_framework_gui=info` and adds `corundum_gui=info`.
4. **Desktop builds are untouched by `android`**: `platform` compiles everywhere; JNI code only with `all(target_os = "android", feature = "android")`.
5. **The panic hook is installed once** on Android: corundum-gui's `run_android` installs it, so sapphire's must stop.

---

### Task 1: corundum-gui `android` feature

**Files:**
- Modify: `crates/corundum-gui/Cargo.toml`, `src/lib.rs`, `src/window.rs` (becomes `src/window/mod.rs`)
- Create: `src/platform/mod.rs`, `src/platform/android.rs` (from sapphire `src/platform/android.rs`), `src/logcat.rs` (writer from sapphire `src/mobile/logcat.rs`), `src/window/text_input.rs` (from sapphire `src/mobile/text_input.rs`), `src/window/android.rs` (`PhoneWindow` from sapphire `src/mobile/android.rs`, plus `run_android`)

**Interfaces:**
- Produces:
  - `corundum_gui::platform::{clipboard_text() -> Option<String>, window_insets() -> Option<[i32; 4]>, needs_paste_button() -> bool}`; `pub(crate) set_android_activity(*mut c_void)` (android + feature)
  - `corundum_gui::logcat::phone_log_filter(app_filter: &str) -> String` (always); `corundum_gui::logcat::install(tag: &str, app_filter: &str)` (android + feature)
  - `corundum_gui::window::{AndroidApp, run_android(android_app: AndroidApp, options: WindowOptions, create: impl FnOnce(&eframe::CreationContext<'_>) -> Box<dyn eframe::App>) -> eframe::Result<()>}` (android + feature)

- [ ] **Step 1: Failing tests.** In `src/logcat.rs` (module compiled always; `install` and the writer gated):

```rust
#[test]
fn the_phone_filter_wraps_the_apps_and_keeps_typed_text_out() {
    let filter = phone_log_filter("my_app=info");
    assert!(filter.starts_with("warn,my_app=info,"));
    assert!(filter.contains(",corundum_gui=info,"));
    assert!(filter.ends_with(",winit::platform_impl::android=error"));
}
```

In `src/platform/mod.rs`:

```rust
#[cfg(not(target_os = "android"))]
#[test]
fn off_android_there_is_no_clipboard_insets_or_paste_button() {
    assert_eq!(clipboard_text(), None);
    assert_eq!(window_insets(), None);
    assert!(!needs_paste_button());
}
```

Run `cargo test -p corundum-gui --lib logcat platform` → FAIL (functions not found).
- [ ] **Step 2: Cargo and lib.rs.** Feature and deps per Global Constraints. `lib.rs`: `pub mod logcat; pub mod platform;`; crate doc mentions `android` (an Android app turns it on; eframe's default does not build there).
- [ ] **Step 3: `platform` and `logcat`.** `platform/mod.rs`: the three functions and `set_android_activity` with the bodies of sapphire's (`cfg(target_os = "android")` → `cfg(all(target_os = "android", feature = "android"))`); `platform/android.rs` moved. `logcat.rs`: `phone_log_filter`, and `install(tag, app_filter)` with sapphire's body, the filter from `phone_log_filter(app_filter)`, the tag `CString::new(tag)`; the `Logcat` writer moved.
- [ ] **Step 4: Run** the Step 1 tests → PASS.
- [ ] **Step 5: `window` as a directory.** `git mv src/window.rs src/window/mod.rs`; add `#[cfg_attr(not(all(target_os = "android", feature = "android")), allow(dead_code))] mod text_input;` (moved file and tests) and `#[cfg(all(target_os = "android", feature = "android"))] mod android; pub use android::{AndroidApp, run_android};`.
- [ ] **Step 6: `window/android.rs`.** `PhoneWindow` moved (it reads `crate::platform::window_insets()` and `super::text_input`). `run_android`: `gpu::install_panic_hook()`; `crate::platform::set_android_activity(android_app.activity_as_ptr())`; `let keyboard = android_app.clone()`; `NativeOptions { android_app: Some(android_app), wgpu_options: gpu::wgpu_options(), ..Default::default() }`; destructure `options` for `title` and `fonts` (sizes ignored — say so in the doc); `open(&title, native, fonts, move |cc| Box::new(PhoneWindow::new(create(cc), keyboard)))`. `WindowOptions` fields are private to `window`, which `window/android.rs` is inside of.
- [ ] **Step 7: Verify.**
  - `cargo test -p corundum-gui` → PASS (text_input's tests now here).
  - `cargo clippy -p corundum-gui --all-targets -- -D warnings` and with `--no-default-features` → clean.
  - With `ANDROID_NDK_HOME` set: `cargo ndk -t arm64-v8a -t x86_64 clippy -p corundum-gui --all-targets --locked --no-default-features -- -D warnings` and `… --features android -- -D warnings` → clean.
  - Emulator: `CARGO_TARGET_X86_64_LINUX_ANDROID_RUNNER=$PWD/.github/scripts/android-test-runner.sh cargo ndk -t x86_64 test -p corundum-gui --lib --features android` → PASS.
- [ ] **Step 8: Commit** `feat(corundum-gui): the Android window behind an android feature (#326)`.

### Task 2: sapphire-framework-gui on corundum-gui's Android window

**Files:**
- Modify: `crates/sapphire-framework-gui/Cargo.toml`, `src/mobile/mod.rs`, `src/mobile/android.rs`, `src/platform/mod.rs`, `src/app/mod.rs`
- Delete: `src/mobile/logcat.rs`, `src/mobile/text_input.rs`, `src/platform/android.rs`

**Interfaces:**
- Consumes: Task 1's `corundum_gui::{platform, logcat, window::{AndroidApp, run_android}}`.
- Produces: unchanged public API; `app::build_app` becomes `pub(crate)` and `open_window` is removed.

- [ ] **Step 1: Cargo.** `mobile`: replace `"eframe/android-game-activity"` with `"corundum-gui/android"`; delete the `[target.'cfg(target_os = "android")'.dependencies]` block.
- [ ] **Step 2: `platform/mod.rs`.** `clipboard_text`, `window_insets`, `needs_paste_button` delegate to `corundum_gui::platform::…` (keep their docs, point to corundum-gui); remove `mod android` and `set_android_activity`. Keep the existing tests.
- [ ] **Step 3: `mobile`.** Remove `mod logcat;` and `mod text_input;` and delete the files. `phone_log_filter(app_name)` returns `corundum_gui::logcat::phone_log_filter(&format!("{},sapphire_framework_gui=info", default_log_filter(app_name)))`; its existing test stays unchanged and must pass.
- [ ] **Step 4: `mobile/android.rs`.** `pub use corundum_gui::window::AndroidApp;`. `run_android`: `corundum_gui::logcat::install(app_name, &…)` — the same app filter string as Step 3 — in place of `super::logcat::install`. `run_once`: drop `gpu::install_panic_hook()`, `set_android_activity`, the `NativeOptions` and `PhoneWindow`; after the config, `corundum_gui::window::run_android(android_app, WindowOptions::new(title)` (plus `.fonts(f)` when `fonts` is `Some`), `move |cc| crate::app::build_app(cc, handle, config, data_dir, create))`. Factor the app filter string into one private fn used by both Step 3 and here.
- [ ] **Step 5: `app/mod.rs`.** Delete `open_window`; `build_app` → `pub(crate)` with `cfg_attr(not(all(feature = "mobile", target_os = "android")), …)` not needed (desktop `run` uses it).
- [ ] **Step 6: Verify.**
  - `cargo test -p sapphire-framework-gui --all-features` → PASS; clippy `--all-features --all-targets` and no features → clean.
  - `cargo ndk -t arm64-v8a check -p sapphire-framework-gui --features mobile` → no errors or warnings (background; ~12 min).
  - `cargo ndk -t arm64-v8a clippy -p sapphire-framework-gui --features mobile -- -D warnings` → clean (after the check, reusing its build).
- [ ] **Step 7: Commit** `refactor(gui): run the phone's window through corundum-gui (#326)`.

### Task 3: Mobile CI and docs

**Files:**
- Modify: `.github/workflows/mobile-ci.yml`, `CHANGELOG.md`, `docs/superpowers/specs/2026-10-23-corundum-gui-eframe-design.md` (Known gaps → this spec)

- [ ] **Step 1: mobile-ci.yml android job.** Replace `--features eframe/android-game-activity` with `--features android` in the clippy line, the `--no-run` build and the emulator script; update the comment (`android` picks GameActivity).
- [ ] **Step 2: CHANGELOG.** corundum-gui 0.1.0 entry: the `android` feature (`window::run_android`, `platform`, `logcat`). Changed: sapphire-framework-gui's `mobile` runs the phone's window through corundum-gui (no API change).
- [ ] **Step 3:** `cargo fmt --all --check`; YAML parses.
- [ ] **Step 4: Commit** `ci(mobile): test corundum-gui with its android feature`.

### Task 4: End to end on the emulator (no framework commit)

- [ ] **Step 1: A sapphire-sync checkout of its own.** `git -C ~/Documents/Dev/project-sapphire/sapphire-sync worktree add --detach ../sapphire-sync-wt-android-check origin/feat/mobile-android` (do not touch `sapphire-sync-wt-mobile`, which another session may use).
- [ ] **Step 2: Point it at this branch.** In that checkout's root `Cargo.toml`, a `[patch.crates-io]` entry with `path = "…/sapphire-framework-wt-android/crates/<crate>"` for every sapphire-framework crate in its `Cargo.lock` (and `corundum-gui`). Build with `CARGO_TARGET_DIR` on disk (`~/.cache/claude-cargo-targets/sync-android-check`). If versions do not match, record what was needed.
- [ ] **Step 3: SDK pieces.** `sdkmanager "platforms;android-35" "build-tools;35.0.0"`; `JAVA_HOME=~/.local/jdk`.
- [ ] **Step 4: Build and install.** `ABIS=x86_64 mobile/build-android.sh debug`; start the AVD; `adb install -r …/app-debug.apk`; `adb shell am start -n io.github.sapphireturtle.sync/com.google.androidgamesdk.GameActivity`.
- [ ] **Step 5: Check.**
  - `adb logcat -d -s sapphire-sync:*` contains "wgpu renderer ready" and "using a system CJK font".
  - `adb exec-out screencap -p > shot.png` and read it: the app's content is clear of the status bar and the navigation bar.
  - Open a text field (Settings → Workgroup → Join), tap it (`adb shell input tap x y`), `adb shell input text hello`, screenshot: "hello" in the field.
- [ ] **Step 6: Record** the results (and screenshots' paths) in the ledger; remove the sapphire-sync worktree and its target dir afterwards.
