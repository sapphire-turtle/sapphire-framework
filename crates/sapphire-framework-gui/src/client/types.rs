//! What the client layer hands the views, and what they hand back.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use grain_id::GrainId;
use sapphire_backend::protocol::{CurrentWorkspace, StatusReport, UpdateStatus};
use sapphire_bridge_api::settings::Process;
use sapphire_bridge_api::{
    ConfigEntry, ConfigFieldError, ConfigWriteResult, EmbedSettingsResult, Entry,
    ExternalDeviceInfo, ExternalDeviceRequest, PeerInfo, Scope, SettingDef, StatusResult, Value,
    WorkgroupWorkspaceInfo, WorkspaceRoles,
};
use sapphire_ipc::Endpoint;

/// Which application this GUI belongs to.
#[derive(Clone, Copy, Debug)]
pub struct AppIdentity {
    /// The app name, which names its server's endpoint and its marker (`.{app_name}`).
    pub app_name: &'static str,
    /// This GUI's version, for the handshake.
    pub version: &'static str,
}

/// Where the app server listens. The GUI reaches the bridge through it.
#[derive(Clone, Debug)]
pub struct Endpoints {
    /// The app server.
    pub app: Endpoint,
}

impl Endpoints {
    /// The built-in endpoints in this user's runtime directory.
    pub fn default_for(app_name: &str) -> sapphire_ipc::Result<Endpoints> {
        let dir = sapphire_ipc::runtime_dir()?;
        Ok(Endpoints {
            app: Endpoint::in_dir(app_name, dir),
        })
    }
}

/// The executables whose `service install` verb registers each service.
#[derive(Clone, Debug)]
pub struct ServiceExes {
    /// `sapphire-bridge`.
    pub bridge: PathBuf,
    /// The application's own CLI, or the desktop binary that carries it.
    pub app: PathBuf,
}

impl ServiceExes {
    /// The binaries next to the running executable — how the apps are shipped.
    pub fn siblings(app_name: &str) -> std::io::Result<ServiceExes> {
        let exe = std::env::current_exe()?;
        let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
        let suffix = std::env::consts::EXE_SUFFIX;
        Ok(ServiceExes {
            bridge: dir.join(format!("sapphire-bridge{suffix}")),
            app: dir.join(format!("{app_name}{suffix}")),
        })
    }

    /// For a desktop binary that carries the app server, the CLI and MCP as subcommands
    /// (#366): the app's server is this executable — the AppImage it runs from, inside one
    /// ([`service_exe`](sapphire_framework_service::service_exe)) — and `sapphire-bridge`
    /// is still looked for next to it, until where the bridge is installed is settled
    /// (#357, #363).
    pub fn single_binary() -> std::io::Result<ServiceExes> {
        let exe = std::env::current_exe()?;
        let dir = exe.parent().map(PathBuf::from).unwrap_or_default();
        let suffix = std::env::consts::EXE_SUFFIX;
        Ok(ServiceExes {
            bridge: dir.join(format!("sapphire-bridge{suffix}")),
            app: sapphire_framework_service::service_exe()?,
        })
    }
}

/// Everything [`FrameworkClient::spawn`](super::FrameworkClient::spawn) needs.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Which application.
    pub app: AppIdentity,
    /// Where to connect.
    pub endpoints: Endpoints,
    /// What to run for "install & start".
    pub service_exes: ServiceExes,
    /// How often to refresh the snapshot.
    pub refresh: Duration,
    /// How long one process may take to answer a refresh (connect, handshake and calls)
    /// before it reads as not answering.
    pub fetch_timeout: Duration,
    /// How long a command may take; a join, which dials peers, gets three times this, and
    /// an install has [`install_timeout`](Self::install_timeout).
    pub command_timeout: Duration,
    /// How long `service install` may take. Long, because it may wait on an elevation
    /// (UAC) prompt a person has to answer.
    pub install_timeout: Duration,
}

