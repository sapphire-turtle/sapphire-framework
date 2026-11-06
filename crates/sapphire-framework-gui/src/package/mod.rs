//! A desktop app as a cargo-packager package (#357, #403): what its `main`, its window and
//! its `update` share with Sapphire Bridge's (#363).
//!
//! An [`AppPackage`] names the package (corundum's [`PackageSpec`]), where it is released and
//! the service its first run offers to keep running. With it:
//!
//! - [`AppOptions::package`](crate::app::AppOptions::package) makes [`app::run`](crate::app::run)
//!   take the window's single-instance lock, start the updater, show the first run before the
//!   app on the first start, and hand [`PackageAbout`] to the app for
//!   [`Settings::with_package_about`](crate::Settings::with_package_about);
//! - [`AppPackage::update`] is `update` on the command line inside the package.

mod about;
mod first_run;
mod update;

use corundum_framework_package::{PackageSpec, Source, Updater};
/// A service's spec: what [`AppPackage::service`] returns.
pub use corundum_framework_service::ServiceSpec;

pub use about::PackageAbout;
/// corundum's package runtime, at the version these types are: `start`, `PackageSpec`,
/// `CliSpec`, `Services` and the Windows console shim's `console::forward`.
pub use corundum_framework_package as runtime;
/// The window's id, icon and app-menu entry ([`AppOptions::launcher`](crate::app::AppOptions::launcher)).
pub use corundum_framework_platform::launcher::LauncherSpec;
pub use first_run::app_first_run_pages;

/// sapphire-turtle's minisign public key, which every desktop app's updates are signed for
/// (sapphire-turtle/.github's `release-packager.yml`, with the organization's
/// `PACKAGER_SIGN_PRIVATE_KEY`): a package's `PackageSpec::update_pubkey`. An update signed
/// otherwise is not installed. The bare binaries are signed with it too (#402).
pub use sapphire_framework_update::UPDATE_PUBKEY;

/// How often a running window looks for an update.
pub const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// A desktop app's cargo-packager package.
#[derive(Clone, Copy)]
pub struct AppPackage {
    /// The app's name as people see it: "Sapphire Sync".
    pub name: &'static str,
    /// The GitHub owner and repository it is released from: `("sapphire-turtle", "sapphire-sync")`.
    pub github: (&'static str, &'static str),
    /// The package: its command on `PATH` and its services, which an update stops and starts.
    pub spec: &'static PackageSpec,
    /// The service the first run and About register: the app server's `serve`.
    pub service: fn() -> ServiceSpec,
}

impl AppPackage {
    /// The release files' name outside a package, as the window's About looks for them:
    /// `<app_name>-desktop`.
    pub fn desktop_asset(&self) -> String {
        format!("{}-desktop", self.spec.app_name)
    }

    /// The command on `PATH`: the package's, else its app name.
    pub fn command(&self) -> &'static str {
        self.spec
            .cli
            .as_ref()
            .map_or(self.spec.app_name, |cli| cli.command)
    }

    fn source(&self) -> Source {
        let (owner, repo) = self.github;
        Source::Github {
            repo: format!("https://github.com/{owner}/{repo}"),
            prerelease: false,
        }
    }

    /// The package's updater, for `update` on the command line: no update applied at start
    /// and no background look; `None` (printed) when it cannot be made.
    pub fn updater(&self) -> Option<Updater> {
        Updater::new(self.spec, self.source())
            .inspect_err(|err| eprintln!("{}'s updater: {err}", self.name))
            .ok()
    }

    /// The package's updater, after applying an update downloaded at an earlier run (which
    /// restarts into the new version) and looking for one now and every [`CHECK_EVERY`];
    /// `None` outside a package or when it cannot be made (logged). Only the start that holds
    /// the window's lock calls it, so a second start never applies an update under the first.
    pub fn start_updater(&self) -> Option<Updater> {
        corundum_framework_platform::package::current()?;
        let updater = match Updater::new(self.spec, self.source()) {
            Ok(updater) => updater,
            Err(err) => {
                tracing::warn!("{}'s updater: {err}", self.name);
                return None;
            }
        };
        if let Err(err) = updater.apply_pending_at_start() {
            tracing::warn!("could not apply the downloaded update: {err}");
        }
        updater.run_in_background(CHECK_EVERY);
        Some(updater)
    }

    /// `update` in the package, through `updater`: look (and download what is found), wait up
    /// to `timeout`, then report on `out`; without `check_only`, apply what is ready (the
    /// process exits for the update's install). The exit code.
    pub fn update(
        &self,
        updater: &Updater,
        check_only: bool,
        out: &mut dyn std::io::Write,
        timeout: std::time::Duration,
    ) -> i32 {
        update::package_update(self.name, self.command(), updater, check_only, out, timeout)
    }
}

