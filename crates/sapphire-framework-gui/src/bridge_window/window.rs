//! [`run_bridge`]: Sapphire Bridge's window on the desktop.

use std::sync::Arc;

use corundum_framework_gui::window::{self, WindowOptions};
use corundum_framework_platform::instance;
use sapphire_workspace::logging::install_console_with_filter;
use sapphire_workspace::{AppContext, AppKind};

use super::{BridgePage, PageInbox};
use crate::Settings;
use crate::client::{ClientConfig, FrameworkClient};

/// The name the window's single-instance lock and its directories go by.
const APP_NAME: &str = "sapphire-bridge-desktop";

/// What `sapphire-bridge-desktop` opens.
pub struct BridgeWindowOptions {
    /// The window's title: "Sapphire Bridge".
    pub title: String,
    /// The binary's version, for the handshake and About.
    pub version: &'static str,
    /// The page to show first, or to show in the window already open.
    pub page: BridgePage,
    /// In a Velopack package: its updater and its service.
    #[cfg(feature = "package")]
    pub package: Option<PackageWindow>,
}

/// What the window does in a Velopack package.
#[cfg(feature = "package")]
pub struct PackageWindow {
    /// Make the package's updater; called once this start holds the window's lock, so a
    /// second start never applies an update under the first.
    pub start_updater: fn() -> Option<corundum_framework_package::Updater>,
    /// The bridge's service, which the first run and About register.
    pub service: fn() -> corundum_framework_service::ServiceSpec,
    /// The package's command on `PATH`, which About offers to link on macOS and Linux.
    pub cli: Option<corundum_framework_package::CliSpec>,
}

/// What the window prints when `RUST_LOG` asks for nothing: its renderer's warnings, its
/// own and the gui's info, and the package runtime's failures only.
const WINDOW_LOG_FILTER: &str = "warn,sapphire_bridge_desktop=info,sapphire_framework_gui=info,velopack=error,corundum_framework_package=warn";

/// Install the window's console logging. [`run_bridge`] does; a binary calls it first when it
/// runs anything before (the package's start, whose hooks log), and then it changes nothing.
pub fn install_logging() {
    install_console_with_filter(APP_NAME, WINDOW_LOG_FILTER);
}

/// Open Sapphire Bridge's window and run it until it closes; when it is open already, bring
/// that one to the front on `options.page` and return. Closing the window stops nothing:
/// the bridge is its own process (`serve`).
pub fn run_bridge(options: BridgeWindowOptions) -> eframe::Result<()> {
    let BridgeWindowOptions {
        title,
        version,
        page,
        #[cfg(feature = "package")]
        package,
    } = options;
    install_logging();
    let Some(instance) = instance::acquire_or_activate_with(APP_NAME, Some(page.as_str())) else {
        tracing::info!("Sapphire Bridge's window is open already: brought it to the front");
        return Ok(());
    };
    // This start is the instance: only now may it apply an update.
    #[cfg(feature = "package")]
    let package = package.map(|p| (p.service, p.cli, (p.start_updater)()));
    let dirs = AppContext::new(APP_NAME);
    dirs.init(AppKind::Desktop);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;
    let config =
        ClientConfig::bridge_only(version).map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;
    let (to_inbox, inbox) = PageInbox::new();
    let to_inbox = std::sync::Mutex::new(to_inbox);
    let options = WindowOptions::new(title)
        .remember(crate::app::corundum_store(&dirs))
        .instance(instance)
        .on_activate(move |message| {
            if let Some(message) = message {
                let _ = to_inbox
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .send(message);
            }
        });
    let handle = runtime.handle().clone();
    let result = window::run(options, move |cc| {
        let ctx = cc.egui_ctx.clone();
        let repaint: Arc<dyn Fn() + Send + Sync> = Arc::new(move || ctx.request_repaint());
        let client = FrameworkClient::spawn(&handle, config, repaint);
        let mut settings = Settings::for_bridge(client);
        #[cfg(feature = "package")]
        let mut first_run = None;
        #[cfg(feature = "package")]
        if let Some((service, cli, updater)) = package {
            settings = settings.with_package(updater, service, cli, version);
            if !corundum_framework_gui::first_run::FirstRun::done(&cc.egui_ctx) {
                first_run = Some(corundum_framework_gui::first_run::FirstRun::new(
                    "Sapphire Bridge",
                    super::first_run_pages(&cc.egui_ctx, service),
                ));
            }
        }
        settings.open_page(page);
        Box::new(BridgeApp {
            settings,
            inbox,
            #[cfg(feature = "package")]
            first_run,
        })
    });
    drop(runtime);
    result
}

/// The window: the bridge's Settings, opening the page an activation names.
struct BridgeApp {
    settings: Settings,
    inbox: PageInbox,
    /// The first run, until it is finished.
    #[cfg(feature = "package")]
    first_run: Option<corundum_framework_gui::first_run::FirstRun>,
}

impl eframe::App for BridgeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(page) = self.inbox.take() {
            self.settings.open_page(page);
        }
        #[cfg(feature = "package")]
        if let Some(first_run) = &mut self.first_run {
            let finished = egui::CentralPanel::default()
                .show(ui, |ui| first_run.ui(ui))
                .inner;
            if finished {
                self.first_run = None;
            }
            return;
        }
        egui::CentralPanel::default().show(ui, |ui| self.settings.ui(ui));
    }
}

#[cfg(test)]
mod log_tests {
    // The window prints its renderer's warnings, but not the package runtime's (#366).
    #[test]
    fn the_window_filter_quiets_the_package_runtime() {
        assert!(super::WINDOW_LOG_FILTER.contains("velopack=error"));
        assert!(super::WINDOW_LOG_FILTER.contains("corundum_framework_package=warn"));
    }
}
