# corundum-gui desktop window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the generic desktop-window code (wgpu setup, opening an eframe window) from sapphire-framework-gui into corundum-gui behind a default `eframe` feature.

**Architecture:** corundum-gui gains `gpu` (moved verbatim) and `window` (`WindowOptions`, `run`, `open`). sapphire-framework-gui's `app::run` keeps its signature and builds on `corundum_gui::window`, adding only the sapphire parts (logging, dirs, runtime, `FrameworkClient`). sapphire-framework-gui takes corundum-gui without default features and enables `corundum-gui/eframe` from its own `eframe` feature.

**Tech Stack:** Rust, egui/eframe 0.36 (wgpu), cargo features, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-10-23-corundum-gui-eframe-design.md`

## Global Constraints

- corundum-gui: `default = ["eframe"]`, `eframe = ["dep:eframe"]`, `eframe = { version = "0.36", optional = true }` (default features). Version stays `0.1.0`.
- sapphire-framework-gui: `corundum-gui = { version = "0.1.0", path = "../corundum-gui", default-features = false }`; its `eframe` feature adds `"corundum-gui/eframe"`. It keeps its own `eframe` dependency and `pub use eframe`.
- sapphire-framework-gui's public API (`app::run`, `AppOptions` and its builder methods, `AppSetup`, `mobile::MobileOptions`) does not change.
- `WindowOptions::new` defaults: `inner_size [900.0, 600.0]`, `min_inner_size [600.0, 400.0]`, `fonts None`.
- Desktop log filter ends with `,corundum_gui=info` (was `,sapphire_framework_gui=info`).
- Worktree: `../sapphire-framework-wt-eframe`, branch `feat/corundum-eframe`. Its cargo target is the shared `project-sapphire/target` (no build is running there now).

## Review Focus

1. **The panic hook is installed once.** `corundum_gui::window::run` installs it; sapphire's `app::run` must no longer call it too, or a lost-GPU panic logs its explanation twice.
2. **eframe does not leak** into sapphire-framework-gui without its `eframe` feature (sapphire-agent's case): checked with `cargo tree` in Task 2.
3. **The renderer's startup line still prints** by default: its target is now `corundum_gui`, so the desktop filter must name it (Task 2 test).
4. **The Android entry still compiles** (`mobile` feature, `target_os = "android"`): no CI job builds it, so Task 2 checks it locally with cargo-ndk.
5. **Default fonts still install** when the app gives none: `open` calls `fonts::install_system_cjk_fallback` in that case, as `open_window` did.

---

### Task 1: corundum-gui `eframe` feature, `gpu` and `window`

**Files:**
- Modify: `crates/corundum-gui/Cargo.toml`, `crates/corundum-gui/src/lib.rs`
- Create: `crates/corundum-gui/src/gpu.rs` (moved from `crates/sapphire-framework-gui/src/app/gpu.rs`), `crates/corundum-gui/src/window.rs`

**Interfaces:**
- Produces:
  - `corundum_gui::eframe` (re-export)
  - `corundum_gui::gpu::{wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration, install_panic_hook(), on_render_state(Option<&eframe::egui_wgpu::RenderState>)}`
  - `corundum_gui::window::{InstallFonts, WindowOptions, run, open}` with the spec's signatures; `WindowOptions` builder: `new(title)`, `inner_size([f32;2])`, `min_inner_size([f32;2])`, `fonts(impl FnOnce(&egui::Context) + 'static)`.
  - `WindowOptions::native_options(&self) -> eframe::NativeOptions` (crate-visible is enough for `run`; make it `pub` only if Task 2 needs it — it does not).

- [ ] **Step 1: Cargo and lib.rs.** Add the feature and dependency per Global Constraints; in `lib.rs` add `#[cfg(feature = "eframe")] pub use eframe;`, `#[cfg(feature = "eframe")] pub mod gpu;`, `#[cfg(feature = "eframe")] pub mod window;`, with a crate-doc paragraph on the feature (default; off for apps that render without eframe).
- [ ] **Step 2: Move `gpu.rs`** with `git mv`-equivalent content (copy now, delete in Task 2), changing only `pub(crate)` → `pub` on the three functions and `use eframe::{egui_wgpu, wgpu};` (same path). Its tests come along unchanged.
- [ ] **Step 3: Write the failing `window` test** in `window.rs`:

```rust
#[test]
fn options_default_and_override_the_window_size() {
    let options = WindowOptions::new("Example");
    assert_eq!(options.title, "Example");
    assert_eq!(options.inner_size, [900.0, 600.0]);
    assert_eq!(options.min_inner_size, [600.0, 400.0]);
    assert!(options.fonts.is_none());
    let options = options
        .inner_size([1000.0, 640.0])
        .min_inner_size([720.0, 440.0])
        .fonts(|_| {});
    assert_eq!(options.inner_size, [1000.0, 640.0]);
    assert_eq!(options.min_inner_size, [720.0, 440.0]);
    assert!(options.fonts.is_some());
}
```

- [ ] **Step 4: Run it.** `cargo test -p corundum-gui --lib window` → FAIL (`WindowOptions` not found).
- [ ] **Step 5: Implement `window.rs`.** `WindowOptions` fields private (`title: String`, `inner_size`, `min_inner_size`, `fonts: Option<InstallFonts>`). `run`: `gpu::install_panic_hook()`, build `NativeOptions { viewport: ViewportBuilder::default().with_inner_size(..).with_min_inner_size(..).with_title(&title), wgpu_options: gpu::wgpu_options(), ..Default::default() }`, then `open(&title, native, fonts, create)`. `open`: `eframe::run_native(title, native, Box::new(move |cc| { gpu::on_render_state(cc.wgpu_render_state.as_ref()); match fonts { Some(f) => f(&cc.egui_ctx), None => { crate::fonts::install_system_cjk_fallback(&cc.egui_ctx); } } Ok(create(cc)) }))`. Module doc says what moved and why (asteria opens its window the same way).
- [ ] **Step 6: Run.** `cargo test -p corundum-gui` → all pass (gpu's 6–7 tests, window's 1, the existing 19). `cargo clippy -p corundum-gui --all-targets -- -D warnings` and `cargo clippy -p corundum-gui --no-default-features --all-targets -- -D warnings` → clean.
- [ ] **Step 7: Commit** `feat(corundum-gui): the desktop window behind a default eframe feature`.

### Task 2: sapphire-framework-gui on corundum-gui's window

**Files:**
- Modify: `crates/sapphire-framework-gui/Cargo.toml`, `src/app/mod.rs`, `src/mobile/mod.rs`, `src/mobile/android.rs`, `src/lib.rs` (doc only if it mentions gpu)
- Delete: `crates/sapphire-framework-gui/src/app/gpu.rs`

**Interfaces:**
- Consumes: Task 1's `corundum_gui::{gpu, window}`.
- Produces: unchanged public API; `pub(crate) open_window(title, native, handle, config, data_dir, fonts, create)` kept for `run_android`, now over `corundum_gui::window::open`.

- [ ] **Step 1: Update the log-filter test first** (`app/mod.rs`): `assert!(filter.ends_with(",corundum_gui=info"));`. Run `cargo test -p sapphire-framework-gui --features eframe --lib app::` → FAIL on that assertion.
- [ ] **Step 2: Cargo.** Per Global Constraints.
- [ ] **Step 3: `app/mod.rs`.**
  - Remove `pub(crate) mod gpu;` and delete `app/gpu.rs`.
  - `pub(crate) type InstallFonts = corundum_gui::window::InstallFonts;` (keeps `mobile/mod.rs`'s import working).
  - `AppOptions { app, window: WindowOptions }`; `new` → `WindowOptions::new(title)`; builder methods forward. The existing `options_default_and_override_the_window_size` test reads sizes through the window options — expose them with `#[cfg(test)]` accessors on `WindowOptions` if its fields are private, or rewrite the sapphire test to check only what `AppOptions` adds (the identity) and leave sizes to corundum's test. Ledger the choice.
  - `run`: logging, dirs, runtime, config as today; **no** `gpu::install_panic_hook()` (corundum's `run` does it); then `corundum_gui::window::run(window, move |cc| build(cc, handle, config, data_dir, create))`.
  - Factor the closure body of today's `open_window` into `fn build(cc, handle, config, data_dir, create) -> Box<dyn eframe::App>` (repaint, `FrameworkClient::spawn`, `AppSetup`). `open_window` becomes `corundum_gui::window::open(title, native, fonts, move |cc| build(..))`.
  - `desktop_log_filter`: replace `env!("CARGO_CRATE_NAME")` with `"corundum_gui"`, and say in its doc why (the renderer line comes from corundum-gui).
- [ ] **Step 4: `mobile/android.rs`.** `use crate::app::{AppSetup, open_window};` and `use corundum_gui::gpu;` — the calls stay `gpu::install_panic_hook()` / `gpu::wgpu_options()`.
- [ ] **Step 5: Run.** `cargo test -p sapphire-framework-gui --all-features` → PASS. `cargo clippy -p sapphire-framework-gui --all-features --all-targets -- -D warnings` and `cargo clippy -p sapphire-framework-gui --all-targets -- -D warnings` (no features) → clean.
- [ ] **Step 6: eframe does not leak.** `cargo tree -p sapphire-framework-gui -e normal --prefix none | grep -c '^eframe '` → `0`; with `--features eframe` → non-zero.
- [ ] **Step 7: The Android entry compiles.** `cargo ndk -t arm64-v8a check -p sapphire-framework-gui --features mobile` → finishes without errors. (Heavy: bridge and server. If it fails for a reason unrelated to this change, compare against `origin/main` with the same command and ledger the result.)
- [ ] **Step 8: Run the whole workspace's checks touched by the move.** `cargo check -p sapphire-framework --features eframe` and `cargo test -p sapphire-framework-gui -p corundum-gui --all-features` → pass.
- [ ] **Step 9: Commit** `refactor(gui): open the window through corundum-gui`.

### Task 3: Mobile CI and docs

**Files:**
- Modify: `.github/workflows/mobile-ci.yml`, `CHANGELOG.md`, `docs/superpowers/specs/2026-10-22-corundum-gui-design.md` (decision 3's "No eframe feature yet" gets a pointer to this spec)

- [ ] **Step 1: Try an eframe test binary on the emulator.** Start the local AVD (`ANDROID_AVD_HOME=$HOME/.config/.android/avd`), then `CARGO_TARGET_X86_64_LINUX_ANDROID_RUNNER=$PWD/.github/scripts/android-test-runner.sh cargo ndk -t x86_64 test -p corundum-gui --lib --features eframe/android-game-activity`. If it runs and passes, CI tests with that; otherwise with `--no-default-features`. Ledger the outcome.
- [ ] **Step 2: android job.** Clippy for `-t arm64-v8a -t x86_64` twice: `--no-default-features` and `--features eframe/android-game-activity` (both `--all-targets --locked -- -D warnings`). The emulator step's test command per Step 1, and the `--no-run` build step to match.
- [ ] **Step 3: ios job.** Unchanged commands (default features now include eframe); confirm locally with `cargo clippy -p corundum-gui --target aarch64-apple-ios --all-targets -- -D warnings` (no linking).
- [ ] **Step 4: CHANGELOG.** Extend the corundum-gui 0.1.0 Added entry: the `eframe` feature (default) with `window::run` / `window::open` / `WindowOptions` and `gpu`. Under Changed: sapphire-framework-gui's `app::run` opens its window through corundum-gui; the renderer's startup line's log target is now `corundum_gui`.
- [ ] **Step 5: Run** `cargo fmt --all --check`, `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/mobile-ci.yml'))"` → ok.
- [ ] **Step 6: Commit** `ci(mobile): build corundum-gui with and without eframe on Android`.
