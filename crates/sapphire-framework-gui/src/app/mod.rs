//! [`run`]: a sapphire app's desktop window, started the same way for every app.
//!
//! Every desktop app is a pure client of its app server, which relays the bridge, so they
//! all start alike: the framework's logging, a panic hook that explains a lost GPU device,
//! a tokio runtime, the app's directories, a [`FrameworkClient`], and corundum-framework-gui's
//! window (eframe on wgpu, with a backend that works on the host), which opens in the look
//! last applied (corundum's store, in the app's config directory). What differs — the app's name,
//! its window and its own [`eframe::App`] — goes in [`AppOptions`] and the closure.
//!
//! ```no_run
//! use sapphire_framework_gui::app::{self, AppOptions, AppSetup};
//! use sapphire_framework_gui::client::AppIdentity;
//! use sapphire_framework_gui::{Settings, eframe};
//!
//! const APP: AppIdentity = AppIdentity {
//!     app_name: "sapphire-example",
//!     version: "0.1.0",
//! };
//!
//! struct Example(Settings);
//!
//! impl eframe::App for Example {
//!     fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
//!         self.0.ui(ui);
//!     }
//! }
//!
//! fn main() -> eframe::Result<()> {
//!     app::run(
//!         AppOptions::new(APP, "Sapphire Example").inner_size([900.0, 600.0]),
//!         |setup: AppSetup<'_>| Box::new(Example(Settings::new(setup.framework))),
//!     )
//! }
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use sapphire_workspace::AppKind;
use sapphire_workspace::logging::{default_log_filter, install_console_with_filter};

use corundum_framework_gui::store::{self, Store};
use corundum_framework_gui::window::{self, WindowOptions};

use crate::client::{AppIdentity, ClientConfig, Endpoints, FrameworkClient};

/// Sets the egui context's fonts, once, before the app is built.
pub(crate) type InstallFonts = window::InstallFonts;

/// What [`run`] needs to know about the app and its window.
pub struct AppOptions {
    app: AppIdentity,
    window: WindowOptions,
}

impl AppOptions {
    /// `app`'s window, titled `title`, 900 × 600 and no smaller than 600 × 400.
    pub fn new(app: AppIdentity, title: impl Into<String>) -> AppOptions {
        AppOptions {
            app,
            window: WindowOptions::new(title),
        }
    }

    /// The window's size when it opens.
    pub fn inner_size(mut self, size: [f32; 2]) -> AppOptions {
        self.window = self.window.inner_size(size);
        self
    }

    /// The smallest the window can be made.
    pub fn min_inner_size(mut self, size: [f32; 2]) -> AppOptions {
        self.window = self.window.min_inner_size(size);
        self
    }

    /// Set the fonts with `install` instead of the default: egui's own, with a CJK font
    /// the OS has behind them ([`fonts::install_system_cjk_fallback`](crate::fonts::install_system_cjk_fallback)).
    pub fn fonts(mut self, install: impl FnOnce(&egui::Context) + 'static) -> AppOptions {
        self.window = self.window.fonts(install);
        self
    }
}

/// What the app's closure gets to build its [`eframe::App`] from.
pub struct AppSetup<'a> {
    /// eframe's creation context: the egui context, the renderer, persisted storage.
    pub cc: &'a eframe::CreationContext<'a>,
    /// The runtime the clients run on. [`run`] keeps it alive until the window closes.
    pub runtime: tokio::runtime::Handle,
    /// The client of the bridge and the app server, for the framework's views.
    pub framework: FrameworkClient,
    /// Wakes the window: give it to a client of the app's own methods.
    pub repaint: Arc<dyn Fn() + Send + Sync>,
    /// Where the app server listens, which the app's own client connects to as well.
    pub endpoints: Endpoints,
    /// The app's data directory (`<platform data dir>/<app name>`, honouring
    /// `SAPPHIRE_<APP>_DATA_DIR`), for what the window keeps between runs: its UI
    /// settings, not the workspace's.
    pub data_dir: PathBuf,
}

