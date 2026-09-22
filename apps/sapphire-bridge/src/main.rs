//! The host-wide sapphire daemon.
//!
//! Running it with no subcommand starts the bridge; every other subcommand is a one-shot
//! command against a running one. Two of them work directly on the bridge directory instead —
//! `workgroup create`, because there is nothing to ask about a workgroup that does not exist
//! yet, and `device forget`, because the control plane has no method for it. Both then see
//! their effect immediately: the ledger is re-read on every authorization.
//!
//! `service install` registers this binary with the OS service manager: a unit that runs
//! `sapphire-bridge run` and nothing else, described by [`bridge_service_spec`] through the
//! command type the bridge re-exports.
//! command against a running one. Two of them work directly on the bridge directory instead —
//! `workgroup create`, because there is nothing to ask about a workgroup that does not exist
//! yet, and `device forget`, because the control plane has no method for it. Both then see
//! their effect immediately: the ledger is re-read on every authorization.
//!
//! `service install` registers this binary with the OS service manager: a unit that runs
//! `sapphire-bridge run` and nothing else, described by `bridge_service_spec` through the
//! command type the bridge re-exports.

use clap::Parser;
use sapphire_bridge::{BridgeCommand, ServiceCommand, bridge_service_spec};

#[derive(Parser)]
#[command(name = "sapphire-bridge", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<BridgeCommand>,
}

// The `service` subcommand is this type; naming it here is what keeps it in the bridge's
// command surface and not a second dependency's.
const _: fn() = || {
    let _ = ServiceCommand::Uninstall;
    let _ = bridge_service_spec;
};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    // The bridge's own subscriber: the console, and — once `run` installs the log — the
    // file layer. `tracing` allows one global subscriber per process, so the binary does
    // not install its own and the bridge's file layer is not shut out.
    sapphire_bridge::install_console();

    let cli = Cli::parse();
    let command = cli.command.unwrap_or(BridgeCommand::Run);
    match command.dispatch(env!("CARGO_PKG_VERSION")).await {
        Ok(code) => std::process::ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("sapphire-bridge: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}
