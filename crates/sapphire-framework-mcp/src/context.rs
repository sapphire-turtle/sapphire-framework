//! What a tool holds: the link to the application's server.

use std::sync::Arc;

use sapphire_bridge_client::BridgeClient;
use sapphire_ipc::{Client, ClientInfo, Endpoint};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::broadcast;

use crate::change::{CHANGE_CAPACITY, Change};
use crate::error::{Error, Result};

/// The application a tool serves, and the IPC link to its server.
///
/// Cheap to clone: every MCP session gets its own handler, and they share one link.
///
/// The two transports link differently. On the server side the link is a client already
/// connected in-process ([`connected`](Self::connected)). On the client side — the `mcp`
/// subcommand an agent launches — it is an endpoint dialled on the first call
/// ([`dial`](Self::dial)): an agent starts its MCP servers when it starts, which may be
/// before the app server is up, and the app server may restart while the agent runs. A call
/// that finds no server says so; a call that loses the connection drops it, and the next
/// call dials again.
///
/// The context also carries the changes the application announces to the subscriptions
/// `subscriptions/listen` holds open ([`notify_tool_list_changed`](Self::notify_tool_list_changed)
/// and the rest). Every handler made from one context shares them, which is how the HTTP
/// transport, whose handlers last one request, reaches a subscription another request opened.
#[derive(Clone, Debug)]
pub struct ToolContext {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    app: &'static str,
    version: &'static str,
    link: Link,
    changes: broadcast::Sender<Change>,
}

#[derive(Debug)]
enum Link {
    /// A client connected for the context's whole life.
    Connected(Arc<Client>),
    /// An endpoint, dialled when a call needs it.
    Dial {
        endpoint: Endpoint,
        app_api: Option<u32>,
        client: tokio::sync::Mutex<Option<Arc<Client>>>,
    },
}

impl ToolContext {
    /// A context over a client that is already connected: the app server's own `/mcp`,
    /// whose client is an in-process connection to the server's router.
    pub fn connected(app: &'static str, version: &'static str, client: Arc<Client>) -> Self {
        ToolContext {
            inner: Arc::new(Inner {
                app,
                version,
                link: Link::Connected(client),
                changes: broadcast::channel(CHANGE_CAPACITY).0,
            }),
        }
    }

    /// A context that dials `endpoint` when a call needs the server: the `mcp` subcommand.
    ///
    /// `app_api` is the version of the application's own API its tools call, as with
    /// `IpcBackend::connect`; `None` when they call only the framework's methods.
    pub fn dial(
        app: &'static str,
        version: &'static str,
        app_api: Option<u32>,
        endpoint: Endpoint,
    ) -> Self {
        ToolContext {
            inner: Arc::new(Inner {
                app,
                version,
                link: Link::Dial {
                    endpoint,
                    app_api,
                    client: tokio::sync::Mutex::new(None),
                },
                changes: broadcast::channel(CHANGE_CAPACITY).0,
            }),
        }
    }

