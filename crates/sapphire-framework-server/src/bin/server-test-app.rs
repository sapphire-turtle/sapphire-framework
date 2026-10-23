//! A minimal application used by the integration tests: its server and its CLI.
//!
//! Usage: `server-test-app <runtime-dir> <state-dir> [<command>...]`
//!
//! With no command it serves (`serve`): the framework's `workspace.*` namespace, and the
//! settings below through `config.*`. Any other command is the framework's CLI, run against
//! that server — or, for `config`, against the files when no server runs. `<state-dir>`
//! becomes the application's cache, data and config root, and `<runtime-dir>` the IPC
//! runtime directory, so the bridge it reaches is the one listening there.

use clap::Parser;
use sapphire_framework_server::{
    AppServer, AppSettings, Apply, FrameworkCommand, Kind, Scope, SettingDef, Settings,
};
use sapphire_ipc::Endpoint;
use sapphire_workspace::{AppContext, AppKind};

static CTX: AppContext = AppContext::new("sapphire-servertest");

/// The test application's own settings: one plain key and one secret.
#[derive(Clone)]
struct TestSettings;

impl AppSettings for TestSettings {
    fn defs(&self) -> Vec<SettingDef> {
        let def = |key: &str, kind: Kind, scopes: Vec<Scope>| SettingDef {
            key: key.into(),
            kind,
            scopes,
            overridable: false,
            apply: Apply::Live,
            default: None,
            section: "Test".into(),
            doc: String::new(),
            process: Default::default(),
        };
        vec![
            def(
                "sapphire-servertest.greeting",
                Kind::String { non_empty: true },
                vec![Scope::Device, Scope::Workspace],
            ),
            def(
                "sapphire-servertest.token",
                Kind::Secret,
                vec![Scope::Device],
            ),
        ]
    }
}

/// The command line after the two directories.
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: FrameworkCommand,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let runtime_dir = args.next().expect("a runtime directory");
    let state_dir = args.next().expect("a state directory");
    let rest: Vec<String> = args.collect();

    // SAFETY: set before the runtime does anything else with the environment.
    unsafe {
        std::env::set_var(sapphire_ipc::RUNTIME_DIR_ENV, &runtime_dir);
        std::env::set_var(
            "SAPPHIRE_SERVERTEST_CACHE_DIR",
            format!("{state_dir}/cache"),
        );
        std::env::set_var("SAPPHIRE_SERVERTEST_DATA_DIR", format!("{state_dir}/data"));
        std::env::set_var(
            "SAPPHIRE_SERVERTEST_CONFIG_DIR",
            format!("{state_dir}/config"),
        );
    }
    CTX.init(AppKind::Server);

    let command = if rest.is_empty() {
        FrameworkCommand::Serve
    } else {
        Cli::parse_from(std::iter::once("server-test-app".to_owned()).chain(rest)).command
    };
    let endpoint = Endpoint::in_dir("sapphire-servertest", runtime_dir.into());
    let build = move |_: &Settings| {
        AppServer::new(&CTX, env!("CARGO_PKG_VERSION")).endpoint(endpoint.clone())
    };
    let code = match command
        .dispatch(&CTX, TestSettings, build, env!("CARGO_PKG_VERSION"))
        .await
    {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err}");
            1
        }
    };
    std::process::exit(code);
}
