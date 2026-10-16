//! The Android entry: GameActivity's `android_main` hands its `AndroidApp` here.

pub use android_activity::AndroidApp;
use sapphire_workspace::{AppContext, AppKind};

use super::{BuildServer, MobileOptions, prepare_dirs, start_services};
use crate::app::{AppSetup, gpu, open_window};
use crate::client::ClientConfig;

/// Run the app until Android destroys its activity: the directories under the app's
/// internal storage, the bridge and the server in-process, and the window.
///
/// Call it from the app's `android_main`. Everything it started stops when it returns;
/// Android may call `android_main` again in the same process, and it then starts afresh.
pub fn run_android(
    android_app: AndroidApp,
    options: MobileOptions,
    server: BuildServer,
    create: impl FnOnce(AppSetup<'_>) -> Box<dyn eframe::App>,
) -> eframe::Result<()> {
    let MobileOptions { app, title, fonts } = options;
    super::logcat::install(app.app_name);
    gpu::install_panic_hook();
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

    let native = eframe::NativeOptions {
        android_app: Some(android_app),
        wgpu_options: gpu::wgpu_options(),
        ..Default::default()
    };
    let result = open_window(
        &title,
        native,
        runtime.handle().clone(),
        config,
        data_dir,
        fonts,
        create,
    );
    // The bridge, the server and the clients end with the runtime.
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    result
}
