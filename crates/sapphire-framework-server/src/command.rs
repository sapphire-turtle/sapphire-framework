//! The framework-provided half of an application's CLI.

use sapphire_framework_service::{Environment, ServiceCommand, SystemManager};
use sapphire_ipc::{ClientInfo, Endpoint};

use crate::AppServer;
use crate::error::{Error, Result};

/// The framework's commands, flattened into an application's CLI (spec decision 2).
///
/// An application composes these beside its own verbs by giving its subcommand enum a
/// `#[command(flatten)] Framework(FrameworkCommand)` variant, so every verb — the app's and
/// the framework's — sits at the same top level. The parse tests below pin that shape.
#[derive(Debug, Default, clap::Subcommand)]
pub enum FrameworkCommand {
    /// Run the server in this process until SIGTERM or SIGINT (the bare invocation).
    #[default]
    Serve,
    /// Report whether a server is running, and which version.
    Status,
    /// Install, remove or report this application's operating-system service.
    #[command(subcommand)]
    Service(ServiceCommand),
}

impl FrameworkCommand {
    /// Carry out the command, returning the process exit code.
    ///
    /// The application matches its own variants first, then hands the framework's here;
    /// the app name and the service spec come from `server` itself.
    pub async fn dispatch(self, server: AppServer, version: &'static str) -> Result<i32> {
        match self {
            FrameworkCommand::Serve => {
                server.run().await?;
                Ok(0)
            }
            FrameworkCommand::Status => crate::command::status(&server, version).await,
            FrameworkCommand::Service(command) => {
                let spec = server.service_spec();
                command
                    .run(&spec, &Environment::detect(), &SystemManager)
                    .map_err(Error::from)
            }
        }
    }
}

/// Report whether a server is running, and which version.
///
/// Task 1 stub: probe and fail fast, without the start-on-demand machinery. Task 3
/// replaces it with the typed `StatusReport` over `connect_or_absent`.
async fn status(server: &AppServer, version: &str) -> Result<i32> {
    let endpoint = Endpoint::for_app(server.app_name())?;
    if !sapphire_ipc::probe(&endpoint).await? {
        println!("no {} server is running", server.app_name());
        return Ok(1);
    }
    let (_, info) = sapphire_ipc::Client::handshake(
        sapphire_ipc::connect(&endpoint).await?,
        server.app_name(),
        ClientInfo {
            kind: "cli".to_owned(),
            version: version.to_owned(),
            pid: std::process::id(),
        },
    )
    .await?;
    println!(
        "{} server running: version {}, pid {}, started as {:?}",
        server.app_name(),
        info.version,
        info.pid,
        info.managed_by
    );
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// The composition every app binary uses: an app-owned subcommand enum whose framework
    /// verbs ride in on a flattened variant (spec decision 2).
    ///
    /// clap 4 rejects `#[command(flatten)]` on a struct field beside a
    /// `#[command(subcommand)]` field — two subcommand enums cannot share one level, and a
    /// flattened field must implement `clap::Args`, which the `Subcommand` derive does not
    /// provide. The plan's Risks section names this probe as the decision, and the flat
    /// fallback is what it decided: one subcommand enum, every verb at the top level.
    #[derive(Parser)]
    struct Probe {
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Debug, clap::Subcommand)]
    enum Command {
        /// An app-specific verb that must parse beside the framework's.
        Greet,
        #[command(flatten)]
        Framework(FrameworkCommand),
    }

    #[test]
    fn app_and_framework_commands_parse_side_by_side() {
        let probe = Probe::try_parse_from(["app", "greet"]).unwrap();
        assert!(matches!(probe.command, Command::Greet));
        let probe = Probe::try_parse_from(["app", "serve"]).unwrap();
        assert!(matches!(
            probe.command,
            Command::Framework(FrameworkCommand::Serve)
        ));
        let probe = Probe::try_parse_from(["app", "status"]).unwrap();
        assert!(matches!(
            probe.command,
            Command::Framework(FrameworkCommand::Status)
        ));
    }

    #[test]
    fn the_service_subcommands_parse() {
        for args in [
            vec!["app", "service", "install"],
            vec!["app", "service", "install", "--system"],
            vec!["app", "service", "install", "--run-as", "alice"],
            vec!["app", "service", "uninstall"],
            vec!["app", "service", "status"],
        ] {
            assert!(Probe::try_parse_from(&args).is_ok(), "{args:?}");
        }
    }

    #[test]
    fn bare_invocation_defaults_to_serve() {
        // <app> with no subcommand at all means serve (spec decision 1). An app CLI
        // achieves it by `Option<Command>` and handing the framework's default here; this
        // probe pins the framework half.
        #[derive(Parser)]
        struct Bare {
            #[command(subcommand)]
            command: Option<Command>,
        }
        let bare = Bare::try_parse_from(["app"]).unwrap();
        assert!(bare.command.is_none());
        assert!(matches!(
            FrameworkCommand::default(),
            FrameworkCommand::Serve
        ));
    }
}

#[cfg(test)]
mod service_spec_tests {
    use super::*;
    use crate::privilege::{HelperSpec, PrivilegeConfig};
    use sapphire_framework_service::RunAs;
    use sapphire_workspace::AppContext;

    static CTX: AppContext = AppContext::new("sapphire-servicetest");

    /// The privilege configuration a privilege-separated application describes: a drop to
    /// the human user, plus a helper under another one.
    fn privileges_for(run_as: &str, helper: &str) -> PrivilegeConfig {
        PrivilegeConfig {
            run_as: run_as.parse().unwrap(),
            helper: Some(HelperSpec {
                user: helper.parse().unwrap(),
                program: std::path::PathBuf::from("/usr/lib/sapphire-agent/tool-broker"),
                args: vec![],
            }),
        }
    }

    #[test]
    fn the_generated_spec_runs_the_server_not_the_cli() {
        // A service manager starts the executable directly, and the executable's bare
        // invocation is `serve`: the unit's argv must be exactly that, so the server that
        // starts has no CLI verbs on its command line to misparse.
        let spec = AppServer::new(&CTX, "0.0.0").service_spec();
        assert_eq!(spec.args, vec!["serve".to_owned()]);
    }

    #[test]
    fn the_generated_spec_carries_the_apps_privileges() {
        let privileges = privileges_for("alice", "tools");
        let spec = AppServer::new(&CTX, "0.0.0")
            .privileges(privileges.clone())
            .service_spec();
        assert!(spec.privileges.is_some());
    }

    #[test]
    fn the_generated_spec_names_this_application() {
        let spec = AppServer::new(&CTX, "0.0.0").service_spec();
        assert_eq!(spec.app_name, CTX.app_name);
    }

    #[test]
    fn the_generated_spec_describes_the_server() {
        let spec = AppServer::new(&CTX, "0.0.0").service_spec();
        assert!(
            spec.description.contains(CTX.app_name),
            "the unit's description should name the application: {:?}",
            spec.description
        );
        assert!(spec.description.contains("0.0.0"));
    }

    #[test]
    fn the_generated_spec_runs_a_system_unit_as_the_invoking_user() {
        let spec = AppServer::new(&CTX, "0.0.0").service_spec();
        assert!(
            matches!(spec.system_run_as, RunAs::InvokingUser),
            "a server started as root would create its cache, its data and its sockets \
             under /root"
        );
    }
}
