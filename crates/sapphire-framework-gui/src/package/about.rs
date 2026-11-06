//! About in a package: the package's updates, the service's row, the command line and the
//! app's entry in the OS's app menu, for an app's window and Sapphire Bridge's.

use corundum_framework_gui::settings::AboutRows;
use corundum_framework_gui::update::semver;
use corundum_framework_gui::update::{AfterUpdate, Target, UpdateTarget, UpdatesPanel};
use corundum_framework_package::{CliSpec, Updater};
use corundum_framework_service::ServiceSpec;

/// The rows About adds in a package: corundum's [`AboutRows`]. The app-menu row is the
/// window's own spec ([`AppOptions::launcher`](crate::app::AppOptions::launcher)): an
/// AppImage's window writes no app-menu entry of its own, so this row is where the person
/// adds it, and with it the app's icon in the dock.
pub struct PackageAbout {
    rows: AboutRows,
    /// Whether a command-line row is there, for the tests.
    #[cfg(test)]
    offers_cli: bool,
}

impl PackageAbout {
    /// Sapphire Bridge's rows, in the window `ctx` belongs to: for `service`, the package's
    /// command `cli` and its `updater` (none outside a package: the binary's release row
    /// instead).
    pub fn new(
        ctx: &egui::Context,
        updater: Option<Updater>,
        service: fn() -> ServiceSpec,
        cli: Option<CliSpec>,
        version: &str,
    ) -> PackageAbout {
        let release = corundum_framework_update::ReleaseSource::github(
            "sapphire-turtle",
            "sapphire-framework",
        )
        .asset("sapphire-bridge-desktop");
        PackageAbout::with_release(
            ctx,
            "Sapphire Bridge",
            release,
            "sapphire-bridge-desktop",
            updater,
            service,
            cli,
            version,
        )
    }

    /// An app's rows, in the window `ctx` belongs to, for `package` at `version` and its
    /// `updater` (none outside a package: the app's `<app>-desktop` release row instead).
    pub fn for_app(
        ctx: &egui::Context,
        package: &super::AppPackage,
        updater: Option<Updater>,
        version: &str,
    ) -> PackageAbout {
        let (owner, repo) = package.github;
        let asset = package.desktop_asset();
        let release = corundum_framework_update::ReleaseSource::github(owner, repo).asset(&asset);
        PackageAbout::with_release(
            ctx,
            package.name,
            release,
            &asset,
            updater,
            package.service,
            package.spec.cli,
            version,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_release(
        ctx: &egui::Context,
        name: &str,
        release: corundum_framework_update::ReleaseSource,
        user_agent: &str,
        updater: Option<Updater>,
        service: fn() -> ServiceSpec,
        cli: Option<CliSpec>,
        version: &str,
    ) -> PackageAbout {
        let binary = UpdateTarget {
            label: name.to_owned(),
            current: semver::Version::parse(version).ok(),
            source: release,
            exe: std::env::current_exe().unwrap_or_default(),
            after: AfterUpdate::Relaunch,
        };
        let target = Target::this_app(name.to_owned(), updater, binary);
        let mut rows = AboutRows::for_window(ctx)
            .updates(UpdatesPanel::new(
                vec![target],
                &format!("{user_agent}/{version}"),
            ))
            .service(service);
        #[cfg(test)]
        let offers_cli = cli.is_some();
        if let Some(cli) = cli {
            rows = rows.cli(cli);
        }
        PackageAbout {
            rows,
            #[cfg(test)]
            offers_cli,
        }
    }

    /// Draw the rows.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.rows.ui(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc() -> ServiceSpec {
        ServiceSpec {
            app_name: "sapphire-bridge",
            description: String::new(),
            args: vec!["serve".into()],
            legacy_labels: Vec::new(),
            post_install: None,
        }
    }

    // macOS and Linux put the command on `PATH` only when asked, from this row.
    #[test]
    fn about_offers_the_command_line() {
        let cli = corundum_framework_package::CliSpec {
            command: "sapphire-bridge",
        };
        let ctx = egui::Context::default();
        assert!(PackageAbout::new(&ctx, None, svc, Some(cli), "0.0.0").offers_cli);
        assert!(!PackageAbout::new(&ctx, None, svc, None, "0.0.0").offers_cli);
    }

    #[test]
    fn an_apps_about_offers_its_command_line() {
        let ctx = egui::Context::default();
        let about = PackageAbout::for_app(&ctx, &crate::package::tests::EXAMPLE, None, "1.2.3");
        assert!(about.offers_cli);
    }
}