/// `app`, behind the first run on the window's first start in a package (corundum's
/// `FirstRun`, which also skips itself outside a package).
#[cfg(feature = "eframe")]
pub(crate) fn first_run_wrapper(
    cc: &eframe::CreationContext<'_>,
    package: &AppPackage,
    app: Box<dyn eframe::App>,
) -> Box<dyn eframe::App> {
    use corundum_framework_gui::first_run::FirstRun;
    if FirstRun::done(&cc.egui_ctx) {
        return app;
    }
    let pages = app_first_run_pages(&cc.egui_ctx, package.name, package.service);
    Box::new(FirstRunApp {
        first_run: Some(FirstRun::new(package.name, pages)),
        app,
    })
}

/// The app, after the first run while it is not finished.
#[cfg(feature = "eframe")]
struct FirstRunApp {
    first_run: Option<corundum_framework_gui::first_run::FirstRun>,
    app: Box<dyn eframe::App>,
}

#[cfg(feature = "eframe")]
impl eframe::App for FirstRunApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if let Some(first_run) = &mut self.first_run {
            let finished = egui::CentralPanel::default()
                .show(ui, |ui| first_run.ui(ui))
                .inner;
            if finished {
                self.first_run = None;
            }
            return;
        }
        self.app.ui(ui, frame);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app.save(storage);
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if self.first_run.is_none() {
            self.app.logic(ctx, frame);
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.app.clear_color(visuals)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use corundum_framework_package::{CliSpec, Services};

    pub(crate) fn svc() -> ServiceSpec {
        ServiceSpec {
            app_name: "sapphire-example",
            description: String::new(),
            args: vec!["serve".into()],
            legacy_labels: Vec::new(),
            post_install: None,
        }
    }

    pub(crate) static SPEC: PackageSpec = PackageSpec {
        app_name: "sapphire-example",
        version: "1.0.0",
        update_pubkey: super::UPDATE_PUBKEY,
        on_installed: None,
        on_uninstalling: None,
        before_apply: None,
        cli: Some(CliSpec {
            command: "sapphire-example",
        }),
        services: Services::new(&[svc]),
    };

    pub(crate) const EXAMPLE: AppPackage = AppPackage {
        name: "Sapphire Example",
        github: ("sapphire-turtle", "sapphire-example"),
        spec: &SPEC,
        service: svc,
    };

    #[test]
    fn the_package_names_its_files_and_command() {
        assert_eq!(EXAMPLE.desktop_asset(), "sapphire-example-desktop");
        assert_eq!(EXAMPLE.command(), "sapphire-example");
        let Source::Github { repo, prerelease } = EXAMPLE.source() else {
            panic!("a GitHub source");
        };
        assert_eq!(repo, "https://github.com/sapphire-turtle/sapphire-example");
        assert!(!prerelease);
    }

    // Outside a package (a test, a cargo build) there is no updater to start.
    #[test]
    fn no_updater_outside_a_package() {
        assert!(EXAMPLE.start_updater().is_none());
    }
}