    /// The application's name.
    pub fn app(&self) -> &'static str {
        self.inner.app
    }

    /// The version this process reports: to the agent, and in the IPC handshake.
    pub fn version(&self) -> &'static str {
        self.inner.version
    }

    /// The client connected to the application's server, dialling it if need be.
    pub async fn client(&self) -> Result<Arc<Client>> {
        match &self.inner.link {
            Link::Connected(client) => Ok(Arc::clone(client)),
            Link::Dial {
                endpoint,
                app_api,
                client,
            } => {
                let mut held = client.lock().await;
                // A client whose connection closed — the server restarted — is replaced
                // here, before the call, so nothing has been sent on it.
                if let Some(client) = held.as_ref().filter(|client| !client.is_closed()) {
                    return Ok(Arc::clone(client));
                }
                let info = ClientInfo {
                    kind: crate::CLIENT_KIND.to_owned(),
                    version: self.inner.version.to_owned(),
                    api: sapphire_server_api::API_VERSION,
                    app_api: *app_api,
                    pid: std::process::id(),
                };
                let (connected, _) =
                    sapphire_ipc::connect_or_absent(endpoint, self.inner.app, info)
                        .await?
                        .ok_or(Error::NotRunning(self.inner.app))?;
                let connected = Arc::new(connected);
                *held = Some(Arc::clone(&connected));
                Ok(connected)
            }
        }
    }

    /// Call one of the server's methods — the framework's or the application's own.
    ///
    /// A call is never retried: a write the server may have applied before the connection
    /// dropped must not be applied twice. The lost connection is forgotten instead, so the
    /// agent's next call dials again.
    pub async fn call<P, R>(&self, method: &str, params: P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let client = self.client().await?;
        match client.call(method, params).await {
            Ok(result) => Ok(result),
            Err(sapphire_ipc::Error::Closed | sapphire_ipc::Error::Io(_)) => {
                self.forget(&client).await;
                Err(Error::Disconnected(self.inner.app))
            }
            Err(err) => Err(Error::Ipc(err)),
        }
    }

    /// The running bridge, for the workgroup's side of things.
    pub async fn bridge(&self) -> Result<BridgeClient> {
        BridgeClient::connect_running(crate::CLIENT_KIND, self.inner.version)
            .await
            .map_err(Error::Bridge)
    }

    /// Tell every subscription that asked that the tool list changed.
    ///
    /// A change is a signal to read again, not the change itself: a subscription cannot be
    /// resumed, so a client that reconnects reads everything afresh anyway. Nobody listening
    /// is not an error.
    pub fn notify_tool_list_changed(&self) {
        self.announce(Change::ToolListChanged);
    }

    /// Tell every subscription that asked that the resource list changed.
    pub fn notify_resource_list_changed(&self) {
        self.announce(Change::ResourceListChanged);
    }

    /// Tell every subscription to `uri` that the resource changed.
    pub fn notify_resource_updated(&self, uri: impl Into<String>) {
        self.announce(Change::ResourceUpdated(uri.into()));
    }

    fn announce(&self, change: Change) {
        // An error only means no subscription is listening.
        let _ = self.inner.changes.send(change);
    }

    /// A receiver of the changes announced from now on.
    pub(crate) fn changes(&self) -> broadcast::Receiver<Change> {
        self.inner.changes.subscribe()
    }

    /// How many receivers of [`changes`](Self::changes) are alive.
    #[cfg(test)]
    pub(crate) fn listeners(&self) -> usize {
        self.inner.changes.receiver_count()
    }

    /// Drop `lost` if it is still the held client, so the next call dials again.
    async fn forget(&self, lost: &Arc<Client>) {
        if let Link::Dial { client, .. } = &self.inner.link {
            let mut held = client.lock().await;
            if held.as_ref().is_some_and(|held| Arc::ptr_eq(held, lost)) {
                *held = None;
            }
        }
    }
}

impl AsRef<ToolContext> for ToolContext {
    fn as_ref(&self) -> &ToolContext {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::Change;

    fn ctx() -> ToolContext {
        let endpoint = Endpoint::in_dir("sapphire-test", std::env::temp_dir());
        ToolContext::dial("sapphire-test", "0.0.0", None, endpoint)
    }

    #[test]
    fn notifying_with_no_listener_is_fine() {
        let ctx = ctx();
        ctx.notify_tool_list_changed();
        ctx.notify_resource_list_changed();
        ctx.notify_resource_updated("x://a");
    }

    #[test]
    fn a_clone_shares_the_changes() {
        let ctx = ctx();
        let mut rx = ctx.changes();
        assert_eq!(ctx.listeners(), 1);
        ctx.clone().notify_resource_updated("x://a");
        assert_eq!(rx.try_recv(), Ok(Change::ResourceUpdated("x://a".into())));
    }
}
