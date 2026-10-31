//! The framework-provided half of an application's CLI.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use sapphire_backend::protocol as proto;
use sapphire_bridge_api::{
    DevicePrioritySetParams, DeviceRetireParams, InviteParams, JoinParams, WorkgroupCreateParams,
};
use sapphire_bridge_client::BridgeClient;
use sapphire_framework_service::{Environment, ServiceCommand, SystemManager};
use sapphire_ipc::{ClientInfo, Endpoint};

use crate::AppServer;
use crate::error::{Error, Result};
use crate::selection::SelectionFile;
use crate::settings::{AppSettings, Settings};
use sapphire_workspace::AppContext;

mod config;

pub use config::{AppConfigCommand, AppScopeFlags};

pub use sapphire_backend::protocol::{StatusReport, StatusRow};

/// The framework's commands, flattened into an application's CLI (spec decision 2).
///
/// An application composes these beside its own verbs by giving its subcommand enum a
/// `#[command(flatten)] Framework(FrameworkCommand)` variant, so every verb — the app's and
/// the framework's — sits at the same top level. The parse tests below pin that shape.
///
/// A desktop app's binary carries the same enum (#366): it is the CLI as well, so the CLI
/// version is a subset of it. Its subcommand is optional, and with none it opens the
/// window; the CLI version's is required, so a bare invocation prints the help. The verbs
/// are one definition either way:
///
/// ```rust,ignore
/// #[derive(clap::Parser)]
/// struct Desktop {
///     #[command(subcommand)]
///     command: Option<Command>, // `Command` for the CLI version
/// }
///
/// match Desktop::parse().command {
///     None => open_the_window(),
///     Some(command) => runtime.block_on(run(command)),
/// }
/// ```
#[derive(Debug, clap::Subcommand)]
pub enum FrameworkCommand {
    /// Run the server in this process until SIGTERM or SIGINT.
    Serve,
    /// Report whether a server is running, and which version.
    Status,
    /// Install, remove or report this application's operating-system service.
    #[command(subcommand)]
    Service(ServiceCommand),
    /// Replace this executable with the newest release, then do the same for
    /// `sapphire-bridge` (unless `--no-bridge`). Restart the services afterwards to run
    /// them.
    Update(AppUpdateArgs),
    /// Create, select, show and map this application's workspace. The server serves one
    /// at a time.
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    /// Found, show or join the workgroup this device belongs to.
    #[command(subcommand)]
    Workgroup(WorkgroupCommand),
    /// List the workgroup's devices, invite one or retire one.
    #[command(subcommand)]
    Device(DeviceCommand),
    /// Show or change the embedding models (the workgroup's local and remote one), this
    /// device's switches, and its API key. Needs the running bridge.
    #[command(subcommand)]
    Embedding(sapphire_bridge_api::EmbeddingCommand),
    /// Add, list, retire, restore and rotate the API keys of the workspace this application
    /// serves — for clients that reach it over HTTP. Works on the workspace's files, with or
    /// without a running server; the keys sync with the workspace.
    #[command(subcommand)]
    ApiKey(corundum_framework_keys::KeyCommand),
    /// Renamed to `api-key` (#364).
    #[command(hide = true)]
    ExternalDevice(LegacyExternalDevice),
    /// List, show, set and unset this application's settings and the bridge's, by key.
    /// Without a running server, this application's own, on its files.
    #[command(subcommand)]
    Config(AppConfigCommand),
    /// Serve this application's tools to an AI agent over MCP on stdin/stdout. Talks to the
    /// running server, which it does not start; register this command with the agent.
    #[cfg(feature = "mcp")]
    Mcp,
}

impl FrameworkCommand {
    /// Carry out the command, returning the process exit code.
    ///
    /// The application matches its own variants first, then hands the framework's here,
    /// with what [`serve_restarting`](crate::serve_restarting) takes: its context, its own
    /// settings (`()` for none) and the function that builds its server from the settings.
    /// `serve` runs [`serve_restarting`](crate::serve_restarting); the commands that need an
    /// [`AppServer`] — `service`, `update` and `mcp` — read the settings and call `build`
    /// once; `config` reads and writes the settings through the running server, or the
    /// files themselves when none runs.
    ///
    /// ```rust,ignore
    /// command
    ///     .dispatch(&CTX, MyOwner, |settings: &Settings| AppServer::new(&CTX, VERSION), VERSION)
    ///     .await
    /// ```
    pub async fn dispatch<A, F>(
        self,
        ctx: &'static AppContext,
        owner: A,
        build: F,
        version: &'static str,
    ) -> Result<i32>
    where
        A: AppSettings + Clone,
        F: Fn(&Settings) -> AppServer,
    {
        let app = ctx.app_name;
        // Built only by the commands that need a server, and then once. These are the way
        // out of a bad release or a broken install, so settings that cannot be read are
        // warned about and replaced by the defaults rather than stopping them.
        let server = |owner: A| -> Result<AppServer> {
            Ok(build(&settings_or_defaults(ctx, Arc::new(owner))?))
        };
        match self {
            FrameworkCommand::Serve => {
                crate::serve_restarting(ctx, owner, build).await?;
                Ok(0)
            }
            FrameworkCommand::Status => {
                let endpoint = Endpoint::for_app(app)?;
                run_status(&endpoint, app, version).await
            }
            FrameworkCommand::Service(command) => {
                let spec = server(owner)?.service_spec();
                command
                    .run(&spec, &Environment::detect(), &SystemManager)
                    .map_err(Error::from)
            }
            FrameworkCommand::Update(args) => {
                let server = server(owner)?;
                let own = match server.release_source() {
                    Some(source) => {
                        sapphire_framework_update::run(&args.update, source, app, version)
                            .await
                            .map_err(Error::from)
                    }
                    None => {
                        println!("{app} does not say where its releases are");
                        Ok(1)
                    }
                };
                // The bridge is updated whatever became of the app: one is independent of
                // the other, and an app installed with cargo still has a released bridge.
                let bridge = if args.no_bridge {
                    0
                } else {
                    update_bridge(&args.update).await?
                };
                Ok(own?.max(bridge))
            }
            FrameworkCommand::Config(command) => config::config(ctx, owner, command, version).await,
            FrameworkCommand::Workspace(command) => command.dispatch(app, version).await,
            FrameworkCommand::Workgroup(command) => command.dispatch(app, version).await,
            FrameworkCommand::Device(command) => command.dispatch(app, version).await,
            FrameworkCommand::Embedding(command) => embedding(command, app, version).await,
            FrameworkCommand::ApiKey(command) => api_key(ctx, command),
            FrameworkCommand::ExternalDevice(legacy) => {
                eprintln!("warning: `external-device` is renamed to `api-key`");
                if !legacy.app.is_empty() {
                    eprintln!(
                        "API keys are per workspace now, not per application: `--app` is not taken"
                    );
                    return Ok(2);
                }
                api_key(ctx, legacy.command)
            }
            #[cfg(feature = "mcp")]
            FrameworkCommand::Mcp => {
                server(owner)?.serve_mcp_stdio(version).await?;
                Ok(0)
            }
        }
    }
}

