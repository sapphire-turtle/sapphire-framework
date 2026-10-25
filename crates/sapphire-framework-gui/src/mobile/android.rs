//! The Android entry: GameActivity's `android_main` hands its `AndroidApp` here.

pub use corundum_framework_gui::window::AndroidApp;
use corundum_framework_gui::window::{self, WindowOptions};
use sapphire_workspace::{AppContext, AppKind};

use super::{BuildServer, EntryGuard, MobileOptions, app_log_filter, prepare_dirs, start_services};
use crate::app::{AppSetup, build_app, corundum_store};
use crate::client::ClientConfig;

static ENTRY: EntryGuard = EntryGuard::new();

/// Run the app, then end the process: the directories under the app's internal storage,
/// the bridge and the server in-process, and the window.
///
/// Call it from the app's `android_main`. It never returns. The app starts once per
/// process (see [`EntryGuard`]), so when the window closes the process ends, and an entry
/// Android makes again in the same process — after recreating the activity — ends it too;
/// the next launch then starts afresh.
pub fn run_android(
    android_app: AndroidApp,
    options: MobileOptions,
    server: BuildServer,
    create: impl FnOnce(AppSetup<'_>) -> Box<dyn eframe::App>,
) -> ! {
    let app_name = options.app.app_name;
    corundum_framework_gui::logcat::install(app_name, &app_log_filter(app_name));
    if !ENTRY.first() {
        tracing::warn!("{app_name} was started again in the same process; restarting it");
        std::process::exit(0);
    }
    if let Err(err) = run_once(android_app, options, server, create) {
        tracing::error!("{app_name} could not run: {err}");
    }
    std::process::exit(0);
}

fn run_once(
    android_app: AndroidApp,
    options: MobileOptions,
    server: BuildServer,
    create: impl FnOnce(AppSetup<'_>) -> Box<dyn eframe::App>,
) -> eframe::Result<()> {
    let MobileOptions { app, title, fonts } = options;
    let creation = |e: Box<dyn std::error::Error + Send + Sync>| eframe::Error::AppCreation(e);

    let base = android_app
        .internal_data_path()
        .ok_or_else(|| creation("Android gave no internal data path".into()))?;
    prepare_dirs(&base, app.app_name).map_err(|e| creation(Box::new(e)))?;
    let dirs = AppContext::new(app.app_name);
    dirs.init(AppKind::Server);
    let data_dir = dirs.data_dir().to_owned();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| creation(Box::new(e)))?;
    runtime.block_on(start_services(app.version, server));
    let config = ClientConfig::new(app).map_err(|e| creation(Box::new(e)))?;

    // corundum-framework-gui installs the panic hook, gives its platform calls the activity, and
    // wraps the app in the phone's frame (the insets and the keyboard).
    let mut window_options = WindowOptions::new(title).remember(corundum_store(&dirs));
    if let Some(install) = fonts {
        window_options = window_options.fonts(install);
    }
    let handle = runtime.handle().clone();
    let result = window::run_android(android_app, window_options, move |cc| {
        build_app(cc, handle, config, data_dir, create)
    });
    // The bridge, the server and the clients end with the runtime.
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    result
}
