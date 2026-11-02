//! About in a package: the service's row, the command line and the package's updates, for
//! an app's window and Sapphire Bridge's.

use corundum_framework_gui::settings::{CliRow, ServiceRow};
use corundum_framework_gui::update::semver;
use corundum_framework_gui::update::{AfterUpdate, Target, UpdateTarget, UpdatesPanel};
use corundum_framework_package::{CliSpec, Updater};
use corundum_framework_service::ServiceSpec;

/// The rows About adds in a package.
pub struct PackageAbout {
    service: ServiceRow,
    /// The command on `PATH`: macOS and Linux link it only when asked, here.
    cli: Option<CliRow>,
    updates: UpdatesPanel,
}

impl PackageAbout {
    /// Sapphire Bridge's rows: for `service`, the package's command `cli` and its `updater`
    /// (none outside a package: the binary's release row instead).
    pub fn new(
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
            "Sapphire Bridge",
            release,
            "sapphire-bridge-desktop",
            updater,
            service,
            cli,
            version,
        )
    }

    /// An app's rows, for `package` at `version` and its `updater` (none outside a package:
    /// the app's `<app>-desktop` release row instead).
    pub fn for_app(
        package: &super::AppPackage,
        updater: Option<Updater>,
        version: &str,
    ) -> PackageAbout {
        let (owner, repo) = package.github;
        let asset = package.desktop_asset();
        let release = corundum_framework_update::ReleaseSource::github(owner, repo).asset(&asset);
        PackageAbout::with_release(
            package.name,
            release,
            &asset,
            updater,
            package.service,
            package.spec.cli,
            version,
        )
    }

    fn with_release(
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
        PackageAbout {
            service: ServiceRow::new(service),
            cli: cli.map(CliRow::new),
            updates: UpdatesPanel::new(vec![target], &format!("{user_agent}/{version}")),
        }
    }

    /// Draw the rows.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.service.ui(ui);
        if let Some(cli) = &mut self.cli {
            cli.ui(ui);
        }
        ui.add_space(8.0);
        self.updates.ui(ui);
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
        assert!(
            PackageAbout::new(None, svc, Some(cli), "0.0.0")
                .cli
                .is_some()
        );
        assert!(PackageAbout::new(None, svc, None, "0.0.0").cli.is_none());
    }

    #[test]
    fn an_apps_about_offers_its_command_line() {
        let about = PackageAbout::for_app(&crate::package::tests::EXAMPLE, None, "1.2.3");
        assert!(about.cli.is_some());
    }
}
