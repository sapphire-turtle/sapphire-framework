//! `update` on the command line inside a package: through the package's updater.

use std::time::Duration;

use corundum_framework_package::{After, Updater};
use corundum_framework_update::PackageState;

/// `update` through `updater`, for the app `name` whose command is `command`: look (and
/// download what is found), wait up to `timeout`, then report on `out`; without
/// `check_only`, apply what is ready. The exit code.
pub(super) fn package_update(
    name: &str,
    command: &str,
    updater: &Updater,
    check_only: bool,
    out: &mut dyn std::io::Write,
    timeout: Duration,
) -> i32 {
    // What is downloaded already needs no look.
    if !matches!(updater.status().state, PackageState::Ready { .. }) {
        updater.check_now();
    }
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        let status = updater.status();
        if settled(&status.state) || std::time::Instant::now() >= deadline {
            break status;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    report(name, command, updater, status, check_only, out)
}

/// What `update` says and does for `status`, the updater's answer: the exit code.
fn report(
    name: &str,
    command: &str,
    updater: &Updater,
    status: corundum_framework_update::PackageStatus,
    check_only: bool,
    out: &mut dyn std::io::Write,
) -> i32 {
    let current = status.current.map_or_else(String::new, |v| format!(" {v}"));
    let said = match status.state {
        PackageState::UpToDate => writeln!(out, "{name}{current} is the newest"),
        PackageState::Ready { version } if check_only => writeln!(
            out,
            "{name} {version} is ready; run `{command} update` to install it"
        ),
        PackageState::Ready { version } => {
            let _ = writeln!(out, "installing {name} {version}");
            return match updater.apply(after_update()) {
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
        PackageState::Idle | PackageState::Checking | PackageState::Downloading { .. } => {
            let _ = writeln!(out, "the update did not finish in time");
            return 1;
        }
    };
    if said.is_err() {
        return 1;
    }
    0
}

/// Whether the updater has answered the check: a fresh updater is `Idle` until its thread
/// takes the request.
fn settled(state: &PackageState) -> bool {
    !matches!(
        state,
        PackageState::Idle | PackageState::Checking | PackageState::Downloading { .. }
    )
}

/// After `update` applies: the new version starts once with `--version`, so its package
/// start starts the services the update stopped (`serve` skips the package start), and exits.
fn after_update() -> After {
    After::RestartWith(vec!["--version".into()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(state: PackageState, check_only: bool) -> (i32, String) {
        let updater = Updater::for_tests(state);
        let mut out = Vec::new();
        let code = package_update(
            "Sapphire Example",
            "sapphire-example",
            &updater,
            check_only,
            &mut out,
            Duration::from_secs(1),
        );
        (code, String::from_utf8(out).unwrap())
    }

    #[test]
    fn up_to_date_says_so() {
        let (code, out) = run(PackageState::UpToDate, true);
        assert_eq!(code, 0);
        assert!(out.contains("Sapphire Example"), "{out}");
        assert!(out.contains("newest"), "{out}");
    }

    #[test]
    fn ready_with_check_reports_the_version_and_the_command() {
        let ready = PackageState::Ready {
            version: "9.9.9".parse().unwrap(),
        };
        let (code, out) = run(ready, true);
        assert_eq!(code, 0);
        assert!(out.contains("9.9.9"), "{out}");
        assert!(out.contains("`sapphire-example update`"), "{out}");
    }

    #[test]
    fn ready_without_check_applies() {
        let ready = PackageState::Ready {
            version: "9.9.9".parse().unwrap(),
        };
        assert_eq!(run(ready, false).0, 0);
    }

    // On `report` itself: through `package_update` the inert updater's check may answer
    // before the state is read, and it answers up to date (#405).
    #[test]
    fn failed_is_an_error() {
        let updater = Updater::for_tests(PackageState::Idle);
        let status = corundum_framework_update::PackageStatus {
            current: None,
            state: PackageState::Failed("no network".into()),
        };
        let mut out = Vec::new();
        let code = report(
            "Sapphire Example",
            "sapphire-example",
            &updater,
            status,
            true,
            &mut out,
        );
        let out = String::from_utf8(out).unwrap();
        assert_eq!(code, 1);
        assert!(out.contains("no network"), "{out}");
    }

    // A fresh updater is Idle until its thread takes the check.
    #[test]
    fn idle_is_not_an_answer() {
        assert!(!settled(&PackageState::Idle));
        assert!(!settled(&PackageState::Checking));
        assert!(settled(&PackageState::UpToDate));
        assert!(settled(&PackageState::Failed(String::new())));
    }

    // The new version starts once, quietly, so its package start restarts the services the
    // update stopped.
    #[test]
    fn update_restarts_for_the_services() {
        let After::RestartWith(args) = after_update() else {
            panic!("an update from the command line must start the new version");
        };
        assert_eq!(args, ["--version"]);
    }
}
