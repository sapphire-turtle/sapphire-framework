//! The host-wide sapphire daemon.
//!
//! Running it with no subcommand starts the bridge; every other subcommand is a one-shot
//! command against a running one.

use clap::Parser;
use sapphire_bridge::BridgeCommand;

#[derive(Parser)]
#[command(name = "sapphire-bridge", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<BridgeCommand>,
}

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