impl ClientConfig {
    /// The defaults: built-in endpoints, sibling executables, a 2 s refresh, 30 s per
    /// command and 5 min for a service install. A desktop window started with `app::run`
    /// (the `eframe` feature) registers itself as the app server instead
    /// ([`ServiceExes::single_binary`]).
    pub fn new(app: AppIdentity) -> std::io::Result<ClientConfig> {
        Ok(ClientConfig {
            endpoints: Endpoints::default_for(app.app_name).map_err(std::io::Error::other)?,
            service_exes: ServiceExes::siblings(app.app_name)?,
            refresh: Duration::from_secs(2),
            fetch_timeout: Duration::from_secs(5),
            command_timeout: Duration::from_secs(30),
            install_timeout: Duration::from_secs(5 * 60),
            app,
        })
    }
}

/// One process, as last seen.
#[derive(Clone, Debug)]
pub enum Conn<T> {
    /// Nothing is listening.
    Absent,
    /// It answers, but speaks another API version. The message says which to update.
    Incompatible(String),
    /// It answered with an error, or the connection failed mid-call.
    Error(String),
    /// It answered.
    Up(T),
}

impl<T> Conn<T> {
    /// The state, when the process answered.
    pub fn up(&self) -> Option<&T> {
        match self {
            Conn::Up(t) => Some(t),
            _ => None,
        }
    }
}

/// What the bridge said.
#[derive(Clone, Debug)]
pub struct BridgeState {
    /// `bridge.status`.
    pub status: StatusResult,
    /// `bridge.peers` (empty without a workgroup).
    pub peers: Vec<PeerInfo>,
    /// The roles `bridge.peers` reports per workspace (empty without a workgroup).
    pub peer_roles: Vec<WorkspaceRoles>,
    /// `bridge.workspaces` (empty without a workgroup).
    pub ledger: Vec<WorkgroupWorkspaceInfo>,
    /// `embed.settings`; `None` from a bridge older than 2.3.
    pub embedding: Option<EmbedSettingsResult>,
    /// `external_device.list` (empty without a workgroup); `None` from an older bridge.
    pub external_devices: Option<Vec<ExternalDeviceInfo>>,
    /// The theme's settings (`gui.theme.*`) and the language (`gui.locale`), effective
    /// values; `None` from a bridge that does not know the theme's. `gui.locale` is missing
    /// from a bridge that knows the theme but not the language: the theme is followed, the
    /// language stays as installed.
    pub gui: Option<GuiValues>,
}

/// The effective values of the window's settings, by key.
pub type GuiValues = BTreeMap<String, Value>;

/// What the app server said.
#[derive(Clone, Debug)]
pub struct ServerState {
    /// `server.info`.
    pub info: StatusReport,
    /// `workspace.current`: the one workspace the server serves, if it has one (#215).
    pub current: Option<CurrentWorkspace>,
    /// `server.update`; `None` from a server older than the method.
    pub update: Option<UpdateStatus>,
}

/// Both processes, as of the last refresh.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The bridge.
    pub bridge: Conn<BridgeState>,
    /// The app server.
    pub server: Conn<ServerState>,
    /// `false` until the first refresh has finished.
    pub fetched: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            bridge: Conn::Absent,
            server: Conn::Absent,
            fetched: false,
        }
    }
}

/// Which service an install targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceTarget {
    /// `sapphire-bridge`.
    Bridge,
    /// The application's server.
    App,
}

