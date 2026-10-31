//! `sapphire-framework` — a single-dependency facade over the local-first
//! framework crates.
//!
//! Depend on this one crate and enable the features you need; each feature
//! re-exports an internal `sapphire-framework-*` crate as a module. This mirrors
//! the way large Rust libraries (e.g. bevy) ship a facade over many internal
//! crates: the split crates keep compile-time isolation, while consumers depend
//! on one name.
//!
//! ```toml
//! # A native app that indexes a local workspace and talks to its app server:
//! sapphire-framework = { version = "0.1", features = ["native", "redb-store"] }
//! ```
//!
//! ```ignore
//! // (requires the `backend` feature)
//! use sapphire_framework::prelude::*;
//! let backend = LocalBackend::new(state);
//! ```
//!
//! ## Modules (feature-gated)
//!
//! | feature | module | crate |
//! |---|---|---|
//! | `workspace` | [`workspace`] | `sapphire-framework-workspace` |
//! | `retrieve` | [`retrieve`] | `sapphire-framework-retrieve` |
//! | `track` | [`track`] | `sapphire-framework-track` |
//! | `sync` | [`sync`] | `sapphire-framework-sync` |
//! | `session` | [`session`] | `sapphire-framework-session` |
//! | `ipc` | [`ipc`] | `sapphire-framework-ipc` |
//! | `server` | [`server`] | `sapphire-framework-server` |
//! | `keys` | [`keys`] | `sapphire-framework-keys` |
//! | `registry` | [`registry`] | `sapphire-framework-registry` |
//! | `bridge` | [`bridge`] | `sapphire-framework-bridge` |
//! | `bridge-embed` | [`bridge_embed`] | `sapphire-framework-bridge-embed` (for apps that embed the bridge in-process, e.g. mobile; not in `native`) |
//! | `backend` | [`backend`] | `sapphire-framework-backend` |
//! | `gui` | [`gui`] | `sapphire-framework-gui` |
//! | `eframe` | `gui::app`, `gui::eframe` | `sapphire-framework-gui` with its `eframe` feature: a desktop window started the same way for every app |
//! | `mobile` | `gui::mobile` | `sapphire-framework-gui` with its `mobile` feature: a phone app hosting its bridge and server in-process (implies `eframe`, `server`, `bridge`, `ipc`) |
//! | `service` | [`service`] | `sapphire-framework-service` |
//! | `http` | — | the app server's HTTP listener (`AppServer::http`) |
//! | `frontmatter` | [`frontmatter`] | `sapphire-framework-frontmatter` |
//! | `mcp` | [`mcp`] | `sapphire-framework-mcp`, plus the `mcp` subcommand and `/mcp` on the app server |
//! | `metrics` | — | Prometheus metrics at `/metrics` on the app server's HTTP listener |

#[cfg(feature = "workspace")]
pub use sapphire_framework_workspace as workspace;

// The dependency re-exports ride in on the `workspace` feature: apps depend on
// this facade alone and get the same `clap`/`serde`/`dirs` versions the
// workspace crate builds the directory helpers with (issue #128).
#[cfg(feature = "workspace")]
pub use sapphire_framework_workspace::{clap, dirs, serde};

#[cfg(feature = "retrieve")]
pub use sapphire_framework_retrieve as retrieve;

#[cfg(feature = "track")]
pub use sapphire_framework_track as track;

#[cfg(feature = "sync")]
pub use sapphire_framework_sync as sync;

#[cfg(feature = "session")]
pub use sapphire_framework_session as session;

#[cfg(feature = "ipc")]
pub use sapphire_framework_ipc as ipc;

#[cfg(feature = "server")]
pub use sapphire_framework_server as server;

/// The `metrics` crate, for an application's own metrics (see
/// `AppServer::metrics_collector`); the same crate the server records its own with.
#[cfg(feature = "server")]
pub use sapphire_framework_server::metrics;

#[cfg(feature = "keys")]
pub use sapphire_framework_keys as keys;

#[cfg(feature = "registry")]
pub use sapphire_framework_registry as registry;

#[cfg(feature = "bridge")]
pub use sapphire_framework_bridge as bridge;

#[cfg(feature = "bridge-embed")]
pub use sapphire_framework_bridge_embed as bridge_embed;

#[cfg(feature = "backend")]
pub use sapphire_backend as backend;

