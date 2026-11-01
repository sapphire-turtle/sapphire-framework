//! Sapphire Bridge's desktop app (#363).
//!
//! Bare, it opens Sapphire Bridge's window, on `--page` (Status by default), or brings the
//! window already open to the front on that page. With a subcommand it runs the same code
//! as `sapphire-bridge`, so the service manager can start it with `serve`. The window is a
//! client of the bridge: closing it stops nothing.

use clap::Parser;
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

fn parse_page(s: &str) -> Result<BridgePage, String> {
    BridgePage::parse(s).ok_or_else(|| {
        let pages: Vec<&str> = BridgePage::ALL.iter().map(|p| p.as_str()).collect();
        format!("no page {s:?}: one of {}", pages.join(", "))
    })
}

fn main() -> std::process::ExitCode {
    let cli = Desktop::parse();
    let Some(command) = cli.command else {
        let options = sapphire_framework_gui::bridge_window::BridgeWindowOptions {
            title: "Sapphire Bridge".to_owned(),
            version: env!("CARGO_PKG_VERSION"),
            page: cli.page,
        };
        return match sapphire_framework_gui::bridge_window::run_bridge(options) {
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
    if matches!(command, BridgeCommand::Update(_)) {
        eprintln!(
            "sapphire-bridge-desktop is not updated by `update`, which would put sapphire-bridge \
             (no window) in its place: download sapphire-bridge-desktop from \
             https://github.com/sapphire-turtle/sapphire-framework/releases/latest"
        );
        return std::process::ExitCode::FAILURE;
    }
    // The command line's own code, as `sapphire-bridge` runs it: the bridge's subscriber
    // first, which `serve`'s file layer joins.
    sapphire_bridge::install_console();
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
}
