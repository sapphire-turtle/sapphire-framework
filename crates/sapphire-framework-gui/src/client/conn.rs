//! Cached connections, and turning failures into [`Conn`] states.

use std::sync::Arc;
use std::time::Duration;

use sapphire_backend::protocol as proto;
use sapphire_bridge_client::BridgeClient;
use sapphire_ipc::{Client, ClientInfo};

use sapphire_bridge_api::settings::EntryValue;

use super::types::{BridgeState, ClientConfig, Conn, GuiValues, ServerState};

/// What a failure means for the display.
pub fn classify<T>(err: &sapphire_ipc::Error) -> Conn<T> {
    match err {
        sapphire_ipc::Error::NotRunning(_) => Conn::Absent,
        sapphire_ipc::Error::ApiVersionMismatch { .. }
        | sapphire_ipc::Error::AppApiVersionMismatch { .. }
        | sapphire_ipc::Error::VersionMismatch { .. } => Conn::Incompatible(err.to_string()),
        other => Conn::Error(other.to_string()),
    }
}

/// The message for a process that did not answer within `limit`.
pub(crate) fn unanswered(process: &str, limit: Duration) -> String {
    if limit.as_millis() < 1000 {
        format!("{process} did not answer within {} ms", limit.as_millis())
    } else {
        format!("{process} did not answer within {} s", limit.as_secs())
    }
}

/// Whether a relayed bridge call that failed with `err` leaves the app connection unfit.
///
/// An RPC error, the relay's "no bridge is running" among them, is an answer: the app
/// server is there, and only the bridge refused. Anything else — the connection closed, a
/// frame that would not decode — is the connection's own failure.
pub(crate) fn drops_app(err: &sapphire_ipc::Error) -> bool {
    !matches!(
        err,
        sapphire_ipc::Error::Rpc(_) | sapphire_ipc::Error::NotRunning(_)
    )
}

/// What a failed read of one of the theme's settings means.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GuiFailure {
    /// The bridge does not know the key: it is older than the theme's settings.
    OldBridge,
    /// The bridge cannot read this key's value (a hand-edited file, say): that setting
    /// takes its default, the others stand.
    SkipKey,
    /// The connection failed: the whole fetch did.
    Connection,
}

pub(crate) fn gui_failure(err: &sapphire_ipc::Error) -> GuiFailure {
    match err {
        sapphire_ipc::Error::Rpc(rpc) if rpc.message.starts_with("unknown setting") => {
            GuiFailure::OldBridge
        }
        sapphire_ipc::Error::Rpc(_) => GuiFailure::SkipKey,
        _ => GuiFailure::Connection,
    }
}

/// The effective value of each of the theme's settings, and of the language
/// (`gui.locale`) where the bridge knows it; `None` from a bridge that does not know the
/// theme's. A bridge that knows the theme but not the language gives the theme's values
/// alone, so the theme is still followed.
async fn gui_values(c: &BridgeClient) -> sapphire_ipc::Result<Option<GuiValues>> {
    use sapphire_bridge_api::settings::gui;
    let mut values = GuiValues::new();
    for key in gui::THEME_KEYS {
        match c.config_get(key).await {
            Ok(entry) => {
                if let EntryValue::Value(value) = entry.value {
                    values.insert(key.to_owned(), value);
                }
            }
            Err(err) => match gui_failure(&err) {
                GuiFailure::OldBridge => return Ok(None),
                GuiFailure::SkipKey => {}
                GuiFailure::Connection => return Err(err),
            },
        }
    }
    match c.config_get(gui::LOCALE).await {
        Ok(entry) => {
            if let EntryValue::Value(value) = entry.value {
                values.insert(gui::LOCALE.to_owned(), value);
            }
        }
        Err(err) => match gui_failure(&err) {
            GuiFailure::OldBridge | GuiFailure::SkipKey => {}
            GuiFailure::Connection => return Err(err),
        },
    }
    Ok(Some(values))
}

/// The one cached connection, to the app server, which relays the bridge's methods too. A
/// call that fails other than with the server's answer drops the cache, so the next
/// refresh reconnects — and finds the server gone, or back.
#[derive(Default)]
pub(crate) struct Connections {
    app: Option<Arc<Client>>,
}

impl Connections {
    /// The bridge, through the cached app-server connection; `None` until
    /// [`app`](Self::app) has connected.
    pub(crate) fn bridge(&self, cfg: &ClientConfig) -> Option<BridgeClient> {
        let client = Arc::clone(self.app.as_ref()?);
        Some(BridgeClient::relayed(client, cfg.endpoints.app.dir.clone()))
    }

