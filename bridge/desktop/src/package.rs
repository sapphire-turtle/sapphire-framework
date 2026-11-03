//! Sapphire Bridge as a cargo-packager package (#363 part C), on corundum-framework-package:
//! [`PACKAGE`] is what `main` hands to `package::start`, its one service is the bridge's
//! `serve`, and its command goes on `PATH` as `sapphire-bridge`. The updater and `update`
//! are the framework's, shared with the apps' packages ([`BRIDGE`]).

use corundum_framework_package::CliSpec;
use corundum_framework_package::{PackageSpec, Services, Updater};
use sapphire_bridge::{ServiceSpec, bridge_service_spec};
use sapphire_framework_gui::package::{AppPackage, UPDATE_PUBKEY};

/// The package: its command, and the bridge's service, which an update stops and starts.
pub static PACKAGE: PackageSpec = PackageSpec {
    app_name: "sapphire-bridge",
    version: env!("CARGO_PKG_VERSION"),
    update_pubkey: UPDATE_PUBKEY,
    on_installed: None,
    on_uninstalling: None,
    before_apply: None,
    cli: Some(CliSpec {
        command: "sapphire-bridge",
    }),
    services: Services::new(&[bridge_service]),
};

/// The package as the framework runs it: released with the framework's `v<version>`
/// releases.
pub const BRIDGE: AppPackage = AppPackage {
    name: "Sapphire Bridge",
    github: ("sapphire-turtle", "sapphire-framework"),
    spec: &PACKAGE,
    service: bridge_service,
};

/// The bridge's service, for this binary's version.
pub fn bridge_service() -> ServiceSpec {
    bridge_service_spec(env!("CARGO_PKG_VERSION"))
}

/// The package's updater, after applying an update downloaded at an earlier run and looking
/// for one now and every day; `None` outside a package or when it cannot be made (logged).
pub fn start_updater() -> Option<Updater> {
    BRIDGE.start_updater()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_the_bridge_and_its_service() {
        assert_eq!(PACKAGE.app_name, "sapphire-bridge");
        assert_eq!(PACKAGE.cli.as_ref().unwrap().command, "sapphire-bridge");
        let service = bridge_service();
        assert_eq!(service.app_name, "sapphire-bridge");
        assert_eq!(service.args, ["serve"]);
        assert_eq!(BRIDGE.desktop_asset(), "sapphire-bridge-desktop");
    }
}
