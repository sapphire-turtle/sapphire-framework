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
use std::time::Duration;

use sapphire_bridge_api::{BRIDGE_NOT_RUNNING, RELAYED, RESTART};
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

    /// An open connection to the bridge, or the error to answer with when there is none.
    ///
    /// The sync runtime's is whatever its slot holds. The relay's own is the one it made
    /// last if that is still open, and otherwise one attempt to make another: no bridge
    /// there is [`BRIDGE_NOT_RUNNING`], and a bridge that refuses the connection — one of
    /// another API version, say — is its refusal, which carries no params to keep secret.
    async fn client(&self) -> Result<Arc<BridgeClient>, RpcError> {
        match self {
            BridgeLink::Sync(runtime) => runtime.connected_bridge().ok_or_else(not_running),
            BridgeLink::Own(own) => {
                // Held across the attempt, so calls racing in on a fresh server make one
                // connection between them.
                let mut current = own.current.lock().await;
                if let Some(client) = current.as_ref().filter(|c| !c.is_closed()) {
                    return Ok(Arc::clone(client));
                }
                *current = None;
                match own.connector.connect().await {
                    Ok(Some(client)) => {
                        let client = Arc::new(client);
                        *current = Some(Arc::clone(&client));
                        Ok(client)
                    }
                    Ok(None) => Err(not_running()),
                    Err(err) => {
                        tracing::warn!("could not connect to sapphire-bridge: {err}");
                        Err(RpcError::internal(err.to_string()))
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

    /// Wait until `old` has closed and an open connection to another bridge is held.
    ///
    /// The sync runtime reconnects on its own, so this follows its slot. The relay's own
    /// link tries the connector every [`RECONNECT_INTERVAL`] until a bridge answers.
    async fn replaced(&self, old: &Arc<BridgeClient>) {
        old.closed().await;
        match self {
            BridgeLink::Sync(runtime) => {
                let mut changes = runtime.bridge_changes();
                loop {
                    if changes
                        .borrow_and_update()
                        .as_ref()
                        .is_some_and(|c| !Arc::ptr_eq(c, old) && !c.is_closed())
                    {
                        return;
                    }
                    if changes.changed().await.is_err() {
                        // The runtime is gone, so no new bridge is coming: the caller's
                        // timeout ends the wait.
                        std::future::pending::<()>().await;
                    }
                }
            }
            BridgeLink::Own(own) => loop {
                {
                    let mut current = own.current.lock().await;
                    // A call racing this one may have reached the new bridge first.
                    if current
                        .as_ref()
                        .is_some_and(|c| !Arc::ptr_eq(c, old) && !c.is_closed())
                    {
                        return;
                    }
                    match own.connector.connect().await {
                        Ok(Some(client)) => {
                            *current = Some(Arc::new(client));
                            return;
                        }
                        Ok(None) => {}
                        // Not logged at warn: the new bridge refusing for a moment is part
                        // of a restart, and this tries again every interval.
                        Err(err) => tracing::debug!("could not connect to sapphire-bridge: {err}"),
                    }
                }
                tokio::time::sleep(RECONNECT_INTERVAL).await;
            },
        }
    }
}

/// How often the relay's own link tries the new bridge after a relayed `bridge.restart`.
const RECONNECT_INTERVAL: Duration = Duration::from_millis(100);

/// The relay's answer when there is no bridge to forward to.
fn not_running() -> RpcError {
    RpcError {
        code: BRIDGE_NOT_RUNNING,
        message: "no sapphire-bridge is running".to_owned(),
        data: None,
    }
}

/// How long a relayed `bridge.restart` waits for the app server to reach the new bridge.
const RESTART_WAIT: Duration = Duration::from_secs(30);

/// Forwards [`RELAYED`] to the bridge over a [`BridgeLink`].
pub(crate) struct Relay {
    link: BridgeLink,
    /// [`RESTART_WAIT`], but for tests, which cannot wait that long.
    restart_wait: Duration,
}

impl Relay {
    /// A relay over `link`.
    pub(crate) fn new(link: BridgeLink) -> Relay {
        Relay {
            link,
            restart_wait: RESTART_WAIT,
        }
    }

    /// Give up on a restarted bridge after `wait` rather than [`RESTART_WAIT`].
    #[cfg(test)]
    fn restart_wait(mut self, wait: Duration) -> Relay {
        self.restart_wait = wait;
        self
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
    /// The bridge's own errors come back unchanged, and so does a bridge's refusal to be
    /// connected to. No bridge, or a connection that fails under the call, is
    /// [`BRIDGE_NOT_RUNNING`]. Only `method` and the outcome are logged.
    async fn forward(&self, method: &'static str, params: Value) -> Result<Value, RpcError> {
        let result = self.call(method, params).await;
        match &result {
            Ok(_) => tracing::debug!(method, outcome = "ok", "relayed a bridge call"),
            Err(e) => tracing::debug!(method, outcome = e.code, "relayed a bridge call"),
        }
        result
    }

    /// [`forward`](Self::forward), unlogged.
    ///
    /// A `bridge.restart` the bridge accepted answers only once this server is connected to
    /// the new bridge, so the client's next call finds it.
    async fn call(&self, method: &'static str, params: Value) -> Result<Value, RpcError> {
        let client = self.link.client().await?;
        let result = match client.call_raw(method, params).await {
            Ok(result) => result,
            Err(sapphire_ipc::Error::Rpc(e)) => return Err(e),
            // The transport's error is not logged: a codec error may quote the frame.
            Err(_) => {
                self.link.lost(&client).await;
                return Err(not_running());
            }
        };
        if method == RESTART
            && tokio::time::timeout(self.restart_wait, self.link.replaced(&client))
                .await
                .is_err()
        {
            return Err(RpcError::internal(format!(
                "the bridge did not come back within {} s",
                RESTART_WAIT.as_secs()
            )));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::sync::testing::StubBridge;

    /// A connector handing out `bridges` in order, then "no bridge is running" forever.
    fn queued(bridges: Vec<BridgeClient>) -> BridgeConnector {
        let bridges = Arc::new(Mutex::new(VecDeque::from(bridges)));
        BridgeConnector::new(move || {
            let next = bridges.lock().unwrap().pop_front();
            async move { Ok(next) }
        })
    }

    /// A stub answering `bridge.restart` as the bridge does, then going away once asked.
    async fn restarting_stub() -> (Arc<StubBridge>, BridgeClient) {
        let (stub, client) = StubBridge::start_client_in(std::env::temp_dir()).await;
        stub.answer(RESTART, json!({}));
        let stub = Arc::new(stub);
        let going = Arc::clone(&stub);
        tokio::spawn(async move {
            while going.calls().is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            going.disconnect();
        });
        (stub, client)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_bridge_that_refuses_the_connection_is_reported_as_it_said() {
        // An app server and a bridge of different API versions: "no bridge is running"
        // would send the user looking for a bridge that is there.
        let refusal = || sapphire_ipc::Error::ApiVersionMismatch {
            running: 1,
            ours: 2,
            server_version: "9.9.9".into(),
        };
        let relay = Relay::new(BridgeLink::own(BridgeConnector::new(move || async move {
            Err(refusal())
        })));

        let e = relay.forward("bridge.status", json!({})).await.unwrap_err();
        assert_ne!(e.code, BRIDGE_NOT_RUNNING);
        assert_eq!(e.message, refusal().to_string());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_bridge_is_reported_as_not_running() {
        let relay = Relay::new(BridgeLink::own(queued(Vec::new())));

        let e = relay.forward("bridge.status", json!({})).await.unwrap_err();
        assert_eq!(e.code, BRIDGE_NOT_RUNNING);
        assert_eq!(e.message, "no sapphire-bridge is running");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restart_that_never_comes_back_fails_after_the_wait() {
        let (_stub, client) = restarting_stub().await;
        let relay = Relay::new(BridgeLink::own(queued(vec![client])))
            .restart_wait(Duration::from_millis(300));

        let e = relay.forward(RESTART, json!({})).await.unwrap_err();
        assert_eq!(e.message, "the bridge did not come back within 30 s");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_own_link_restart_returns_once_it_reaches_the_new_bridge() {
        let (_old, client) = restarting_stub().await;
        let (new, next) = StubBridge::start_client_in(std::env::temp_dir()).await;
        new.answer("embed.settings", json!({"from": "new"}));
        let relay = Relay::new(BridgeLink::own(queued(vec![client, next])))
            .restart_wait(Duration::from_secs(5));

        let ack = relay.forward(RESTART, json!({})).await.unwrap();
        assert_eq!(ack, json!({}));
        // The next call goes to the new bridge, over the connection the wait made.
        let answer = relay.forward("embed.settings", json!({})).await.unwrap();
        assert_eq!(answer, json!({"from": "new"}));
    }
}
