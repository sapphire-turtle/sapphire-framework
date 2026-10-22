//! The relay: the bridge's client-facing methods, served by the app server (#314).
//!
//! The app's CLI and GUI reach `sapphire-bridge` only through their app server. Each method
//! of [`RELAYED`] is registered under its own name and forwarded as it came — params and
//! result untouched, never read and never logged, since `embed.key_set`, `config.set` of a
//! secret and `external_device.add` / `rotate` carry API keys and tokens. Anything else the
//! bridge serves is not registered, so a client calling it gets `method not found`.
//!
//! See `docs/superpowers/specs/2026-10-22-bridge-relay-design.md`, "The relay".

use std::sync::Arc;

use sapphire_bridge_api::{BRIDGE_NOT_RUNNING, RELAYED};
use sapphire_bridge_client::BridgeClient;
use sapphire_ipc::{Router, RpcError};
use serde_json::Value;

use crate::sync::{BridgeConnector, SyncRuntime};

/// The relay's own connection, for an app server without sync.
///
/// Made on the first relayed call and made again on the next call after it drops: there is
/// no background reconnect loop, so an app that never relays never connects.
pub(crate) struct OwnLink {
    connector: BridgeConnector,
    /// The connection last made, open or not.
    current: tokio::sync::Mutex<Option<Arc<BridgeClient>>>,
}

/// Where the relay's bridge connection comes from. The app server keeps at most one.
pub(crate) enum BridgeLink {
    /// The sync runtime's, which keeps itself connected.
    Sync(Arc<SyncRuntime>),
    /// The relay's own.
    Own(OwnLink),
}

impl BridgeLink {
    /// A link of the relay's own, reaching the bridge with `connector`.
    pub(crate) fn own(connector: BridgeConnector) -> BridgeLink {
        BridgeLink::Own(OwnLink {
            connector,
            current: tokio::sync::Mutex::new(None),
        })
    }

    /// An open connection to the bridge, or `None` when there is none to be had.
    ///
    /// The sync runtime's is whatever its slot holds. The relay's own is the one it made
    /// last if that is still open, and otherwise one attempt to make another.
    async fn client(&self) -> Option<Arc<BridgeClient>> {
        match self {
            BridgeLink::Sync(runtime) => runtime.connected_bridge(),
            BridgeLink::Own(own) => {
                // Held across the attempt, so calls racing in on a fresh server make one
                // connection between them.
                let mut current = own.current.lock().await;
                if let Some(client) = current.as_ref().filter(|c| !c.is_closed()) {
                    return Some(Arc::clone(client));
                }
                *current = None;
                match own.connector.connect().await {
                    Ok(Some(client)) => {
                        let client = Arc::new(client);
                        *current = Some(Arc::clone(&client));
                        Some(client)
                    }
                    Ok(None) => None,
                    Err(err) => {
                        tracing::warn!("could not connect to sapphire-bridge: {err}");
                        None
                    }
                }
            }
        }
    }

    /// `client` failed under a call: forget it, if it is the relay's own, so the next call
    /// makes a new one. The sync runtime notices its own loss.
    async fn lost(&self, client: &Arc<BridgeClient>) {
        if let BridgeLink::Own(own) = self {
            let mut current = own.current.lock().await;
            if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, client)) {
                *current = None;
            }
        }
    }
}

/// The relay's answer when there is no bridge to forward to.
fn not_running() -> RpcError {
    RpcError {
        code: BRIDGE_NOT_RUNNING,
        message: "no sapphire-bridge is running".to_owned(),
        data: None,
    }
}

/// Forwards [`RELAYED`] to the bridge over a [`BridgeLink`].
pub(crate) struct Relay {
    link: BridgeLink,
}

impl Relay {
    /// A relay over `link`.
    pub(crate) fn new(link: BridgeLink) -> Relay {
        Relay { link }
    }

    /// `router` with every method of [`RELAYED`] forwarded.
    pub(crate) fn routes(self: Arc<Self>, router: Router) -> Router {
        RELAYED.iter().fold(router, |router, &method| {
            let relay = Arc::clone(&self);
            router.method(method, move |ctx| {
                let relay = Arc::clone(&relay);
                async move { relay.forward(method, ctx.params).await }
            })
        })
    }

    /// Forward one call, and hand back what the bridge answered.
    ///
    /// The bridge's own errors come back unchanged. No bridge, or a connection that fails
    /// under the call, is [`BRIDGE_NOT_RUNNING`]. Only `method` and the outcome are logged.
    async fn forward(&self, method: &'static str, params: Value) -> Result<Value, RpcError> {
        let Some(client) = self.link.client().await else {
            tracing::debug!(
                method,
                outcome = BRIDGE_NOT_RUNNING,
                "relayed a bridge call"
            );
            return Err(not_running());
        };
        match client.call_raw(method, params).await {
            Ok(result) => {
                tracing::debug!(method, outcome = "ok", "relayed a bridge call");
                Ok(result)
            }
            Err(sapphire_ipc::Error::Rpc(e)) => {
                tracing::debug!(method, outcome = e.code, "relayed a bridge call");
                Err(e)
            }
            // The transport's error is not logged: a codec error may quote the frame.
            Err(_) => {
                self.link.lost(&client).await;
                tracing::debug!(
                    method,
                    outcome = BRIDGE_NOT_RUNNING,
                    "relayed a bridge call"
                );
                Err(not_running())
            }
        }
    }
}
