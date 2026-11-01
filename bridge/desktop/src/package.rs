//! Sapphire Bridge as a Velopack package (#363 part C), on corundum-framework-package:
//! [`PACKAGE`] is what `main` hands to `package::start`, its one service is the bridge's
//! `serve`, and its command goes on `PATH` as `sapphire-bridge`.

use std::io::Write as _;
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

/// The package's updater, for `update` on the command line: no update applied at start and
/// no background look; `None` (logged) when it cannot be made.
pub fn updater() -> Option<Updater> {
    let source = Source::Github {
        repo: REPO.to_owned(),
        prerelease: false,
    };
    Updater::new(&PACKAGE, source)
        .inspect_err(|err| eprintln!("Sapphire Bridge's updater: {err}"))
        .ok()
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

/// `update` in the package, through `updater`: look (and download what is found), wait up
/// to `timeout`, then report; without `check_only`, apply what is ready (the process exits
/// for Velopack to replace it). The exit code.
pub fn package_update(
    updater: &Updater,
    check_only: bool,
    out: &mut dyn std::io::Write,
    timeout: Duration,
) -> i32 {
    use corundum_framework_update::PackageState;
    // What is downloaded already needs no look.
    if !matches!(updater.status().state, PackageState::Ready { .. }) {
        updater.check_now();
    }
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        let status = updater.status();
        match status.state {
            PackageState::Checking | PackageState::Downloading { .. }
                if std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(200));
            }
            _ => break status,
        }
    };
    let current = status.current.map_or_else(String::new, |v| format!(" {v}"));
    let said = match status.state {
        PackageState::UpToDate | PackageState::Idle => {
            writeln!(out, "Sapphire Bridge{current} is the newest")
        }
        PackageState::Ready { version } if check_only => writeln!(
            out,
            "Sapphire Bridge {version} is ready; run `sapphire-bridge update` to install it"
        ),
        PackageState::Ready { version } => {
            let _ = writeln!(out, "installing Sapphire Bridge {version}");
            return match updater.apply(corundum_framework_package::After::Exit) {
                Ok(()) => 0,
                Err(err) => {
                    let _ = writeln!(out, "could not install the update: {err}");
                    1
                }
            };
        }
        PackageState::Failed(reason) => {
            let _ = writeln!(out, "could not look for an update: {reason}");
            return 1;
        }
        PackageState::Checking | PackageState::Downloading { .. } => {
            let _ = writeln!(out, "the update did not finish in time");
            return 1;
        }
    };
    if said.is_err() {
        return 1;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use corundum_framework_update::PackageState;

    fn run(state: PackageState, check_only: bool) -> (i32, String) {
        let updater = Updater::for_tests(state);
        let mut out = Vec::new();
        let code = package_update(&updater, check_only, &mut out, Duration::from_secs(1));
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn up_to_date_says_so() {
        let (code, out) = run(PackageState::UpToDate, true);
        assert_eq!(code, 0);
        assert!(out.contains("newest"), "{out}");
    }

    #[test]
    fn ready_with_check_reports_the_version() {
        let ready = PackageState::Ready {
            version: "9.9.9".parse().unwrap(),
        };
        let (code, out) = run(ready, true);
        assert_eq!(code, 0);
        assert!(out.contains("9.9.9"), "{out}");
    }

    #[test]
    fn ready_without_check_applies() {
        let ready = PackageState::Ready {
            version: "9.9.9".parse().unwrap(),
        };
        assert_eq!(run(ready, false).0, 0);
    }

    #[test]
    fn failed_is_an_error() {
        let (code, out) = run(PackageState::Failed("no network".into()), true);
        assert_eq!(code, 1);
        assert!(out.contains("no network"), "{out}");
    }

    #[test]
    fn package_names_the_bridge_and_its_service() {
        assert_eq!(PACKAGE.app_name, "sapphire-bridge");
        assert_eq!(PACKAGE.cli.as_ref().unwrap().command, "sapphire-bridge");
        let service = bridge_service();
        assert_eq!(service.app_name, "sapphire-bridge");
        assert_eq!(service.args, ["serve"]);
    }
}
