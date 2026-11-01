//! Sapphire Bridge as a Velopack package (#363 part C), on corundum-framework-package:
//! [`PACKAGE`] is what `main` hands to `package::start`, its one service is the bridge's
//! `serve`, and its command goes on `PATH` as `sapphire-bridge`.

use std::time::Duration;

use corundum_framework_package::CliSpec;
use corundum_framework_package::{PackageSpec, Services, Source, Updater};
use sapphire_bridge::{ServiceSpec, bridge_service_spec};

/// Where the packages are released: the framework's `v<version>` releases.
pub const REPO: &str = "https://github.com/sapphire-turtle/sapphire-framework";

/// How often a running window looks for an update.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The package: its command, and the bridge's service, which an update stops and starts.
pub static PACKAGE: PackageSpec = PackageSpec {
    app_name: "sapphire-bridge",
    on_installed: None,
    on_uninstalling: None,
    before_apply: None,
    cli: Some(CliSpec {
        command: "sapphire-bridge",
    }),
    services: Services::new(&[bridge_service]),
};

/// The bridge's service, for this binary's version.
pub fn bridge_service() -> ServiceSpec {
    bridge_service_spec(env!("CARGO_PKG_VERSION"))
}

/// The package's updater, after applying an update downloaded at an earlier run (which
/// restarts into the new version) and looking for one now and every [`CHECK_EVERY`];
/// `None` outside a package or when it cannot be made (logged).
pub fn start_updater() -> Option<Updater> {
    corundum_framework_platform::package::current()?;
    let source = Source::Github {
        repo: REPO.to_owned(),
        prerelease: false,
    };
    let updater = match Updater::new(&PACKAGE, source) {
        Ok(updater) => updater,
        Err(err) => {
            tracing::warn!("Sapphire Bridge's updater: {err}");
            return None;
        }
    };
    if let Err(err) = updater.apply_pending_at_start() {
        tracing::warn!("could not apply the downloaded update: {err}");
    }
    updater.run_in_background(CHECK_EVERY);
    Some(updater)
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
    }
}
