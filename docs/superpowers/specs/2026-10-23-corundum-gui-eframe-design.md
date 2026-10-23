# corundum-gui: the desktop window (`eframe` feature)

Follows the foundation (`2026-10-22-corundum-gui-design.md`, #318), which left eframe out
until something window-level moved: "It is added when something window-level moves".
This moves the part of `sapphire_framework_gui::app` that has nothing to do with sapphire —
the renderer setup and opening the window — so asteria opens its desktop window the same
way.

## Scope

- **In:** the wgpu setup (`app/gpu.rs`) and the generic half of opening a desktop window.
- **Out (next PR):** the Android window — `PhoneWindow`, the soft-keyboard bridge
  (`mobile/text_input.rs`), logcat, the clipboard and the insets. They need eframe's
  `android-game-activity` and checking on a device, so they get their own PR.

## Decisions

1. **`eframe` is a default feature of corundum-gui.** Every corundum app is an eframe app
   except sapphire-agent, which will render with bevy and turns default features off.
2. **sapphire-framework-gui takes corundum-gui with `default-features = false`** and turns
   on `corundum-gui/eframe` from its own `eframe` feature, so a user of
   sapphire-framework-gui without `eframe` (sapphire-agent) still gets no eframe.
3. **sapphire-framework-gui's public API does not change.** `app::run`, `AppOptions` and
   `AppSetup` keep their signatures; only their insides call corundum-gui.
4. **corundum-gui stays at 0.1.0**: it has not been published yet (the release waits for
   bridge-api), so the CHANGELOG entry for 0.1.0 grows.

## Design

### corundum-gui

`Cargo.toml`:

```toml
[features]
default = ["eframe"]
# The desktop window: `window::run` and the wgpu setup in `gpu`.
eframe = ["dep:eframe"]

[dependencies]
eframe = { version = "0.36", optional = true }   # default features, as sapphire-framework-gui's
```

`lib.rs`: `#[cfg(feature = "eframe")] pub use eframe;`, `pub mod gpu;` and `pub mod window;`
(both behind the feature).

**`gpu`** — `sapphire-framework-gui/src/app/gpu.rs` moved unchanged with its tests. Made
`pub`: `wgpu_options() -> egui_wgpu::WgpuConfiguration`, `install_panic_hook()`,
`on_render_state(Option<&egui_wgpu::RenderState>)`.

**`window`**:

```rust
/// Sets the egui context's fonts, once, before the app is built.
pub type InstallFonts = Box<dyn FnOnce(&egui::Context)>;

/// A desktop window: its title, its size, and its fonts.
pub struct WindowOptions { title, inner_size, min_inner_size, fonts: Option<InstallFonts> }

impl WindowOptions {
    /// Titled `title`, 900 × 600 and no smaller than 600 × 400.
    pub fn new(title: impl Into<String>) -> WindowOptions;
    pub fn inner_size(self, size: [f32; 2]) -> WindowOptions;
    pub fn min_inner_size(self, size: [f32; 2]) -> WindowOptions;
    /// Instead of the default: egui's fonts with a system CJK font behind them.
    pub fn fonts(self, install: impl FnOnce(&egui::Context) + 'static) -> WindowOptions;
}

/// Open a desktop window and run it until it closes: installs `gpu`'s panic hook, builds
/// the `NativeOptions` (the viewport, `gpu::wgpu_options`) and calls `open`.
pub fn run(
    options: WindowOptions,
    create: impl FnOnce(&eframe::CreationContext<'_>) -> Box<dyn eframe::App>,
) -> eframe::Result<()>;

/// Run eframe with `native` as the caller built it (a phone's entry sets more on it):
/// reports the renderer (`gpu::on_render_state`), installs `fonts` or the default, then
/// builds the app with `create`.
pub fn open(
    title: &str,
    native: eframe::NativeOptions,
    fonts: Option<InstallFonts>,
    create: impl FnOnce(&eframe::CreationContext<'_>) -> Box<dyn eframe::App>,
) -> eframe::Result<()>;
```

`open` is today's `open_window` without the framework client.

### sapphire-framework-gui

- `Cargo.toml`: `corundum-gui = { version = "0.1.0", path = "../corundum-gui",
  default-features = false }`; `eframe = ["dep:eframe", "corundum-gui/eframe", …]`. It
  keeps its own `eframe` dependency (the `mobile` feature turns on
  `eframe/android-game-activity`) and its `pub use eframe`.
- `app/gpu.rs` removed.
- `AppOptions` holds the app's identity and a `corundum_gui::window::WindowOptions`; its
  builder methods forward to it. `InstallFonts` becomes corundum-gui's.
- `app::run`: the logging, the app's directories, the runtime and the `ClientConfig` as
  today, then `corundum_gui::window::run` with a `create` that spawns the
  `FrameworkClient` and calls the app's closure with `AppSetup` as today.
- `open_window` (shared with `mobile::run_android`) keeps its role: it calls
  `corundum_gui::window::open`. `run_android` takes `wgpu_options` and the panic hook from
  `corundum_gui::gpu`.
- `desktop_log_filter`: `sapphire_framework_gui=info` becomes `corundum_gui=info`. The
  renderer's startup line now comes from corundum-gui, and would otherwise be filtered out.

### Mobile CI

eframe on Android needs one of winit's activity features, or winit fails to compile. So:

- **android:** clippy twice, with `--no-default-features` and with
  `--features eframe/android-game-activity`. The emulator's tests use
  `--no-default-features` (the font tests need no eframe), unless a test binary with eframe
  turns out to run on the emulator, which is tried locally first.
- **ios:** default features, clippy and the simulator's tests as today.

## Testing

- `gpu`'s tests, moved.
- `window`: `WindowOptions` defaults and overrides (moved from `AppOptions`'s test).
- sapphire-framework-gui: `AppOptions`'s test kept (now through `WindowOptions`), and the log
  filter's test updated to `corundum_gui=info`.
- `cargo tree -p sapphire-framework-gui --no-default-features -e normal | grep eframe` is
  empty: without its `eframe` feature, nothing pulls eframe in.
- Mobile CI green; the desktops in ci.yml (`--all-features`).

## Known gaps

- The Android window (`PhoneWindow`, text input, logcat, clipboard, insets) stays in
  sapphire-framework-gui until the next PR — done in
  `2026-10-23-corundum-gui-android-design.md` (#326).
