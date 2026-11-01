//! Sapphire Bridge's desktop app (#363).
//!
//! Bare, it opens Sapphire Bridge's window, on `--page` (Status by default), or brings the
//! window already open to the front on that page. With a subcommand it runs the same code
//! as `sapphire-bridge`, so the service manager can start it with `serve`. The window is a
//! client of the bridge: closing it stops nothing.

// In the package, a GUI binary on Windows: no console window behind the window or the
// service. The command line goes through the console shim (src/bin/console.rs).
#![cfg_attr(all(windows, feature = "velopack"), windows_subsystem = "windows")]

#[cfg(feature = "velopack")]
mod package;

use clap::Parser;
use corundum_framework_platform::launcher::LauncherSpec;
use sapphire_bridge::BridgeCommand;
use sapphire_framework_gui::bridge_window::BridgePage;

#[derive(Parser, Debug)]
#[command(name = "sapphire-bridge-desktop", version, about)]
struct Desktop {
    /// The page the window opens on.
    #[arg(long, value_parser = parse_page, default_value = "status")]
    page: BridgePage,
    #[command(subcommand)]
    command: Option<BridgeCommand>,
}

/// Sapphire Bridge in the OS's app menu, and the window's id and icon. As the package's
/// (`release-bridge.yml`): `org.sapphire-turtle.sapphire-bridge` is its macOS bundle id and
/// `Network` its Linux category. Not the packId (`sapphire-bridge-desktop`), as corundum's
/// demo.
const LAUNCHER: LauncherSpec = LauncherSpec {
    id: "sapphire-bridge",
    name: "Sapphire Bridge",
    description: "Sync and embedding for Sapphire apps",
    category: "Network",
    icon_svg: include_bytes!("../assets/app-icon.svg"),
    bundle_prefix: "org.sapphire-turtle",
};

/// What the window opens with, on `page`.
fn window_options(page: BridgePage) -> sapphire_framework_gui::bridge_window::BridgeWindowOptions {
    sapphire_framework_gui::bridge_window::BridgeWindowOptions {
        title: "Sapphire Bridge".to_owned(),
        version: env!("CARGO_PKG_VERSION"),
        page,
        launcher: Some(LAUNCHER),
        #[cfg(feature = "velopack")]
        package: Some(sapphire_framework_gui::bridge_window::PackageWindow {
            start_updater: package::start_updater,
            service: package::bridge_service,
            cli: package::PACKAGE.cli,
        }),
    }
}

fn parse_page(s: &str) -> Result<BridgePage, String> {
    BridgePage::parse(s).ok_or_else(|| {
        let pages: Vec<&str> = BridgePage::ALL.iter().map(|p| p.as_str()).collect();
        format!("no page {s:?}: one of {}", pages.join(", "))
    })
}

/// Whether this start goes through the package's start: every start but `serve`, the
/// package's service (the start starts the services an update stopped, and that is this
/// process). Velopack's hooks pass arguments clap rejects, so this is decided before clap
/// may exit on them.
#[cfg_attr(not(feature = "velopack"), allow(dead_code))]
fn runs_package_start(parsed: &Result<Desktop, clap::Error>) -> bool {
    !matches!(
        parsed,
        Ok(Desktop {
            command: Some(BridgeCommand::Serve),
            ..
        })
    )
}