/// `ctx`'s application's settings, or — when they cannot be read — every key at its
/// default, after one warning on stderr naming the device file and the error.
///
/// For `update`, `service` and `mcp`, which must work whatever the files hold: `update` is
/// the way out of a bad release, and `service uninstall` must always be possible.
fn settings_or_defaults(ctx: &AppContext, owner: Arc<dyn AppSettings>) -> Result<Settings> {
    match Settings::load(ctx, Arc::clone(&owner)) {
        Ok(settings) => Ok(settings),
        Err(err) => {
            let file = Settings::device_file(ctx)
                .map_or_else(|| "the settings".to_owned(), |f| f.display().to_string());
            eprintln!("warning: could not read {file} ({err}); going on with the default settings");
            Settings::defaults(ctx, owner)
        }
    }
}

/// `update`'s arguments in an application: its own, and whether the bridge follows.
#[derive(Clone, Debug, Default, clap::Args)]
pub struct AppUpdateArgs {
    /// `--check` and `--yes`, for the application and the bridge alike.
    #[command(flatten)]
    pub update: sapphire_framework_update::UpdateArgs,
    /// Update this application alone, leaving `sapphire-bridge` as it is.
    #[arg(long)]
    pub no_bridge: bool,
}

/// The bridge's executable, as it is installed beside the apps.
const BRIDGE_BINARY: &str = "sapphire-bridge";

/// Where the bridge's releases are, for a bridge too old to update itself.
const BRIDGE_RELEASES: &str = "https://github.com/sapphire-turtle/sapphire-framework/releases";

/// The first `name` (with the platform's executable suffix) in the directories of `path`.
fn find_on_path(name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(path?)
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
}

/// Run the bridge on `PATH`'s own `update` with `args`; returns its exit code. No bridge on
/// `PATH` is nothing to update.
async fn update_bridge(args: &sapphire_framework_update::UpdateArgs) -> Result<i32> {
    match find_on_path(BRIDGE_BINARY, std::env::var_os("PATH").as_deref()) {
        Some(exe) => run_bridge_update(&exe, args).await,
        None => {
            println!("{BRIDGE_BINARY} is not on PATH; not updating it");
            Ok(0)
        }
    }
}

