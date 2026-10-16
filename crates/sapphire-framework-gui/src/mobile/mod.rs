//! A sapphire app on a phone: the bridge, the app server and the window in one process.
//!
//! A desktop app is a pure client of two service-managed processes. A phone has no service
//! manager and no shared directory, so the app hosts both itself, for as long as it runs,
//! and they talk over the in-process IPC carrier — no Unix socket, whose path an iOS
//! container would overflow. Everything above the carrier is the desktop's: the same
//! handshakes, the same [`FrameworkClient`](crate::client::FrameworkClient), the same views.
//!
//! An app's mobile crate calls the platform's entry ([`run_android`]; iOS to follow), giving
//! it the app's identity, how to build its sync-mounted server, and its `eframe::App`.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
mod logcat;

use std::io;
use std::path::{Path, PathBuf};

use sapphire_framework_server::AppServer;
use sapphire_ipc::Carrier;
use sapphire_workspace::app_dirs::app_dir_env_var;

use crate::app::InstallFonts;
use crate::client::AppIdentity;

#[cfg(target_os = "android")]
pub use android::{AndroidApp, run_android};

/// Builds the app's server once the directories are in place. It initialises the app's
/// own `AppContext` (`CTX.init(AppKind::Server)`: this process is the server) and returns
/// the server with sync mounted, exactly as its `serve` does.
pub type BuildServer =
    Box<dyn FnOnce() -> Result<AppServer, Box<dyn std::error::Error + Send + Sync>> + Send>;

/// What the platform entry needs to know about the app.
// Read by the platform entries, which a desktop build of this module has none of.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub struct MobileOptions {
    pub(crate) app: AppIdentity,
    pub(crate) title: String,
    pub(crate) fonts: Option<InstallFonts>,
}

impl MobileOptions {
    /// `app`, titled `title` where the platform shows one.
    pub fn new(app: AppIdentity, title: impl Into<String>) -> MobileOptions {
        MobileOptions {
            app,
            title: title.into(),
            fonts: None,
        }
    }

    /// Set the fonts with `install` instead of the default (egui's own, with the system's
    /// CJK font behind them).
    pub fn fonts(mut self, install: impl FnOnce(&egui::Context) + 'static) -> MobileOptions {
        self.fonts = Some(Box::new(install));
        self
    }
}

/// The directory new workspaces go in, under the app's storage `base`.
pub fn workspaces_dir(base: &Path) -> PathBuf {
    base.join("workspaces")
}

/// Point every directory the framework resolves at `base`, the app's private storage.
///
/// `dirs` finds nothing on Android or iOS (it reads only `$HOME`), so the framework's
/// overrides are set instead: the IPC runtime directory, the bridge's directory, and the
/// app's data, cache and config roots. Each is created.
///
/// Call it before anything reads them, and before this process starts threads of its own.
pub fn prepare_dirs(base: &Path, app_name: &str) -> io::Result<()> {
    let mut vars = vec![
        (sapphire_ipc::RUNTIME_DIR_ENV.to_owned(), base.join("run")),
        (
            sapphire_framework_bridge::BRIDGE_DIR_ENV.to_owned(),
            base.join("bridge"),
        ),
    ];
    for category in ["data", "cache", "config"] {
        vars.push((app_dir_env_var(app_name, category), base.join(category)));
    }
    for (var, dir) in vars {
        std::fs::create_dir_all(&dir)?;
        // SAFETY: called at start-up, before this process's own threads exist. The
        // platform's threads (a JVM's, say) do not read Rust's environment.
        unsafe { std::env::set_var(var, dir) };
    }
    std::fs::create_dir_all(workspaces_dir(base))?;
    crate::platform::set_app_storage(workspaces_dir(base));
    Ok(())
}

/// Start the bridge and the app server inside this process, on the current runtime.
///
/// The carrier is switched to in-process first, so neither binds a socket. A failure is
/// logged, not returned: the window still opens, and Settings shows the process that is
/// missing — which is where a person looks.
pub async fn start_services(version: &'static str, build: BuildServer) {
    sapphire_ipc::set_carrier(Carrier::InProcess);
    match sapphire_framework_bridge::serve_in_process(version).await {
        Ok(task) => {
            tokio::spawn(async move {
                match task.await {
                    Ok(Ok(())) => tracing::info!("the bridge stopped"),
                    Ok(Err(err)) => tracing::error!("the bridge stopped: {err}"),
                    Err(err) => tracing::error!("the bridge task failed: {err}"),
                }
            });
        }
        Err(err) => tracing::error!("could not start the bridge: {err}"),
    }
    match build() {
        Ok(server) => {
            tokio::spawn(async move {
                if let Err(err) = server.signals(false).run().await {
                    tracing::error!("the app server stopped: {err}");
                }
            });
        }
        Err(err) => tracing::error!("could not build the app server: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_dirs_sets_every_variable() {
        let tmp = tempfile::tempdir().unwrap();
        prepare_dirs(tmp.path(), "sapphire-mobiletest").unwrap();
        for (var, sub) in [
            ("SAPPHIRE_RUNTIME_DIR", "run"),
            ("SAPPHIRE_BRIDGE_DIR", "bridge"),
            ("SAPPHIRE_MOBILETEST_DATA_DIR", "data"),
            ("SAPPHIRE_MOBILETEST_CACHE_DIR", "cache"),
            ("SAPPHIRE_MOBILETEST_CONFIG_DIR", "config"),
        ] {
            let value = PathBuf::from(std::env::var_os(var).expect(var));
            assert_eq!(value, tmp.path().join(sub), "{var}");
            assert!(value.is_dir(), "{var} is created");
        }
        assert!(workspaces_dir(tmp.path()).is_dir());
        assert_eq!(
            crate::platform::app_storage(),
            Some(workspaces_dir(tmp.path()).as_path())
        );
    }
}
