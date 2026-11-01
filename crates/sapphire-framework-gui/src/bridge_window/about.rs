//! The bridge window's About in a package: its service's row and its updates.

use corundum_framework_gui::settings::ServiceRow;
use corundum_framework_gui::update::semver;
use corundum_framework_gui::update::{AfterUpdate, Target, UpdateTarget, UpdatesPanel};
use corundum_framework_package::Updater;
use corundum_framework_service::ServiceSpec;

/// The rows About adds in a package.
pub struct PackageAbout {
    service: ServiceRow,
    updates: UpdatesPanel,
}

impl PackageAbout {
    /// The rows for `service` and the package's `updater` (none outside a package: the
    /// binary's release row instead).
    pub fn new(
        updater: Option<Updater>,
        service: fn() -> ServiceSpec,
        version: &str,
    ) -> PackageAbout {
        let binary = UpdateTarget {
            label: "Sapphire Bridge".to_owned(),
            current: semver::Version::parse(version).ok(),
            source: corundum_framework_update::ReleaseSource::github(
                "sapphire-turtle",
                "sapphire-framework",
            )
            .asset("sapphire-bridge-desktop"),
            exe: std::env::current_exe().unwrap_or_default(),
            after: AfterUpdate::Relaunch,
        };
        let target = Target::this_app("Sapphire Bridge".to_owned(), updater, binary);
        PackageAbout {
            service: ServiceRow::new(service),
            updates: UpdatesPanel::new(vec![target], &format!("sapphire-bridge-desktop/{version}")),
        }
    }

    /// Draw the rows.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.service.ui(ui);
        ui.add_space(8.0);
        self.updates.ui(ui);
    }
}