    /// The cached app-server connection itself; `None` until [`app`](Self::app) has
    /// connected.
    pub(crate) fn app_client(&self) -> Option<Arc<Client>> {
        self.app.clone()
    }

    pub(crate) async fn app(
        &mut self,
        cfg: &ClientConfig,
    ) -> sapphire_ipc::Result<Option<&Client>> {
        if self.app.is_none() {
            let info = ClientInfo {
                kind: "gui".to_owned(),
                version: cfg.app.version.to_owned(),
                api: proto::API_VERSION,
                app_api: None,
                pid: std::process::id(),
            };
            self.app = sapphire_ipc::connect_or_absent(&cfg.endpoints.app, cfg.app.app_name, info)
                .await?
                .map(|(client, _)| Arc::new(client));
        }
        Ok(self.app.as_deref())
    }

    pub(crate) fn drop_app(&mut self) {
        self.app = None;
    }

    /// Ask the bridge, through the app server, everything the views show.
    pub(crate) async fn fetch_bridge(&mut self, cfg: &ClientConfig) -> Conn<BridgeState> {
        let result = tokio::time::timeout(cfg.fetch_timeout, async {
            if self.app(cfg).await?.is_none() {
                return Ok(None);
            }
            let Some(c) = self.bridge(cfg) else {
                return Ok(None);
            };
            let status = c.status().await?;
            let (peers, peer_roles, ledger) = if status.workgroup.is_some() {
                let p = c.peers().await?;
                (p.peers, p.roles, c.workspaces().await?.workspaces)
            } else {
                (Vec::new(), Vec::new(), Vec::new())
            };
            // A bridge older than 2.3 has no such method: the screen says so.
            let embedding = c.embed_settings().await.ok();
            // A bridge older than 2.3 has no such method: the screen says so.
            let external_devices = if status.workgroup.is_some() {
                match c
                    .external_device_request(sapphire_bridge_api::ExternalDeviceRequest::List)
                    .await
                {
                    Ok(sapphire_bridge_api::ExternalDeviceOutcome::List(list)) => Some(list),
                    _ => None,
                }
            } else {
                Some(Vec::new())
            };
            // A bridge older than the theme's settings does not know them: the theme stays.
            let gui = gui_values(&c).await?;
            Ok::<_, sapphire_ipc::Error>(Some(BridgeState {
                status,
                peers,
                peer_roles,
                ledger,
                embedding,
                external_devices,
                gui,
            }))
        })
        .await;
        match result {
            Ok(Ok(Some(state))) => Conn::Up(state),
            // Without its app server the GUI cannot tell whether a bridge runs.
            Ok(Ok(None)) => Conn::Error(format!("the {} server is not running", cfg.app.app_name)),
            // The app server answered, for the bridge: its connection is fine.
            Ok(Err(err)) if !drops_app(&err) => classify(&err),
            Ok(Err(err)) => {
                self.drop_app();
                classify(&err)
            }
            Err(_) => {
                self.drop_app();
                Conn::Error(unanswered("sapphire-bridge", cfg.fetch_timeout))
            }
        }
    }