/// Something the user asked for.
#[derive(Clone, Debug)]
pub enum Command {
    /// Found a workgroup.
    WorkgroupCreate {
        /// The workgroup's name.
        name: String,
        /// This device's name in it.
        device_name: String,
    },
    /// Join with a ticket.
    WorkgroupJoin {
        /// The invite ticket.
        ticket: String,
        /// This device's name, or the bridge's default.
        device_name: Option<String>,
    },
    /// Issue an invite ticket.
    DeviceInvite {
        /// The name the invited device will carry.
        name: String,
        /// How long the ticket stays valid, in seconds.
        ttl_secs: Option<u64>,
    },
    /// Retire a device.
    DeviceRetire {
        /// The device's name or id.
        selector: String,
    },
    /// Manage the workgroup's external devices.
    ExternalDevice(ExternalDeviceRequest),
    /// Set a device's election priority.
    DevicePrioritySet {
        /// The device's name or id.
        selector: String,
        /// `0..=255`; 0 opts the device out of the election.
        priority: u8,
    },
    /// Create (or adopt) a workspace in `dir` and switch the server to it, optionally
    /// syncing it.
    WorkspaceInit {
        /// The workspace directory.
        dir: PathBuf,
        /// Enable sync right after.
        sync: bool,
    },
    /// Switch the server to the existing workspace in `dir`.
    WorkspaceSelect {
        /// The workspace's root.
        dir: PathBuf,
    },
    /// Move the current workspace's folder to `dir`, which must not exist yet.
    WorkspaceMove {
        /// The new root.
        dir: PathBuf,
    },
    /// Stop serving the current workspace on this host; with `delete`, delete its folder.
    WorkspaceRemove {
        /// Also delete the folder and every file in it.
        delete: bool,
    },
    /// Start syncing the current workspace.
    SyncEnable,
    /// Stop syncing the current workspace.
    SyncDisable,
    /// Bring a workgroup workspace to `dir`, creating the folder and initialising it as a
    /// workspace first if needed, and switch the server to it.
    WorkspaceMap {
        /// The workgroup workspace to map.
        workspace_id: GrainId,
        /// Where it goes on this host.
        dir: PathBuf,
    },
    /// Run the target's `service install`.
    ServiceInstall(ServiceTarget),
    /// Read the settings: the schema, the effective values and each scope's file.
    ConfigLoad,
    /// Write settings to one scope: `set` and `unset` are checked first, then written, in
    /// that order; the first failure stops it.
    ConfigWrite {
        /// The scope to write.
        scope: Scope,
        /// The keys to set, with their values.
        set: Vec<ConfigEntry>,
        /// The keys (or prefixes) to remove.
        unset: Vec<String>,
    },
    /// Restart a process so its restart-only settings take effect, and wait until it is back.
    Restart(Process),
}

/// The settings, as one [`Command::ConfigLoad`] read them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConfigState {
    /// Every setting the app server and the bridge know; the bridge's are missing when it
    /// does not answer.
    pub schema: Vec<SettingDef>,
    /// Every setting's effective value.
    pub effective: Vec<Entry>,
    /// What each scope's file holds: `Device`, `Workgroup`, and `Workspace` only when a
    /// workspace is selected.
    pub by_scope: BTreeMap<Scope, Vec<Entry>>,
}

/// What a finished command produced.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandOutput {
    /// Nothing to show beyond success.
    Done,
    /// An invite ticket, to show once.
    Ticket(String),
    /// An external device's token, to show once.
    Token(String),
    /// The settings [`Command::ConfigLoad`] read.
    Config(ConfigState),
    /// What a [`Command::ConfigWrite`] changed, and every key awaiting a restart.
    Written(ConfigWriteResult),
}

/// Why a command failed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandError {
    /// The message to show.
    pub message: String,
    /// The keys a rejected settings write names, each with what is wrong with it.
    pub fields: Vec<ConfigFieldError>,
}

impl From<String> for CommandError {
    fn from(message: String) -> CommandError {
        CommandError {
            message,
            fields: Vec::new(),
        }
    }
}

impl From<&str> for CommandError {
    fn from(message: &str) -> CommandError {
        message.to_owned().into()
    }
}

/// Names one sent command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CommandId(pub u64);

/// How a sent command ended.
#[derive(Clone, Debug)]
pub struct Outcome {
    /// Which command.
    pub id: CommandId,
    /// Its result; the error carries the message to show.
    pub result: Result<CommandOutput, CommandError>,
}
