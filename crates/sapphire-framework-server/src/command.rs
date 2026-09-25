//! The framework-provided half of an application's CLI.

use std::fmt::Write as _;

use sapphire_framework_service::{Environment, ServiceCommand, SystemManager};
use sapphire_ipc::{ClientInfo, Endpoint, ManagedBy};
use serde::{Deserialize, Serialize};

use crate::AppServer;
use crate::error::{Error, Result};

/// The typed answer to a status question, shared by the CLI and the IPC `server.info`
/// response (spec decision 4).
///
/// When a server answers, the CLI prints the framework's fields and then the
/// application's [rows](StatusReport::app) as `name: value` lines; a GUI could read the
/// same serialised shape from the IPC method instead. When nothing is listening, the
/// report is [`StatusReport::running`] = `false` and the app rows are skipped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusReport {
    /// Whether a server is answering at all.
    pub running: bool,
    /// The server's version, when it is running.
    pub version: Option<String>,
    /// Its pid, when it is running.
    pub pid: Option<u32>,
    /// How the running server was started, when it is running.
    pub managed_by: Option<ManagedBy>,
    /// The application's own rows, rendered after the framework's.
    pub app: Vec<StatusRow>,
}

/// One application-provided line of the status report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusRow {
    /// The row's name, e.g. `sync`.
    pub name: String,
    /// The value shown beside it.
    pub value: String,
}

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

/// The status command's face: render and return the exit code, going to the process's
/// standard output.
async fn status(server: &AppServer, version: &str) -> Result<i32> {
    let endpoint = Endpoint::for_app(server.app_name())?;
    run_status(&endpoint, server.app_name(), version).await
}

/// Probe, handshake and call `SERVER_INFO` in one go, or report absence.
///
/// Every step the CLI needs — liveness and the version gate — is inside
/// [`connect_or_absent`], so the command is that call plus one `SERVER_INFO` round trip.
/// The version gate doubles as the liveness check here: a live server of another version
/// is `ServiceVersionMismatch`, which names both versions and advises restarting the
/// service. The report goes through [`run_status_into`] so tests can capture it.
async fn run_status(endpoint: &Endpoint, app: &str, version: &str) -> Result<i32> {
    let mut out = String::new();
    let code = run_status_into(endpoint, app, version, &mut out).await?;
    for line in out.lines() {
        println!("{line}");
    }
    Ok(code)
}

/// [`run_status`], writing the rendered lines into `out` instead of the process's
/// standard output.
async fn run_status_into(
    endpoint: &Endpoint,
    app: &str,
    version: &str,
    out: &mut String,
) -> Result<i32> {
    let client_info = ClientInfo {
        kind: "cli".to_owned(),
        version: version.to_owned(),
        pid: std::process::id(),
    };
    let Some((client, _)) = sapphire_ipc::connect_or_absent(endpoint, app, client_info).await?
    else {
        // Nothing is listening: the framework's half is only the one line, and the
        // application's rows are skipped (there is no server to have configured them).
        writeln!(out, "no {app} server is running").expect("writing to a String cannot fail");
        return Ok(1);
    };
    let report: StatusReport = client
        .call(
            sapphire_backend::protocol::SERVER_INFO,
            serde_json::json!({}),
        )
        .await?;
    writeln!(out, "running: {}", report.running).expect("writing to a String cannot fail");
    if let Some(version) = &report.version {
        writeln!(out, "version: {version}").expect("writing to a String cannot fail");
    }
    if let Some(pid) = report.pid {
        writeln!(out, "pid: {pid}").expect("writing to a String cannot fail");
    }
    if let Some(managed_by) = &report.managed_by {
        writeln!(out, "managed_by: {managed_by:?}").expect("writing to a String cannot fail");
    }
    for row in &report.app {
        writeln!(out, "{}: {}", row.name, row.value).expect("writing to a String cannot fail");
    }
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
mod status_tests {
    use super::*;
    use sapphire_workspace::AppContext;

    static CTX: AppContext = AppContext::new("sapphire-statustest");

    /// Wait until something is listening on `endpoint`.
    ///
    /// `tokio::spawn(server.run())` only schedules the server; without this wait a fast
    /// `run_status` probe can run before `run` has bound the socket, and with nothing
    /// listening that single unlucky probe is the whole test failing.
    async fn wait_until_listening(endpoint: &Endpoint) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while !sapphire_ipc::probe(endpoint).await.unwrap() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the server never started listening"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn status_reports_no_server_when_none_is_running() {
        let tmp = tempfile::tempdir().unwrap();
        let endpoint = sapphire_ipc::Endpoint::in_dir("status-test", tmp.path().to_path_buf());
        let code = run_status(&endpoint, "status-test", "0.0.0").await.unwrap();
        assert_eq!(code, 1, "no server is a non-zero exit");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn status_shows_the_extension_rows_of_a_running_server() {
        let tmp = tempfile::tempdir().unwrap();
        let endpoint = sapphire_ipc::Endpoint::in_dir("status-test", tmp.path().to_path_buf());
        let server = AppServer::new(&CTX, "0.0.0")
            .endpoint(endpoint.clone())
            .status_rows(std::sync::Arc::new(|| {
                vec![StatusRow {
                    name: "sync".into(),
                    value: "on".into(),
                }]
            }));
        let handle = tokio::spawn(async move { server.run().await });
        wait_until_listening(&endpoint).await;

        // The report goes through the sink face so the rendering is asserted, not just
        // the exit code.
        let mut out = String::new();
        let code = run_status_into(&endpoint, CTX.app_name, "0.0.0", &mut out)
            .await
            .unwrap();
        assert_eq!(code, 0);
        assert!(out.contains("running: true"), "output was: {out}");
        assert!(out.contains("sync: on"), "output was: {out}");

        let (client, _) = sapphire_ipc::connect_or_absent(
            &endpoint,
            CTX.app_name,
            sapphire_ipc::ClientInfo {
                kind: "test".into(),
                version: "0.0.0".into(),
                pid: std::process::id(),
            },
        )
        .await
        .unwrap()
        .expect("the server is listening");
        let _: serde_json::Value = client
            .call(sapphire_ipc::SHUTDOWN_METHOD, serde_json::json!({}))
            .await
            .unwrap();
        handle.await.unwrap().unwrap();
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
