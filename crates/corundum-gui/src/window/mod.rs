//! A desktop window: eframe on wgpu, with the renderer set up by [`gpu`](crate::gpu) and a
//! CJK font behind egui's own.
//!
//! [`run`] is the whole of opening an app's desktop window; [`open`] is its lower half, for
//! an entry that builds its own [`eframe::NativeOptions`]. With the `android` feature,
//! `run_android` opens the window on an Android phone. The sapphire apps
//! and asteria open their windows through here, so a renderer fix reaches all of them.
//!
//! ```no_run
//! use corundum_gui::{eframe, egui, window};
//!
//! struct Example;
//!
//! impl eframe::App for Example {
//!     fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
//!         ui.label("Hello");
//!     }
//! }
//!
//! fn main() -> eframe::Result<()> {
//!     window::run(
//!         window::WindowOptions::new("Example").inner_size([800.0, 500.0]),
//!         |_cc| Box::new(Example),
//!     )
//! }
//! ```

use crate::gpu;

#[cfg(all(target_os = "android", feature = "android"))]
mod android;
// Used by the Android window; kept platform-free so its tests run on a desktop.
#[cfg_attr(not(all(target_os = "android", feature = "android")), allow(dead_code))]
mod text_input;

#[cfg(all(target_os = "android", feature = "android"))]
pub use android::{AndroidApp, run_android};

/// Sets the egui context's fonts, once, before the app is built.
pub type InstallFonts = Box<dyn FnOnce(&egui::Context)>;

/// A desktop window: its title, its size and its fonts.
pub struct WindowOptions {
    title: String,
    inner_size: [f32; 2],
    min_inner_size: [f32; 2],
    fonts: Option<InstallFonts>,
}

impl WindowOptions {
    /// A window titled `title`, 900 × 600 and no smaller than 600 × 400.
    pub fn new(title: impl Into<String>) -> WindowOptions {
        WindowOptions {
            title: title.into(),
            inner_size: [900.0, 600.0],
            min_inner_size: [600.0, 400.0],
            fonts: None,
        }
    }

    /// The window's size when it opens.
    pub fn inner_size(mut self, size: [f32; 2]) -> WindowOptions {
        self.inner_size = size;
        self
    }

    /// The smallest the window can be made.
    pub fn min_inner_size(mut self, size: [f32; 2]) -> WindowOptions {
        self.min_inner_size = size;
        self
    }

    /// Set the fonts with `install` instead of the default: egui's own, with a CJK font
    /// the OS has behind them ([`fonts::install_system_cjk_fallback`](crate::fonts::install_system_cjk_fallback)).
    pub fn fonts(mut self, install: impl FnOnce(&egui::Context) + 'static) -> WindowOptions {
        self.fonts = Some(Box::new(install));
        self
    }
}

/// Open a desktop window and run it until it closes.
///
/// Installs [`gpu::install_panic_hook`], opens the window on wgpu with
/// [`gpu::wgpu_options`], and builds the app with `create` once the window exists.
pub fn run(
    options: WindowOptions,
    create: impl FnOnce(&eframe::CreationContext<'_>) -> Box<dyn eframe::App>,
) -> eframe::Result<()> {
    let WindowOptions {
        title,
        inner_size,
        min_inner_size,
        fonts,
    } = options;
    gpu::install_panic_hook();
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(inner_size)
            .with_min_inner_size(min_inner_size)
            .with_title(&title),
        wgpu_options: gpu::wgpu_options(),
        ..Default::default()
    };
    open(&title, native, fonts, create)
}

/// Run eframe with `native` as the caller built it, building the app with `create` once the
/// window exists: reports the renderer ([`gpu::on_render_state`]) and installs `fonts`, or
/// the default ones when `None`. [`run`] is this for a desktop window.
pub fn open(
    title: &str,
    native: eframe::NativeOptions,
    fonts: Option<InstallFonts>,
    create: impl FnOnce(&eframe::CreationContext<'_>) -> Box<dyn eframe::App>,
) -> eframe::Result<()> {
    eframe::run_native(
        title,
        native,
        Box::new(move |cc| {
            gpu::on_render_state(cc.wgpu_render_state.as_ref());
            match fonts {
                Some(install) => install(&cc.egui_ctx),
                None => {
                    crate::fonts::install_system_cjk_fallback(&cc.egui_ctx);
                }
            }
            Ok(create(cc))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
