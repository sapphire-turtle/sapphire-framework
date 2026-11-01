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
}

/// Open Sapphire Bridge's window and run it until it closes; when it is open already, bring
/// that one to the front on `options.page` and return. Closing the window stops nothing:
/// the bridge is its own process (`serve`).
pub fn run_bridge(options: BridgeWindowOptions) -> eframe::Result<()> {
    let BridgeWindowOptions {
        title,
        version,
        page,
    } = options;
    install_console_with_filter(
        APP_NAME,
        "warn,sapphire_bridge_desktop=info,sapphire_framework_gui=info",
    );
    let Some(instance) = instance::acquire_or_activate_with(APP_NAME, Some(page.as_str())) else {
        tracing::info!("Sapphire Bridge's window is open already: brought it to the front");
        return Ok(());
    };
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
        settings.open_page(page);
        Box::new(BridgeApp { settings, inbox })
    });
    drop(runtime);
    result
}

/// The window: the bridge's Settings, opening the page an activation names.
struct BridgeApp {
    settings: Settings,
    inbox: PageInbox,
}

impl eframe::App for BridgeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(page) = self.inbox.take() {
            self.settings.open_page(page);
        }
        egui::CentralPanel::default().show(ui, |ui| self.settings.ui(ui));
    }
}