#[cfg(feature = "gui")]
pub use sapphire_framework_gui as gui;

#[cfg(feature = "service")]
pub use sapphire_framework_service as service;

#[cfg(feature = "mcp")]
pub use sapphire_framework_mcp as mcp;

#[cfg(feature = "frontmatter")]
pub use sapphire_framework_frontmatter as frontmatter;

/// Commonly-used types, re-exported for `use sapphire_framework::prelude::*;`.
///
/// What is available depends on the enabled features.
pub mod prelude {
    #[cfg(feature = "workspace")]
    #[allow(deprecated)]
    pub use crate::workspace::{
        AppContext, AppKind, FileSearchResult, RetrieveConfig, RetrieveParams, SearchMode,
        Workspace, WorkspaceArgs, WorkspaceState,
    };

    #[cfg(feature = "backend")]
    #[allow(deprecated)]
    pub use crate::backend::{
        BackendEvent, LocalBackend, SyncSummary, WorkspaceBackend, WorkspaceEntry,
        WorkspaceLocator, WorkspaceRegistry, WorkspaceSelection, WorkspaceSource,
    };

    // A UI leaves the cache to the app server: `IpcBackend` implements
    // `WorkspaceBackend` over IPC, and `protocol` names the `workspace.*`
    // methods and their types for both sides of the connection.
    #[cfg(feature = "backend")]
    pub use crate::backend::{IpcBackend, protocol};

    // Bearer-token authentication against the served workspace's API keys (#364); the
    // `keys` feature chains the crate's `axum` feature in, so an application hosting its own
    // authenticated routes gets `protect` from here, and wraps the server's
    // `WorkspaceKeyVerifier` in a `Verifier` of its own when it maps keys to something.
    #[cfg(feature = "keys")]
    pub use crate::keys::{AuthConfig, Authenticated, KeyId, Verdict, Verifier, protect};

    // The app server skeleton: an application builds one of these, adds its own
    // methods, and runs it. `WorkspaceHost` is here for handlers that reach a
    // workspace the same way the framework's own do. `FrameworkCommand` is the
    // flat command vocabulary an app flattens into its own CLI beside its own
    // subcommands (issue #142); the sub-enums and the typed status report come
    // with it, so an app never needs the `server` module to build its CLI.
    // `ReleaseSource` names where the app's releases are, for `AppServer::releases`.
    // `serve_restarting` is the app's entry point, with its own settings (`AppSettings`)
    // and the `Settings` its build function reads; `AppConfigCommand` is `config`.
    #[cfg(feature = "server")]
    pub use sapphire_framework_server::{
        AppConfigCommand, AppScopeFlags, AppServer, AppSettings, DeviceCommand, FrameworkCommand,
        ReleaseSource, Settings, StatusReport, StatusRow, WorkgroupCommand, WorkspaceCommand,
        WorkspaceHost, serve_restarting,
    };
}

#[cfg(all(test, feature = "server"))]
mod tests {
    //! Pin the prelude's server-side surface (issue #142, plan Task 5).

    use super::prelude::*;

    #[test]
    fn the_prelude_exports_the_whole_framework_command_surface() {
        // The flat command vocabulary an app flattens beside its own verbs.
        let _serve: FrameworkCommand = FrameworkCommand::Serve;
        let _status: FrameworkCommand = FrameworkCommand::Status;
        let _workspace: FrameworkCommand = FrameworkCommand::Workspace(WorkspaceCommand::Show);
        let _workgroup: FrameworkCommand = FrameworkCommand::Workgroup(WorkgroupCommand::List);
        let _device: FrameworkCommand = FrameworkCommand::Device(DeviceCommand::List);
        let _config: FrameworkCommand = FrameworkCommand::Config(AppConfigCommand::Get {
            key: "http.listen".to_owned(),
        });
        // The entry point, with an application's settings and its build function.
        async fn _entry<A: AppSettings + Clone, F: Fn(&Settings) -> AppServer>(
            ctx: &'static AppContext,
            owner: A,
            build: F,
        ) {
            let _ = serve_restarting(ctx, owner, build).await;
        }

        // The typed status report shared by the CLI and the IPC `server.info`.
        let _report = StatusReport {
            running: false,
            version: None,
            pid: None,
            managed_by: None,
            app: vec![StatusRow {
                name: "sync".to_owned(),
                value: "enabled".to_owned(),
            }],
        };
    }
}