    /// Ask the app server everything the views show.
    pub(crate) async fn fetch_server(&mut self, cfg: &ClientConfig) -> Conn<ServerState> {
        let result = tokio::time::timeout(cfg.fetch_timeout, async {
            let Some(c) = self.app(cfg).await? else {
                return Ok(None);
            };
            let info: proto::StatusReport =
                c.call(proto::SERVER_INFO, serde_json::json!({})).await?;
            let current: proto::WorkspaceCurrentResult = c
                .call(proto::WORKSPACE_CURRENT, serde_json::json!({}))
                .await?;
            // A server older than `server.update` has no such method: the banner says nothing.
            let update: Option<proto::UpdateStatus> = c
                .call(proto::SERVER_UPDATE, serde_json::json!({}))
                .await
                .ok();
            Ok::<_, sapphire_ipc::Error>(Some(ServerState {
                info,
                current: current.workspace,
                update,
            }))
        })
        .await;
        match result {
            Ok(Ok(Some(state))) => Conn::Up(state),
            Ok(Ok(None)) => Conn::Absent,
            Ok(Err(err)) => {
                self.drop_app();
                classify(&err)
            }
            Err(_) => {
                self.drop_app();
                Conn::Error(unanswered(cfg.app.app_name, cfg.fetch_timeout))
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sapphire_ipc::{Connection, ManagedBy, Router, RpcError, ServerInfo};

    /// A configuration whose endpoint nothing listens on: a test hands [`Connections`] its
    /// app connection instead.
    pub(crate) fn config() -> ClientConfig {
        let dir = std::path::PathBuf::from("/run");
        ClientConfig {
            app: super::super::AppIdentity {
                app_name: "app",
                version: "0",
            },
            endpoints: super::super::Endpoints {
                app: sapphire_ipc::Endpoint::in_dir("a", dir),
            },
            service_exes: super::super::ServiceExes {
                bridge: "/b".into(),
                app: "/a".into(),
            },
            refresh: Duration::from_secs(2),
            fetch_timeout: Duration::from_secs(5),
            command_timeout: Duration::from_secs(5),
            install_timeout: Duration::from_secs(5),
        }
    }

    /// Connections holding an app server whose relay answers every bridge method with the
    /// bridge's own error: the app server is up, the bridge refused.
    pub(crate) async fn over_a_refusing_relay() -> Connections {
        let router = sapphire_bridge_api::RELAYED
            .iter()
            .fold(Router::new(), |r, &m| {
                r.method(m, |_| async {
                    Err(RpcError::internal("the bridge said no"))
                })
            });
        let (client_conn, server_conn) = Connection::pair();
        tokio::spawn(async move {
            let info = ServerInfo {
                version: "0".into(),
                api: proto::API_VERSION,
                app_api: None,
                pid: std::process::id(),
                managed_by: ManagedBy::Spawned,
            };
            let _ = sapphire_ipc::serve(server_conn, Arc::new(router), "app", info).await;
        });
        let info = ClientInfo {
            kind: "gui".into(),
            version: "0".into(),
            api: proto::API_VERSION,
            app_api: None,
            pid: std::process::id(),
        };
        let (client, _) = Client::handshake(client_conn, "app", info).await.unwrap();
        Connections {
            app: Some(Arc::new(client)),
        }
    }

    /// Whether `conns` still holds its app connection.
    pub(crate) fn holds_app(conns: &Connections) -> bool {
        conns.app.is_some()
    }

    #[tokio::test]
    async fn a_bridge_error_in_a_poll_keeps_the_app_connection() {
        let cfg = config();
        let mut conns = over_a_refusing_relay().await;

        let state = conns.fetch_bridge(&cfg).await;
        assert!(
            matches!(&state, Conn::Error(m) if m.contains("the bridge said no")),
            "{state:?}"
        );
        assert!(
            holds_app(&conns),
            "the app server answered: its connection is fine"
        );
    }

    #[test]
    fn classify_maps_api_mismatch_to_incompatible() {
        let err = sapphire_ipc::Error::ApiVersionMismatch {
            running: 1,
            ours: 2,
            server_version: "0.1.0".into(),
        };
        match classify::<()>(&err) {
            Conn::Incompatible(msg) => assert_eq!(msg, err.to_string()),
            other => panic!("expected Incompatible, got {other:?}"),
        }
        let err = sapphire_ipc::Error::VersionMismatch { ours: 1, theirs: 2 };
        assert!(matches!(classify::<()>(&err), Conn::Incompatible(_)));
    }

    #[test]
    fn classify_maps_not_running_to_absent_and_the_rest_to_error() {
        assert!(matches!(
            classify::<()>(&sapphire_ipc::Error::NotRunning("x".into())),
            Conn::Absent
        ));
        assert!(matches!(
            classify::<()>(&sapphire_ipc::Error::Closed),
            Conn::Error(_)
        ));
    }
}

#[cfg(test)]
mod gui_tests {
    use super::*;

    fn rpc(message: &str) -> sapphire_ipc::Error {
        sapphire_ipc::Error::Rpc(sapphire_ipc::RpcError::internal(message))
    }

    #[test]
    fn an_unknown_key_is_an_older_bridge() {
        assert_eq!(
            gui_failure(&rpc("unknown setting `gui.theme.contrast`")),
            GuiFailure::OldBridge
        );
    }

    #[test]
    fn a_value_the_bridge_cannot_read_skips_only_that_key() {
        assert_eq!(
            gui_failure(&rpc("cannot read /x/config.toml: invalid type")),
            GuiFailure::SkipKey
        );
    }

    #[test]
    fn a_broken_connection_is_the_connections_failure() {
        assert_eq!(
            gui_failure(&sapphire_ipc::Error::NotRunning("gone".into())),
            GuiFailure::Connection
        );
    }
}