/// Run the bridge at `exe`'s own `update` with `args`; returns its exit code.
///
/// The bridge updates itself rather than being replaced from here: it knows its own
/// version and where it was installed from, and replacing a running executable is its own
/// business on every platform.
async fn run_bridge_update(
    exe: &std::path::Path,
    args: &sapphire_framework_update::UpdateArgs,
) -> Result<i32> {
    // A bridge from before `update` rejects the subcommand.
    let knows_update = tokio::process::Command::new(exe)
        .args(["update", "--help"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await?
        .success();
    if !knows_update {
        println!(
            "{} cannot update itself; install the newest from {BRIDGE_RELEASES}",
            exe.display()
        );
        return Ok(1);
    }
    let mut command = tokio::process::Command::new(exe);
    command.arg("update");
    if args.check {
        command.arg("--check");
    }
    if args.yes {
        command.arg("--yes");
    }
    // Inherited stdio: the bridge asks its own question and says what it did.
    let status = command.status().await?;
    Ok(status.code().unwrap_or(1))
}

/// The `workspace` subcommands (#215: one workspace per server).
///
/// `init`, `select`, `sync`, `move` and `remove` go to the app's server over IPC,
/// because the server owns the marker directories, the sync ids and which workspace it
/// serves; `show` asks the server for its workspace and then the bridge for the workgroup's
/// ledger. `init --remote-workspace` has the server resolve the name through the bridge.
#[derive(Debug, clap::Subcommand)]
pub enum WorkspaceCommand {
    /// Create this app's workspace home in the given directory, and switch the server to it.
    Init {
        /// Where the workspace root goes. Defaults to the current directory.
        dir: Option<PathBuf>,
        /// Also start syncing it, which publishes it into the workgroup ledger.
        #[arg(long)]
        sync: bool,
        /// Sync it as this workspace of the workgroup, by name or id, instead of as a new
        /// one: its files and settings arrive from the other devices.
        #[arg(long, value_name = "NAME|ID", requires = "sync")]
        remote_workspace: Option<String>,
    },
    /// Switch the server to an existing workspace of this app.
    Select {
        /// The workspace's root. Defaults to the current directory.
        dir: Option<PathBuf>,
        /// Also start syncing it, which publishes it into the workgroup ledger. A
        /// workspace that has synced before turns sync back on by itself.
        #[arg(long)]
        sync: bool,
    },
    /// Turn sync on or off for the server's workspace, or show its sync state.
    #[command(subcommand)]
    Sync(WorkspaceSyncCommand),
    /// Show the server's workspace, then the workgroup's workspaces of this app.
    Show,
    /// Move the server's workspace folder to a new path on this host, keeping its sync.
    Move {
        /// The new root. It must not exist yet; its parent must.
        dir: PathBuf,
    },
    /// Stop serving the server's workspace on this host. Other devices keep their copies.
    Remove {
        /// Also delete the workspace's folder and every file in it.
        #[arg(long)]
        delete: bool,
    },
}

impl WorkspaceCommand {
    /// Carry out the command, returning the process exit code.
    ///
    /// `app` names the endpoint to open (`Endpoint::for_app`); `version` is this
    /// process's own, for the handshake.
    pub async fn dispatch(self, app: &'static str, version: &'static str) -> Result<i32> {
        match self {
            WorkspaceCommand::Init {
                dir,
                sync,
                remote_workspace,
            } => workspace_init(app, version, dir, sync, remote_workspace).await,
            WorkspaceCommand::Select { dir, sync } => {
                workspace_select(app, version, dir, sync).await
            }
            WorkspaceCommand::Sync(command) => workspace_sync(app, version, command).await,
            WorkspaceCommand::Show => workspace_show(app, version).await,
            WorkspaceCommand::Move { dir } => workspace_move(app, version, dir).await,
            WorkspaceCommand::Remove { delete } => workspace_remove(app, version, delete).await,
        }
    }
}

/// The `workspace sync` subcommands: the server's workspace's sync, switched after the fact.
#[derive(Debug, clap::Subcommand)]
pub enum WorkspaceSyncCommand {
    /// Start syncing the server's workspace, which publishes it into the workgroup ledger.
    Enable,
    /// Stop syncing the server's workspace on this host. It keeps its sync id, so enabling
    /// it again picks up where it left off; other devices keep their copies.
    Disable,
    /// Show the server's workspace's sync state.
    Status,
}

/// The `workgroup` subcommands.
///
/// The workgroup's ledger is the bridge's business end to end, so these go to the bridge's
/// endpoint directly, including `create`, which goes through the running bridge.
#[derive(Debug, clap::Subcommand)]
pub enum WorkgroupCommand {
    /// Found a workgroup on this host.
    Create {
        /// The workgroup's name.
        name: String,
        /// This host's device name inside it.
        #[arg(long)]
        device_name: String,
    },
    /// Show the workgroup this host belongs to.
    List,
    /// Join the workgroup a ticket names.
    Join {
        /// The ticket the inviting device printed.
        ticket: String,
        /// The name this device will carry. Defaults to this host's name.
        #[arg(long)]
        device_name: Option<String>,
    },
}

impl WorkgroupCommand {
    /// Carry out the command, returning the process exit code.
    pub async fn dispatch(self, app: &'static str, version: &'static str) -> Result<i32> {
        match self {
            WorkgroupCommand::Create { name, device_name } => {
                let Some(client) = relayed_bridge(app, version).await? else {
                    return Ok(1);
                };
                let created = client
                    .workgroup_create(WorkgroupCreateParams { name, device_name })
                    .await?;
                println!(
                    "created workgroup {} ({}); this device is {}",
                    created.name, created.workgroup_id, created.device_id
                );
                Ok(0)
            }
            WorkgroupCommand::List => {
                let Some(client) = relayed_bridge(app, version).await? else {
                    return Ok(1);
                };
                match client.status().await?.workgroup {
                    Some(workgroup) => {
                        println!("{} ({})", workgroup.name, workgroup.workgroup_id);
                        Ok(0)
                    }
                    None => {
                        println!("this host has not joined a workgroup");
                        Ok(1)
                    }
                }
            }
            WorkgroupCommand::Join {
                ticket,
                device_name,
            } => {
                let Some(client) = relayed_bridge(app, version).await? else {
                    return Ok(1);
                };
                let joined = client
                    .join(JoinParams {
                        ticket,
                        device_name,
                    })
                    .await?;
                println!(
                    "joined workgroup {} ({}); this device is {}",
                    joined.workgroup_name, joined.workgroup_id, joined.device_id
                );
                Ok(0)
            }
        }
    }
}

/// The `device` subcommands.
///
/// The ledger's word for taking a device out is `retire` — a device id is written into
/// synced content and must keep resolving, so its record stays as a tombstone — and this
/// command goes through the running bridge to retire a device.
#[derive(Debug, clap::Subcommand)]
pub enum DeviceCommand {
    /// List the workgroup's devices, and which are reachable.
    List,
    /// Create an invite ticket for a device that is about to join.
    Invite {
        /// What the joining device will be called.
        #[arg(long)]
        name: String,
        /// How long the invite stays good, in seconds.
        #[arg(long)]
        ttl: Option<u64>,
        /// The workgroup to invite into, by name or id.
        #[arg(long)]
        workgroup: Option<String>,
    },
    /// Retire a device, so it may no longer connect.
    Retire {
        /// The device's name or id.
        selector: String,
    },
    /// Show or set a device's election priority (0-255; 0 = never primary or secondary).
    /// The election is non-preemptive: raising a priority does not move a role already held.
    Priority {
        /// The device's name or id.
        selector: String,
        /// The new priority. Omit to show the current one.
        priority: Option<u8>,
    },
}

impl DeviceCommand {
    /// Carry out the command, returning the process exit code.
    pub async fn dispatch(self, app: &'static str, version: &'static str) -> Result<i32> {
        match self {
            DeviceCommand::List => {
                let Some(client) = relayed_bridge(app, version).await? else {
                    return Ok(1);
                };
                let peers = client.peers().await?;
                if peers.peers.is_empty() {
                    println!("no devices");
                    return Ok(0);
                }
                for peer in peers.peers {
                    println!(
                        "{} {} p{}{}{}",
                        peer.name,
                        peer.device_id,
                        peer.priority,
                        peer.availability
                            .map(|a| format!(" a{a}"))
                            .unwrap_or_default(),
                        if peer.connected { " (online)" } else { "" }
                    );
                }
                Ok(0)
            }
            DeviceCommand::Invite {
                name,
                ttl,
                workgroup,
            } => {
                let Some(client) = relayed_bridge(app, version).await? else {
                    return Ok(1);
                };
                let invite = client
                    .invite(InviteParams {
                        name,
                        ttl,
                        workgroup,
                    })
                    .await?;
                // The ticket on its own line, and nothing else on it: this is the one
                // output a user pipes to the joining device.
                println!("{}", invite.ticket);
                Ok(0)
            }
            DeviceCommand::Retire { selector } => {
                let Some(client) = relayed_bridge(app, version).await? else {
                    return Ok(1);
                };
                let retired = client
                    .device_retire(DeviceRetireParams { selector })
                    .await?;
                println!("retired device {} ({})", retired.name, retired.device_id);
                Ok(0)
            }
            DeviceCommand::Priority { selector, priority } => {
                let Some(client) = relayed_bridge(app, version).await? else {
                    return Ok(1);
                };
                match priority {
                    Some(priority) => {
                        let set = client
                            .device_priority_set(DevicePrioritySetParams { selector, priority })
                            .await?;
                        println!("{} ({}) priority {}", set.name, set.device_id, set.priority);
                    }
                    None => {
                        let peers = client.peers().await?;
                        match peers
                            .peers
                            .iter()
                            .find(|p| p.name == selector || p.device_id.to_string() == selector)
                        {
                            Some(p) => {
                                println!("{} ({}) priority {}", p.name, p.device_id, p.priority)
                            }
                            None => {
                                println!("no device {selector:?}");
                                return Ok(1);
                            }
                        }
                    }
                }
                Ok(0)
            }
        }
    }
}

/// The bridge, through `app`'s server, or `None` after saying no server is running.
///
/// The app's server relays the bridge's client-facing methods, so the CLI holds one
/// connection, to the app, whichever bridge answers behind it. The bridge being absent
/// surfaces on the first call as the error whose text the `main` prints: "no
/// sapphire-bridge is running".
async fn relayed_bridge(app: &str, version: &str) -> Result<Option<BridgeClient>> {
    let Some(client) = connect_app(app, version).await? else {
        return Ok(None);
    };
    Ok(Some(BridgeClient::relayed(
        Arc::new(client),
        sapphire_ipc::runtime_dir()?,
    )))
}

/// `embedding …`: always through the app's server, to the bridge whose files these are.
async fn embedding(
    command: sapphire_bridge_api::EmbeddingCommand,
    app: &str,
    version: &str,
) -> Result<i32> {
    // Connect first: `key set` asks for the key, which is pointless with no server to take it.
    let Some(client) = relayed_bridge(app, version).await? else {
        return Ok(1);
    };
    let request = command.request()?;
    let report = client.embed_request(request).await?;
    for line in sapphire_bridge_api::describe(&report) {
        println!("{line}");
    }
    Ok(0)
}

/// `external-device …`, kept one release as a hidden alias of `api-key`.
#[derive(Debug, clap::Args)]
pub struct LegacyExternalDevice {
    /// What `external-device add --app` took; refused, as keys are per workspace now.
    #[arg(long = "app", global = true, hide = true)]
    app: Vec<String>,
    #[command(subcommand)]
    command: corundum_framework_keys::KeyCommand,
}

/// `api-key …`: on the files of the workspace this application serves, as its selection
/// file records it. No workspace selected → a line naming `workspace select`, exit 1.
fn api_key(ctx: &AppContext, command: corundum_framework_keys::KeyCommand) -> Result<i32> {
    let Some(selection) = SelectionFile::for_app(ctx).load(ctx.app_name)? else {
        eprintln!(
            "{} serves no workspace yet: choose one with `{} workspace select <dir>`",
            ctx.app_name, ctx.app_name
        );
        return Ok(1);
    };
    let run = sapphire_framework_keys::open_store(ctx.app_name, &selection.root)
        .and_then(|mut store| command.run(&mut store));
    match run {
        Ok(code) => Ok(code),
        Err(err) => {
            eprintln!("{err}");
            Ok(1)
        }
    }
}

/// `workspace init`: the server does the creating, over IPC.
///
/// The server owns the marker, the registry and the sync ids, so a workspace created
/// here is one the server already knows and a GUI sharing the registry sees. Nothing is
/// listening → the one line and exit 1; nothing is ever started (Global Constraints).
/// `--sync` rides the server's existing `sync.enable` path on the same connection, and
/// its state is printed after, so an unjoined host's "no workgroup knows it yet" is
/// plain.
///
/// `--remote-workspace` goes to `sync.map` instead, which makes the marker and takes the
/// workgroup's identity in one step: the workspace's `config.toml` is the one that
/// arrives, not one written here first (#253).
async fn workspace_init(
    app: &'static str,
    version: &'static str,
    dir: Option<PathBuf>,
    sync: bool,
    remote_workspace: Option<String>,
) -> Result<i32> {
    let endpoint = Endpoint::for_app(app)?;
    let client_info = ClientInfo {
        kind: "cli".to_owned(),
        version: version.to_owned(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    let Some((client, _)) = sapphire_ipc::connect_or_absent(&endpoint, app, client_info).await?
    else {
        println!("no {app} server is running");
        return Ok(1);
    };

    // Resolved here: the server's cwd is not the user's.
    let dir = std::env::current_dir()?.join(dir.unwrap_or_else(|| PathBuf::from(".")));
    if let Some(selector) = remote_workspace {
        let mapped: proto::SyncEnableResult = client
            .call(
                proto::SYNC_MAP,
                proto::SyncMapParams {
                    workspace: selector.clone(),
                    dir: dir.clone(),
                },
            )
            .await?;
        println!(
            "initialized {app} workspace in {} from {selector}",
            dir.display()
        );
        println!(
            "the {app} server now serves it, syncing as {}",
            mapped.workspace_id
        );
        return Ok(0);
    }
    let result: proto::WorkspaceInitResult = client
        .call(proto::WORKSPACE_INIT, proto::WorkspaceInitParams { dir })
        .await?;
    if result.created {
        println!("initialized {app} workspace in {}", result.root.display());
    } else {
        println!(
            "{} already exists (workspace {})",
            result.root.display(),
            result.workspace_id
        );
    }
    println!("the {app} server now serves {}", result.root.display());

    if sync {
        enable_sync(&client).await?;
    }
    Ok(0)
}

/// Turn sync on for the server's workspace over `client`, and print what it syncs as.
///
/// The one path behind `init --sync`, `select --sync` and `sync enable`: the server's
/// `sync.enable`, then its status, printed after, so an unjoined host's "no workgroup knows
/// it yet" is plain.
async fn enable_sync(client: &sapphire_ipc::Client) -> Result<()> {
    let enabled: proto::SyncEnableResult = client
        .call(proto::SYNC_ENABLE, serde_json::json!({}))
        .await?;
    // Never fails on account of the bridge: the workspace is synced either way, and the
    // status is the user's word for what the workgroup does not know yet.
    let status: proto::SyncStatusResult = client
        .call(proto::SYNC_STATUS, serde_json::json!({}))
        .await
        .unwrap_or(proto::SyncStatusResult {
            enabled: true,
            workspace_id: Some(enabled.workspace_id),
            ..proto::SyncStatusResult::not_synced()
        });
    println!(
        "syncing as {} ({} peer{})",
        enabled.workspace_id,
        status.peers,
        if status.peers == 1 { "" } else { "s" }
    );
    Ok(())
}

/// `workspace sync …`: the server's workspace's sync, switched after the fact.
async fn workspace_sync(
    app: &'static str,
    version: &'static str,
    command: WorkspaceSyncCommand,
) -> Result<i32> {
    let Some(client) = connect_app(app, version).await? else {
        return Ok(1);
    };
    match command {
        WorkspaceSyncCommand::Enable => enable_sync(&client).await?,
        WorkspaceSyncCommand::Disable => {
            let _: proto::Ack = client
                .call(proto::SYNC_DISABLE, serde_json::json!({}))
                .await?;
            println!("sync is off for the {app} workspace on this host; it keeps its sync id");
        }
        WorkspaceSyncCommand::Status => {
            let status: proto::SyncStatusResult = client
                .call(proto::SYNC_STATUS, serde_json::json!({}))
                .await?;
            for line in describe_sync(&status) {
                println!("{line}");
            }
        }
    }
    Ok(0)
}

/// `workspace sync status`, line by line.
fn describe_sync(status: &proto::SyncStatusResult) -> Vec<String> {
    let mut lines = vec![format!(
        "sync: {}",
        if status.enabled { "on" } else { "off" }
    )];
    if let Some(id) = status.workspace_id {
        lines.push(format!("workspace id: {id}"));
    }
    if status.enabled {
        lines.push(format!(
            "peers: {}{}",
            status.peers,
            if matches!(status.topology, proto::Topology::Star { .. }) {
                " (star)"
            } else {
                ""
            }
        ));
        lines.push(format!(
            "bridge: {}",
            if status.bridge_available {
                "reachable"
            } else {
                "not reachable"
            }
        ));
    }
    if let Some(paused) = &status.paused {
        lines.push(format!("paused: {paused}"));
    }
    if let Some(error) = &status.last_error {
        lines.push(format!("last error: {error}"));
    }
    lines
}

/// Render the server's workspace as the CLI prints it: `root state`.
///
/// Public so the integration tests can capture it; `workspace show` writes it to standard
/// output.
pub async fn render_workspace(client: &sapphire_ipc::Client, out: &mut String) -> Result<i32> {
    let current: proto::WorkspaceCurrentResult = client
        .call(proto::WORKSPACE_CURRENT, serde_json::json!({}))
        .await?;
    let Some(ws) = current.workspace else {
        writeln!(
            out,
            "no workspace yet; run `workspace init <dir>` or `workspace select <dir>`"
        )
        .expect("writing to a String cannot fail");
        return Ok(1);
    };
    writeln!(out, "{} {}", ws.root.display(), describe_state(&ws))
        .expect("writing to a String cannot fail");
    Ok(0)
}

/// `synced (star), embedding 3 pending` and the like.
fn describe_state(ws: &proto::CurrentWorkspace) -> String {
    let state = if !ws.reachable {
        "unreachable"
    } else if ws.sync.enabled {
        "synced"
    } else {
        "not synced"
    };
    let star = if matches!(ws.sync.topology, proto::Topology::Star { .. }) {
        " (star)"
    } else {
        ""
    };
    let pending = match &ws.sync.embedding {
        Some(e) if e.pending > 0 => format!(", embedding {} pending", e.pending),
        _ => String::new(),
    };
    format!("{state}{star}{pending}")
}

/// Connect to `app`'s server, or say none is running.
async fn connect_app(app: &str, version: &str) -> Result<Option<sapphire_ipc::Client>> {
    let client = try_connect_app(app, version).await?;
    if client.is_none() {
        println!("no {app} server is running");
    }
    Ok(client)
}

/// Connect to `app`'s server; `None`, silently, when none is running.
async fn try_connect_app(app: &str, version: &str) -> Result<Option<sapphire_ipc::Client>> {
    let endpoint = Endpoint::for_app(app)?;
    let client_info = ClientInfo {
        kind: "cli".to_owned(),
        version: version.to_owned(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    let client = sapphire_ipc::connect_or_absent(&endpoint, app, client_info).await?;
    Ok(client.map(|(c, _)| c))
}

/// `workspace select`: switch the server to an existing workspace, syncing it with `--sync`.
async fn workspace_select(
    app: &'static str,
    version: &'static str,
    dir: Option<PathBuf>,
    sync: bool,
) -> Result<i32> {
    let Some(client) = connect_app(app, version).await? else {
        return Ok(1);
    };
    // Resolved here: the server's cwd is not the user's.
    let dir = std::env::current_dir()?.join(dir.unwrap_or_else(|| PathBuf::from(".")));
    let ws: proto::CurrentWorkspace = client
        .call(
            proto::WORKSPACE_SELECT,
            proto::WorkspaceSelectParams { dir },
        )
        .await?;
    println!("{} {}", ws.root.display(), describe_state(&ws));
    // A workspace that has synced before is synced again by the select itself.
    if sync && !ws.sync.enabled {
        enable_sync(&client).await?;
    }
    Ok(0)
}

/// `workspace move`: move the server's workspace folder.
async fn workspace_move(app: &'static str, version: &'static str, dir: PathBuf) -> Result<i32> {
    let Some(client) = connect_app(app, version).await? else {
        return Ok(1);
    };
    // Resolved here: the server's cwd is not the user's.
    let dir = std::env::current_dir()?.join(dir);
    let ws: proto::CurrentWorkspace = client
        .call(proto::WORKSPACE_MOVE, proto::WorkspaceMoveParams { dir })
        .await?;
    println!("{} {}", ws.root.display(), describe_state(&ws));
    Ok(0)
}

/// `workspace remove`: let go of the server's workspace, deleting it with `--delete`.
async fn workspace_remove(app: &'static str, version: &'static str, delete: bool) -> Result<i32> {
    let Some(client) = connect_app(app, version).await? else {
        return Ok(1);
    };
    let _: proto::Ack = client
        .call(
            proto::WORKSPACE_REMOVE,
            proto::WorkspaceRemoveParams { delete },
        )
        .await?;
    println!(
        "{}",
        if delete {
            "removed and deleted"
        } else {
            "removed; the files stay"
        }
    );
    Ok(0)
}

/// `workspace show`: the server's workspace, then the workgroup's workspaces of this app.
///
/// The workgroup's ledger is the bridge's to answer, and the bridge being down is not the
/// server half's failure: the ledger's absence is one line, and the exit is the server
/// half's.
async fn workspace_show(app: &'static str, version: &'static str) -> Result<i32> {
    let Some(client) = connect_app(app, version).await? else {
        return Ok(1);
    };
    let mut out = String::new();
    let code = render_workspace_show(Arc::new(client), app, &mut out).await?;
    for line in out.lines() {
        println!("{line}");
    }
    Ok(code)
}

/// [`workspace_show`]'s text, over one connection to the app's server.
///
/// The workgroup half goes through the same connection, relayed to the bridge. Public so
/// the integration tests can capture it.
#[doc(hidden)]
pub async fn render_workspace_show(
    client: Arc<sapphire_ipc::Client>,
    app: &str,
    out: &mut String,
) -> Result<i32> {
    let code = render_workspace(&client, out).await?;
    let bridge = BridgeClient::relayed(client, sapphire_ipc::runtime_dir()?);
    let workspaces = match bridge.workspaces().await {
        Ok(workspaces) => workspaces,
        Err(sapphire_ipc::Error::NotRunning(_)) => {
            writeln!(out, "no sapphire-bridge is running")
                .expect("writing to a String cannot fail");
            return Ok(code);
        }
        Err(error) => return Err(error.into()),
    };
    let mine: Vec<_> = workspaces
        .workspaces
        .into_iter()
        .filter(|w| w.app_name == app)
        .collect();
    if !mine.is_empty() {
        writeln!(out).expect("writing to a String cannot fail");
        writeln!(out, "in the workgroup:").expect("writing to a String cannot fail");
        for workspace in mine {
            writeln!(out, "{} {}", workspace.name, workspace.workspace_id)
                .expect("writing to a String cannot fail");
        }
    }
    Ok(code)
}

/// Probe, handshake and call `SERVER_INFO` in one go, or report absence.
///
/// Every step the CLI needs — liveness and the API gate — is inside
/// [`connect_or_absent`], so the command is that call plus one `SERVER_INFO` round trip.
/// The API gate doubles as the liveness check here: a live server of another API version
/// is `ApiVersionMismatch`, which names both API versions and advises upgrading and
/// restarting the service. The report goes through [`run_status_into`] so tests can
/// capture it.
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
        api: proto::API_VERSION,
        app_api: None,
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
    // A server older than `server.update` answers method-not-found: there is nothing to
    // say about releases then, and the restart line still stands.
    let update: Option<proto::UpdateStatus> = client
        .call(proto::SERVER_UPDATE, serde_json::json!({}))
        .await
        .ok();
    for line in describe_update(app, version, report.version.as_deref(), update.as_ref()) {
        writeln!(out, "{line}").expect("writing to a String cannot fail");
    }
    Ok(0)
}

/// The `update` and `restart` lines of `status`.
///
/// `installed` is this CLI's version — the executable on disk — and `running` the
/// server's. A server older than what is installed was upgraded under it and needs a
/// restart; a release newer than what is installed is an update to install. A release
/// that is newer than the server but already installed is only the restart.
fn describe_update(
    app: &str,
    installed: &str,
    running: Option<&str>,
    update: Option<&proto::UpdateStatus>,
) -> Vec<String> {
    let parse = |v: &str| semver::Version::parse(v).ok();
    let installed_v = parse(installed);
    let mut lines = Vec::new();
    if let Some(update) = update.filter(|u| u.enabled) {
        match (&update.latest, &update.last_error) {
            (Some(latest), _) => {
                let newer = match (parse(&latest.version), &installed_v) {
                    (Some(latest), Some(installed)) => latest > *installed,
                    _ => update.update_available,
                };
                if newer {
                    lines.push(format!(
                        "update: {} is available ({})",
                        latest.version, latest.url
                    ));
                } else {
                    lines.push(format!(
                        "update: up to date (latest release {})",
                        latest.version
                    ));
                }
            }
            (None, Some(error)) => lines.push(format!("update: could not check: {error}")),
            (None, None) => lines.push("update: not checked yet".to_owned()),
        }
    }
    if let (Some(running), Some(installed)) = (running.and_then(parse), installed_v)
        && installed > running
    {
        lines.push(format!(
            "restart: needed: {installed} is installed but {running} is running; restart the {app} service"
        ));
    }
    lines
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

    /// A desktop binary: the same verbs, and none at all to open the window.
    #[derive(Parser)]
    struct DesktopProbe {
        #[command(subcommand)]
        command: Option<Command>,
    }

    #[test]
    fn a_desktop_binary_takes_the_same_verbs_or_none() {
        let probe = DesktopProbe::try_parse_from(["app"]).unwrap();
        assert!(probe.command.is_none());
        let probe = DesktopProbe::try_parse_from(["app", "serve"]).unwrap();
        assert!(matches!(
            probe.command,
            Some(Command::Framework(FrameworkCommand::Serve))
        ));
        let probe = DesktopProbe::try_parse_from(["app", "greet"]).unwrap();
        assert!(matches!(probe.command, Some(Command::Greet)));
        // The CLI version's bare invocation is an error, which clap answers with the help.
        assert!(Probe::try_parse_from(["app"]).is_err());
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

    #[cfg(feature = "mcp")]
    #[test]
    fn the_mcp_command_parses_beside_the_apps() {
        let probe = Probe::try_parse_from(["app", "mcp"]).unwrap();
        assert!(matches!(
            probe.command,
            Command::Framework(FrameworkCommand::Mcp)
        ));
    }

    #[test]
    fn the_update_command_parses_with_and_without_the_bridge() {
        let probe = Probe::try_parse_from(["app", "update", "--check", "-y"]).unwrap();
        let Command::Framework(FrameworkCommand::Update(args)) = probe.command else {
            panic!("not update");
        };
        assert!(args.update.check && args.update.yes && !args.no_bridge);
        let probe = Probe::try_parse_from(["app", "update", "--no-bridge"]).unwrap();
        let Command::Framework(FrameworkCommand::Update(args)) = probe.command else {
            panic!("not update");
        };
        assert!(args.no_bridge);
    }

    #[test]
    fn the_bridge_is_found_in_the_first_directory_of_path_that_has_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (empty, first, second) = (
            tmp.path().join("empty"),
            tmp.path().join("first"),
            tmp.path().join("second"),
        );
        let file = format!("sapphire-bridge{}", std::env::consts::EXE_SUFFIX);
        for dir in [&empty, &first, &second] {
            std::fs::create_dir(dir).unwrap();
        }
        std::fs::write(first.join(&file), b"").unwrap();
        std::fs::write(second.join(&file), b"").unwrap();
        let path = std::env::join_paths([&empty, &first, &second]).unwrap();
        assert_eq!(
            find_on_path("sapphire-bridge", Some(&path)),
            Some(first.join(&file))
        );
        let path = std::env::join_paths([&empty]).unwrap();
        assert_eq!(find_on_path("sapphire-bridge", Some(&path)), None);
        assert_eq!(find_on_path("sapphire-bridge", None), None);
    }

    /// An executable shell script at `dir/name` that runs `body`.
    #[cfg(unix)]
    fn script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_bridge_runs_its_own_update_with_the_apps_flags() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("args");
        // Records the real run (not the `--help` probe) and exits as an update that
        // declined would.
        let exe = script(
            tmp.path(),
            "bridge",
            &format!(
                r#"case "$*" in *--help*) exit 0;; esac; echo "$@" > {}; exit 1"#,
                log.display()
            ),
        );
        let args = sapphire_framework_update::UpdateArgs {
            check: true,
            yes: true,
        };
        assert_eq!(run_bridge_update(&exe, &args).await.unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(&log).unwrap().trim(),
            "update --check --yes"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_bridge_without_update_is_not_run() {
        let tmp = tempfile::tempdir().unwrap();
        let runs = tmp.path().join("runs");
        // What clap does with an unknown subcommand: exit 2, whatever the arguments.
        let exe = script(
            tmp.path(),
            "bridge",
            &format!(r#"echo "$*" >> {}; exit 2"#, runs.display()),
        );
        let code = run_bridge_update(&exe, &Default::default()).await.unwrap();
        assert_eq!(code, 1);
        assert_eq!(
            std::fs::read_to_string(&runs).unwrap(),
            "update --help\n",
            "only the probe ran"
        );
    }

    #[test]
    fn the_service_subcommands_parse() {
        for args in [
            vec!["app", "service", "install"],
            vec!["app", "service", "uninstall"],
            vec!["app", "service", "status"],
        ] {
            assert!(Probe::try_parse_from(&args).is_ok(), "{args:?}");
        }
    }

    #[test]
    fn the_workspace_and_bridge_commands_parse() {
        for args in [
            vec!["app", "workspace", "init"],
            vec!["app", "workspace", "init", "papers"],
            vec!["app", "workspace", "init", "--sync"],
            vec!["app", "workspace", "show"],
            vec!["app", "workspace", "select", "papers"],
            vec!["app", "workspace", "select", "--sync"],
            vec!["app", "workspace", "select", "papers", "--sync"],
            vec!["app", "workspace", "sync", "enable"],
            vec!["app", "workspace", "sync", "disable"],
            vec!["app", "workspace", "sync", "status"],
            vec![
                "app",
                "workspace",
                "init",
                "--sync",
                "--remote-workspace=Ledger",
            ],
            vec![
                "app",
                "workspace",
                "init",
                "papers",
                "--sync",
                "--remote-workspace",
                "Ledger",
            ],
            vec!["app", "workspace", "move", "../papers"],
            vec!["app", "workspace", "remove"],
            vec!["app", "workspace", "remove", "--delete"],
            vec![
                "app",
                "workgroup",
                "create",
                "--device-name",
                "laptop",
                "home",
            ],
            vec!["app", "workgroup", "list"],
            vec![
                "app",
                "workgroup",
                "join",
                "--device-name",
                "laptop",
                "TICKET",
            ],
            vec!["app", "device", "list"],
            vec!["app", "device", "invite", "--name", "phone"],
            vec!["app", "device", "retire", "phone"],
        ] {
            assert!(Probe::try_parse_from(&args).is_ok(), "{args:?}");
        }
    }

    #[test]
    fn a_remote_workspace_needs_sync() {
        let args = ["app", "workspace", "init", "--remote-workspace=Ledger"];
        assert!(Probe::try_parse_from(args).is_err());
    }

    #[test]
    fn bare_invocation_prints_the_help() {
        // <app> with no subcommand prints the help; the server starts only on an explicit
        // `serve`. An app CLI gets that by a required subcommand and
        // `arg_required_else_help`, as this probe does.
        #[derive(Parser)]
        #[command(arg_required_else_help = true)]
        struct Bare {
            #[command(subcommand)]
            command: Command,
        }
        let err = Bare::try_parse_from(["app"]).err().unwrap();
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
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

    fn update(latest: Option<&str>, error: Option<&str>) -> proto::UpdateStatus {
        proto::UpdateStatus {
            enabled: true,
            latest: latest.map(|version| proto::ReleaseInfo {
                version: version.to_owned(),
                url: format!("https://example.invalid/v{version}"),
            }),
            update_available: false,
            last_error: error.map(str::to_owned),
        }
    }

    #[test]
    fn status_names_a_newer_release_and_nothing_when_up_to_date() {
        let lines = describe_update(
            "app",
            "0.19.0",
            Some("0.19.0"),
            Some(&update(Some("0.20.0"), None)),
        );
        assert_eq!(
            lines,
            vec!["update: 0.20.0 is available (https://example.invalid/v0.20.0)"]
        );
        let lines = describe_update(
            "app",
            "0.19.0",
            Some("0.19.0"),
            Some(&update(Some("0.19.0"), None)),
        );
        assert_eq!(lines, vec!["update: up to date (latest release 0.19.0)"]);
    }

    #[test]
    fn status_asks_for_a_restart_when_the_installed_server_is_newer_than_the_running_one() {
        // Installed 0.20.0 under a running 0.19.0: the release is no longer news, the
        // restart is.
        let lines = describe_update(
            "app",
            "0.20.0",
            Some("0.19.0"),
            Some(&update(Some("0.20.0"), None)),
        );
        assert_eq!(
            lines,
            vec![
                "update: up to date (latest release 0.20.0)",
                "restart: needed: 0.20.0 is installed but 0.19.0 is running; restart the app service",
            ]
        );
        // An older server without `server.update` still gets the restart line.
        let lines = describe_update("app", "0.20.0", Some("0.19.0"), None);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("restart: needed"));
    }

    #[test]
    fn status_says_why_a_check_failed_and_nothing_when_checks_are_off() {
        let lines = describe_update(
            "app",
            "0.19.0",
            Some("0.19.0"),
            Some(&update(None, Some("offline"))),
        );
        assert_eq!(lines, vec!["update: could not check: offline"]);
        let off = proto::UpdateStatus::default();
        assert!(describe_update("app", "0.19.0", Some("0.19.0"), Some(&off)).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn status_shows_a_newer_release_the_server_found() {
        let tmp = tempfile::tempdir().unwrap();
        let endpoint = sapphire_ipc::Endpoint::in_dir("status-test", tmp.path().to_path_buf());
        let api = crate::update::stub::one_response(
            "200 OK",
            r#"[{"tag_name":"v0.20.0","html_url":"https://example.invalid/v0.20.0"}]"#,
        )
        .await;
        let server = AppServer::new(&CTX, "0.19.0")
            .endpoint(endpoint.clone())
            .selection_file(tmp.path().join("workspace.toml"))
            .releases(crate::ReleaseSource::github("o", "r").api_base(api));
        let handle = tokio::spawn(async move { server.run().await });
        wait_until_listening(&endpoint).await;

        // The first look runs as the server starts; wait for its answer to land.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let out = loop {
            let mut out = String::new();
            run_status_into(&endpoint, CTX.app_name, "0.19.0", &mut out)
                .await
                .unwrap();
            if !out.contains("not checked yet") {
                break out;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "never checked: {out}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        assert!(
            out.contains("update: 0.20.0 is available (https://example.invalid/v0.20.0)"),
            "output was: {out}"
        );
        assert!(!out.contains("restart:"), "output was: {out}");

        let (client, _) = sapphire_ipc::connect_or_absent(
            &endpoint,
            CTX.app_name,
            sapphire_ipc::ClientInfo {
                kind: "test".into(),
                version: "0.19.0".into(),
                api: sapphire_backend::protocol::API_VERSION,
                app_api: None,
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

    #[tokio::test(flavor = "multi_thread")]
    async fn status_shows_the_extension_rows_of_a_running_server() {
        let tmp = tempfile::tempdir().unwrap();
        let endpoint = sapphire_ipc::Endpoint::in_dir("status-test", tmp.path().to_path_buf());
        let server = AppServer::new(&CTX, "0.0.0")
            .endpoint(endpoint.clone())
            // This context's directories are never initialised here.
            .selection_file(tmp.path().join("workspace.toml"))
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
                api: sapphire_backend::protocol::API_VERSION,
                app_api: None,
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
    use sapphire_workspace::AppContext;

    static CTX: AppContext = AppContext::new("sapphire-servicetest");

    #[test]
    fn the_generated_spec_runs_the_server_not_the_cli() {
        // A service manager starts the executable directly, and the executable's bare
        // invocation is `serve`: the unit's argv must be exactly that, so the server that
        // starts has no CLI verbs on its command line to misparse.
        let spec = AppServer::new(&CTX, "0.0.0").service_spec();
        assert_eq!(spec.args, vec!["serve".to_owned()]);
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
}