fn main() -> std::process::ExitCode {
    let parsed = Desktop::try_parse();
    // The console first, so the package's start below logs where this run prints: the
    // window's filter for a window start, the command line's otherwise.
    if matches!(parsed, Ok(Desktop { command: None, .. })) {
        sapphire_framework_gui::bridge_window::install_logging();
    } else {
        sapphire_bridge::install_console();
    }
    // Velopack's start: a hook runs and exits here.
    #[cfg(feature = "velopack")]
    if runs_package_start(&parsed) {
        let start = corundum_framework_package::start(&package::PACKAGE);
        tracing::debug!("package start: {start:?}");
    }
    let cli = parsed.unwrap_or_else(|e| e.exit());
    let Some(command) = cli.command else {
        return match sapphire_framework_gui::bridge_window::run_bridge(window_options(cli.page)) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("sapphire-bridge-desktop: cannot open the window: {err}");
                eprintln!();
                let _ = <Desktop as clap::CommandFactory>::command().print_help();
                std::process::ExitCode::FAILURE
            }
        };
    };
    // `update` replaces the running binary with `sapphire-bridge`'s release file, which
    // has no window: the desktop app is updated from its own release file instead, until it
    // is packaged (#363 part C).
    // In the package, the package's updater does it.
    #[cfg(feature = "velopack")]
    if let BridgeCommand::Update(args) = &command
        && corundum_framework_platform::package::current().is_some()
    {
        return match package::updater() {
            Some(updater) => std::process::ExitCode::from(package::package_update(
                &updater,
                args.check,
                &mut std::io::stdout(),
                std::time::Duration::from_secs(5 * 60),
            ) as u8),
            None => std::process::ExitCode::FAILURE,
        };
    }
    if matches!(command, BridgeCommand::Update(_)) {
        eprintln!(
            "sapphire-bridge-desktop is not updated by `update`, which would put sapphire-bridge \
             (no window) in its place: download sapphire-bridge-desktop from \
             https://github.com/sapphire-turtle/sapphire-framework/releases/latest"
        );
        return std::process::ExitCode::FAILURE;
    }
    // The command line's own code, as `sapphire-bridge` runs it, on the bridge's subscriber
    // (installed above), which `serve`'s file layer joins.
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("sapphire-bridge-desktop: {err}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let dispatched = runtime.block_on(command.dispatch_with(
        env!("CARGO_PKG_VERSION"),
        Some(sapphire_framework_bridge_embed::factory::factory()),
    ));
    match dispatched {
        Ok(code) => std::process::ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("sapphire-bridge: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_is_parsed() {
        let d = Desktop::try_parse_from(["x", "--page", "devices"]).unwrap();
        assert_eq!(d.page, BridgePage::Devices);
        assert!(d.command.is_none());
        assert_eq!(
            Desktop::try_parse_from(["x"]).unwrap().page,
            BridgePage::Status
        );
        assert!(Desktop::try_parse_from(["x", "--page", "nope"]).is_err());
    }

    // Velopack's hooks start the binary with arguments clap does not know; the package's
    // start must see them before clap exits on them.
    #[test]
    fn package_start_runs_before_clap_rejects() {
        let parsed = |args: &[&str]| Desktop::try_parse_from(args);
        assert!(runs_package_start(&parsed(&[
            "x",
            "--veloapp-install",
            "1.0.0"
        ])));
        assert!(runs_package_start(&parsed(&["x", "--version"])));
        assert!(runs_package_start(&parsed(&["x"])));
        assert!(runs_package_start(&parsed(&["x", "status"])));
        assert!(!runs_package_start(&parsed(&["x", "serve"])));
    }

    // The window shows the package's icon even outside a package, and goes by the
    // package's bundle id, so the OS groups it with the installer's entry.
    #[test]
    fn the_window_has_sapphire_bridges_icon_and_id() {
        let options = window_options(BridgePage::Status);
        let launcher = options.launcher.expect("a launcher spec");
        assert_eq!(launcher.id, "sapphire-bridge");
        assert_eq!(launcher.category, "Network");
        assert_eq!(launcher.name, "Sapphire Bridge");
        assert_eq!(launcher.bundle_prefix, "org.sapphire-turtle");
        assert_eq!(
            launcher.icon_svg,
            include_bytes!("../assets/app-icon.svg").as_slice()
        );
        assert!(corundum_framework_platform::icon::render_rgba(launcher.icon_svg, 64).is_ok());
    }
}