/// Open the app's window and run it until it closes.
///
/// `create` builds the app once the window exists. Nothing is started that outlives the
/// window: the app server and the bridge are service-managed, and the client only talks
/// to them.
pub fn run(
    options: AppOptions,
    create: impl FnOnce(AppSetup<'_>) -> Box<dyn eframe::App>,
) -> eframe::Result<()> {
    let AppOptions { app, window } = options;

    // Before the context's own `install_console`, which then changes nothing.
    install_console_with_filter(app.app_name, &desktop_log_filter(app.app_name));
    let dirs = sapphire_workspace::AppContext::new(app.app_name);
    dirs.init(AppKind::Desktop);
    let data_dir = dirs.data_dir().to_owned();
    let window = window.remember(corundum_store(&dirs));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;
    let config = ClientConfig::new(app).map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;

    // corundum-framework-gui installs the panic hook that explains a lost GPU device.
    let handle = runtime.handle().clone();
    let result = window::run(window, move |cc| {
        build_app(cc, handle, config, data_dir, create)
    });
    // The clients' tasks end with the runtime, after the window that used them.
    drop(runtime);
    result
}

/// corundum's store in the app's config directory (honouring `SAPPHIRE_<APP>_CONFIG_DIR`):
/// the look last applied ([`Settings::follow_theme`](crate::Settings::follow_theme) keeps
/// it), which the window installs before the first frame, so the app opens in it before the
/// bridge answers, or without one.
pub(crate) fn corundum_store(dirs: &sapphire_workspace::AppContext) -> Store {
    Store::at(dirs.config_dir().join(store::FILE_NAME))
}

/// The app, built once its window exists: the framework client, started on `handle`, and
/// the app's closure with everything [`AppSetup`] gives it.
pub(crate) fn build_app(
    cc: &eframe::CreationContext<'_>,
    handle: tokio::runtime::Handle,
    config: ClientConfig,
    data_dir: PathBuf,
    create: impl FnOnce(AppSetup<'_>) -> Box<dyn eframe::App>,
) -> Box<dyn eframe::App> {
    let endpoints = config.endpoints.clone();
    let ctx = cc.egui_ctx.clone();
    let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
    let framework = FrameworkClient::spawn(&handle, config, Arc::clone(&repaint));
    create(AppSetup {
        cc,
        runtime: handle,
        framework,
        repaint,
        endpoints,
        data_dir,
    })
}

/// The console filter a desktop app prints with when `RUST_LOG` asks for nothing.
///
/// The framework's default (its targets and the app's, at info) with corundum-framework-gui's — the
/// renderer's startup line, which its window logs — and every other target's warnings and
/// errors: the console
/// is all the log a window has, and a wgpu warning is how a graphics problem shows up.
/// `RUST_LOG=wgpu_core=debug,wgpu_hal=debug` is the useful setting when one does.
fn desktop_log_filter(app_name: &str) -> String {
    format!(
        "warn,{},corundum_framework_gui=info",
        default_log_filter(app_name)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktop_filter_parses_and_adds_the_gui_and_warnings() {
        let filter = desktop_log_filter("sapphire-example");
        assert!(filter.starts_with("warn,"));
        assert!(filter.contains("sapphire_example=info"));
        assert!(filter.ends_with(",corundum_framework_gui=info"));
    }

    /// The window's size and fonts are corundum-framework-gui's `WindowOptions` (tested there);
    /// `AppOptions` adds the app, which the builder keeps.
    #[test]
    fn options_keep_the_app_through_the_window_builder() {
        let app = AppIdentity {
            app_name: "sapphire-example",
            version: "0.1.0",
        };
        let options = AppOptions::new(app, "Example")
            .inner_size([1000.0, 640.0])
            .min_inner_size([720.0, 440.0])
            .fonts(|_| {});
        assert_eq!(options.app.app_name, "sapphire-example");
        assert_eq!(options.app.version, "0.1.0");
    }
}
