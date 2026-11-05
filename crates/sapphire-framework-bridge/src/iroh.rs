//! The iroh implementation of [`PeerTransport`](crate::PeerTransport).
//!
//! Everything iroh-shaped lives here. The rest of the bridge knows only "open a stream to a
//! node id" and "accept a stream and learn who called", so replacing this file would not
//! touch anything else.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use grain_id::GrainId;
use sapphire_bridge_api::ALPN;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf,
};

// `iroh` is also the name of this module, so the crate is spelled with a leading `::` or the
// path would be ambiguous.
use std::sync::Arc;

use ::iroh::address_lookup::{DnsAddressLookup, MemoryLookup, PkarrPublisher, PkarrResolver};
use ::iroh::endpoint::presets;
use ::iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayMap, RelayMode, RelayUrl, SecretKey, TransportAddr,
    defaults::prod::default_relay_map,
};

use crate::error::{Error, Result};
use crate::hello::HELLO_ALPN;
use crate::net::NetConfig;
use crate::pairing::PAIR_ALPN;
use crate::peer::{BoxedStream, Inbound, PeerTransport, StreamRequest};
use crate::relay::RelayConfig;

/// How many bytes of the request line to accept before giving up.
///
/// The request is a small JSON object; anything past this is a peer that is not speaking this
/// protocol, and reading forever on it would be a way to make the bridge allocate.
const MAX_REQUEST_LINE: u64 = 4 * 1024;

/// How long [`AcceptedStream`] keeps a connection's last handle alive after the stream
/// itself is dropped, waiting for the peer to close it.
///
/// Per [`iroh::endpoint::Connection`]'s own doc: once every handle to a connection — the
/// `Connection` itself, and every stream still live on it — has been dropped, the connection
/// closes at once, and "closing the connection immediately abandons efforts to deliver data
/// to the peer". `SendStream::finish` only *registers* that a stream is done; it is the
/// connection's own background driver that puts the bytes on the wire and waits for the
/// peer's ack. Every caller here follows the same shape — write a reply, then let the
/// stream drop — so without this, the drop can race that driver: the peer sees its
/// connection die instead of receiving the reply it was just sent. Keeping one more handle
/// open until the peer actually closes (or this timeout elapses) is iroh's own fix, lifted
/// from its `listen` example.
const ACCEPT_CLOSE_GRACE: Duration = Duration::from_secs(5);

/// An inbound stream, plus the connection it came from.
///
/// [`IrohTransport::accept`] hands this out instead of the bare joined stream it used to:
/// see [`ACCEPT_CLOSE_GRACE`] for why a connection must outlive every stream that answered
/// on it.
struct AcceptedStream {
    io: tokio::io::Join<::iroh::endpoint::RecvStream, ::iroh::endpoint::SendStream>,
    conn: ::iroh::endpoint::Connection,
}

impl AsyncRead for AcceptedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}

impl AsyncWrite for AcceptedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().io).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }
}

impl Drop for AcceptedStream {
    fn drop(&mut self) {
        // `io`'s streams are each a handle on this same connection and are about to drop
        // right along with this clone, so without it this would be the last handle —
        // exactly the case this type exists to avoid. The wait runs in the background:
        // `Drop::drop` cannot be `async`, and nothing here needs to block on it — the point
        // is only to give the connection's driver a chance to run before every handle that
        // kept it alive is finally gone.
        let conn = self.conn.clone();
        tokio::spawn(async move {
            let _ = tokio::time::timeout(ACCEPT_CLOSE_GRACE, conn.closed()).await;
        });
    }
}

/// A device's node id and the addresses that reach it.
///
/// This is what one device hands another out of band — the pairing flow this plan leaves for
/// later — so that the other can dial it without a discovery service. It deliberately names
/// nothing from iroh: a caller shares it, stores it, or prints it without linking iroh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeAddr {
    /// The device's node id.
    pub node_id: String,
    /// The socket addresses this device can be reached at, as `host:port`.
    pub addrs: Vec<String>,
    /// The relay URLs this device is reachable through.
    ///
    /// Kept apart from `addrs` because they are a different kind of address: a peer with no
    /// direct path can only be dialed through one of these, and a device built relay-only
    /// has nothing else to offer.
    pub relay_urls: Vec<String>,
}

/// The data plane's ALPN on a connection that carries many streams.
///
/// The protocol on each stream is [`ALPN`]'s, unchanged; what this name adds is the promise
/// that the acceptor keeps taking streams on the connection, so the dialer may keep it and
/// open the next stream on it. A peer from before this name accepted one stream per
/// connection and then let it go: a second stream there would never be read. So a dialer
/// offers both names, the acceptor lists this one first, and a connection only goes into
/// [`IrohTransport`]'s cache when this is the name the two agreed on.
const SHARED_ALPN: &[u8] = b"sapphire/ws/2";

/// [`HELLO_ALPN`] on a connection that carries many streams; see [`SHARED_ALPN`].
const SHARED_HELLO_ALPN: &[u8] = b"sapphire/hello/2";

/// How long a dial waits for this host to have an address before calling it offline.
///
/// A host with no address and no relay — asleep a moment ago, its interfaces not up yet —
/// can reach nobody, and every dial it started anyway would begin an address lookup whose
/// DNS queries wait out their timeouts. Waiting here instead costs nothing. Ten seconds is
/// what iroh itself gives a connection with no reachable address.
const OFFLINE_WAIT: Duration = Duration::from_secs(10);

/// How many accepted streams may wait for [`IrohTransport::accept`] before the connections
/// they arrive on are made to wait.
const INBOUND_QUEUE: usize = 64;

/// One dial to a peer on one ALPN, shared by every open that asked while it ran.
///
/// The cell is filled once, by the first open; the others wait on it. That is what makes a
/// burst of opens to the same peer — three app servers retrying at once, a sync sweep and
/// a Hello tick — one connection attempt and one address lookup rather than one each.
type Attempt = tokio::sync::OnceCell<Dialed>;

/// Which dial an [`Attempt`] is: the peer, and the ALPN offered first.
type DialKey = (EndpointId, &'static [u8]);

/// What one [`Attempt`] came to.
#[derive(Clone)]
enum Dialed {
    /// A connection both sides agreed to share, kept for the next open while it lives.
    Shared(::iroh::endpoint::Connection),
    /// A connection to a peer that only takes one stream per connection. It went to the
    /// open that dialed it; every other open dials its own, as before.
    Single,
    /// The dial failed, and every open that waited on it fails the same way.
    Failed(String),
}

/// This host's endpoint on the network.
pub struct IrohTransport {
    endpoint: Endpoint,
    node_id: String,
    /// Addresses learned out of band, so a peer can be dialed without a discovery service.
    known: MemoryLookup,
    /// The latest dial to each peer on each ALPN: the shared connection to open the next
    /// stream on, or the attempt still running. Keyed by the ALPN offered first.
    attempts: std::sync::Mutex<HashMap<DialKey, Arc<Attempt>>>,
    /// How many connections this transport has dialed. Logged with each new one, so a log
    /// shows whether the bridge is reusing them.
    connects: AtomicUsize,
    /// Streams the accept task has taken, in the order they became readable.
    inbound: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Inbound>>,
    /// The task taking connections and streams off the endpoint; stopped with the transport.
    acceptor: tokio::task::AbortHandle,
    /// [`OFFLINE_WAIT`], shortened by the tests that wait it out.
    offline_wait: Duration,
}

impl Drop for IrohTransport {
    fn drop(&mut self) {
        // The task holds a handle on the endpoint; left running, it would keep the endpoint
        // bound after the transport that owns it is gone.
        self.acceptor.abort();
    }
}

impl IrohTransport {
    /// Bind an endpoint, loading or creating the secret key at `key_path`.
    ///
    /// `net` decides what the endpoint offers and uses: `discovery` turns on iroh's address
    /// lookup services, and the relay set is the host's own configuration merged with
    /// `workgroup`'s published one — see [`relays`](crate::relays) for how the two files
    /// combine. A caller that has already merged them (a test, mostly) uses
    /// [`IrohTransport::new`] directly.
    pub async fn new_with_relays(
        key_path: &Path,
        net: &NetConfig,
        workgroup: Option<&crate::workgroup::Workgroup>,
    ) -> Result<IrohTransport> {
        let config = crate::relays(net, workgroup)?;
        Self::new(key_path, net, &config).await
    }

    /// Bind an endpoint over an already-resolved relay configuration.
    ///
    /// `net` decides what the endpoint offers and uses: `discovery` turns on iroh's address
    /// lookup services, and `relays` is taken as given — an empty list with
    /// `use_default: false` disables relays entirely, which is what a test or a fully local
    /// host wants.
    pub async fn new(
        key_path: &Path,
        net: &NetConfig,
        relays: &RelayConfig,
    ) -> Result<IrohTransport> {
        Self::bind(key_path, net, relays, false).await
    }

    /// Bind an endpoint with no direct paths: every connection goes through a relay.
    ///
    /// Direct connections are what iroh prefers whenever it can find a path, so an endpoint
    /// built the ordinary way may reach a peer without the relay ever carrying a byte — which
    /// makes "does the relay work" untestable. Removing the IP transports leaves the relay as
    /// the only path, and a stream that arrives proves the relay carried it.
    ///
    /// `relays` must name at least one relay with `use_default: false`: an endpoint with
    /// neither a direct path nor a relay can reach nobody.
    pub async fn new_relay_only(
        key_path: &Path,
        net: &NetConfig,
        relays: &RelayConfig,
    ) -> Result<IrohTransport> {
        Self::bind(key_path, net, relays, true).await
    }

    /// Bind an endpoint, `relay_only` deciding whether direct paths are offered.
    async fn bind(
        key_path: &Path,
        net: &NetConfig,
        relays: &RelayConfig,
        relay_only: bool,
    ) -> Result<IrohTransport> {
        let secret = load_or_create_key(key_path)?;
        let known = MemoryLookup::new();

        // `Minimal` rather than `N0`: it picks the crypto provider, and nothing else. Every
        // other thing `N0` turns on is what `net` is here to decide.
        // Three protocols on one endpoint: the data plane and, each on its own ALPN, pairing and hello.
        // They are listed together because iroh accepts per endpoint; which one a
        // connection spoke is reported by `accept`, and the two are gated differently.
        // The acceptor's order decides which name a connection speaks, so each shared name
        // comes before the one-stream name it extends (see `SHARED_ALPN`).
        let alpns = vec![
            SHARED_ALPN.to_vec(),
            ALPN.to_vec(),
            PAIR_ALPN.to_vec(),
            SHARED_HELLO_ALPN.to_vec(),
            HELLO_ALPN.to_vec(),
        ];
        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .alpns(alpns)
            .address_lookup(known.clone());
        if net.discovery {
            builder = builder
                .address_lookup(PkarrPublisher::n0_dns())
                .address_lookup(PkarrResolver::n0_dns())
                .address_lookup(DnsAddressLookup::n0_dns());
        }

        if relay_only {
            // No IP transport is registered at all, so there is no direct path to find or
            // fall back to: the relay is the only way out, and the only way in.
            builder = builder.clear_ip_transports();
        }
        let endpoint = builder
            .relay_mode(relay_mode(relays)?)
            .bind()
            .await
            .map_err(|e| Error::Peer(format!("could not bind the endpoint: {e}")))?;
        let node_id = endpoint.id().to_string();
        let (inbound_tx, inbound_rx) = tokio::sync::mpsc::channel(INBOUND_QUEUE);
        let acceptor =
            tokio::spawn(accept_connections(endpoint.clone(), inbound_tx)).abort_handle();

        Ok(IrohTransport {
            endpoint,
            node_id,
            known,
            attempts: std::sync::Mutex::default(),
            connects: AtomicUsize::new(0),
            inbound: tokio::sync::Mutex::new(inbound_rx),
            acceptor,
            offline_wait: OFFLINE_WAIT,
        })
    }

    /// This device's node id and addresses, for handing to a peer out of band.
    ///
    /// Fails while the endpoint has nothing to share: an address with no transports on it
    /// would tell a peer to dial a device it cannot reach, and a pairing flow that shared one
    /// would be silently useless.
    pub async fn node_addr(&self) -> Result<NodeAddr> {
        let addr = self.endpoint.addr();
        if addr.is_empty() {
            return Err(Error::Peer(
                "this endpoint has no address to share yet".to_owned(),
            ));
        }
        Ok(NodeAddr {
            node_id: addr.id.to_string(),
            addrs: addr.ip_addrs().map(|addr| addr.to_string()).collect(),
            relay_urls: addr.relay_urls().map(|url| url.to_string()).collect(),
        })
    }

    /// Teach this endpoint how to reach `addr`, which is how a peer with discovery off is
    /// dialed.
    pub fn add_known_address(&self, addr: &NodeAddr) -> Result<()> {
        let id: EndpointId = addr
            .node_id
            .parse()
            .map_err(|e| Error::Config(format!("{}: not a node id: {e}", addr.node_id)))?;
        let mut transports = Vec::with_capacity(addr.addrs.len());
        for address in &addr.addrs {
            let socket: SocketAddr = address
                .parse()
                .map_err(|e| Error::Config(format!("{address}: not a socket address: {e}")))?;
            transports.push(TransportAddr::Ip(socket));
        }
        for url in &addr.relay_urls {
            let url: RelayUrl = url
                .parse()
                .map_err(|e| Error::Config(format!("{url}: not a relay URL: {e}")))?;
            transports.push(TransportAddr::Relay(url));
        }
        self.known
            .add_endpoint_info(EndpointAddr::from_parts(id, transports));
        Ok(())
    }
}

#[async_trait::async_trait]
impl PeerTransport for IrohTransport {
    async fn open(&self, node_id: &str, workspace_id: GrainId) -> Result<BoxedStream> {
        let id = self.endpoint_id(node_id)?;
        let (mut send, recv) = self.open_bi(id, SHARED_ALPN, ALPN).await?;

        // The request travels ahead of the payload, so the far side knows what was asked for
        // without looking inside the stream. Same framing as the data plane's header: one
        // JSON object and a newline.
        let request = StreamRequest {
            workspace_id,
            node_id: self.node_id.clone(),
        };
        let mut line = serde_json::to_vec(&request).map_err(|e| Error::Protocol(e.to_string()))?;
        line.push(b'\n');
        send.write_all(&line)
            .await
            .map_err(|e| Error::Peer(format!("could not send the request to {node_id}: {e}")))?;
        send.flush().await.map_err(|e| Error::Peer(e.to_string()))?;

        // The two halves keep the connection alive between them, so the caller may keep this
        // for as long as it likes.
        Ok(Box::new(tokio::io::join(recv, send)))
    }

    async fn open_pairing(&self, node_addr: &[u8]) -> Result<BoxedStream> {
        let addr: ::iroh::EndpointAddr = postcard::from_bytes(node_addr)
            .map_err(|e| Error::Peer(format!("the ticket's address is unreadable: {e}")))?;
        let conn = self
            .endpoint
            .connect(addr, PAIR_ALPN)
            .await
            .map_err(|e| Error::Peer(format!("could not reach the inviter: {e}")))?;
        let (send, recv) = conn
            .open_bi()
            .await
            .map_err(|e| Error::Peer(format!("could not open a pairing stream: {e}")))?;
        Ok(Box::new(tokio::io::join(recv, send)))
    }

    async fn open_hello(&self, node_id: &str) -> Result<BoxedStream> {
        let id = self.endpoint_id(node_id)?;
        let (send, recv) = self.open_bi(id, SHARED_HELLO_ALPN, HELLO_ALPN).await?;
        Ok(Box::new(tokio::io::join(recv, send)))
    }

    async fn accept(&self) -> Result<Inbound> {
        // The accept task takes connections and streams as they come, each on its own task,
        // so one slow handshake or one request line that never arrives holds up nobody else.
        // What is left here is the queue between that task and the bridge's loop.
        self.inbound
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| Error::Peer("the endpoint is closed".to_owned()))
    }

    fn node_id(&self) -> String {
        self.node_id.clone()
    }

    fn is_connected(&self, node_id: &str) -> bool {
        // Whether a peer is reachable is the carrier's fact, and iroh's answer is the
        // paths it is *actively* using to that endpoint right now. `remote_info` is a
        // snapshot kept for a while after a peer goes away, so an `Inactive` entry alone
        // proves nothing: only an active path — a QUIC path in use by at least one live
        // connection, or the relay the endpoint is talking through — counts as connected.
        // A peer that has never spoken to this host has no entry at all, and is not
        // connected, which is also what the default says.
        let Ok(id) = self.endpoint_id(node_id) else {
            return false;
        };
        // The trait method is sync, and `remote_info` is async, but it is answered
        // directly from the endpoint's actor state — no network round trip — so a
        // blocking wait for the actor's one-shot is what "ask iroh" means here.
        match self.info_of(id) {
            None => false,
            Some(info) => info
                .addrs()
                .any(|a| matches!(a.usage(), ::iroh::endpoint::TransportAddrUsage::Active)),
        }
    }

    fn ticket_addr(&self) -> Result<Vec<u8>> {
        // The whole `EndpointAddr`, not just the id: a ticket that named no address would
        // send the joiner to a discovery service it may not have (relays and discovery are
        // both configurable off), and pairing must work on a host with neither.
        let addr = self.endpoint.addr();
        if addr.is_empty() {
            return Err(Error::Peer(
                "this endpoint has no address to put in a ticket yet".to_owned(),
            ));
        }
        postcard::to_stdvec(&addr)
            .map_err(|e| Error::Peer(format!("could not encode this host's address: {e}")))
    }

    async fn close(&self) {
        // Gracefully: peers are told the connections are going, and the socket is free
        // when this returns, for the endpoint a restarted bridge binds next.
        self.endpoint.close().await;
    }
}

impl IrohTransport {
    /// Open a stream to `id` on `shared_alpn`, or on `single_alpn` with a peer that only
    /// knows that one.
    ///
    /// The stream goes on the connection already open to `id` when there is one. A stream
    /// that cannot be opened on it means the connection died without this host hearing yet,
    /// so the connection is forgotten and the stream tried once more on a fresh one.
    async fn open_bi(
        &self,
        id: EndpointId,
        shared_alpn: &'static [u8],
        single_alpn: &'static [u8],
    ) -> Result<(::iroh::endpoint::SendStream, ::iroh::endpoint::RecvStream)> {
        let mut retried = false;
        loop {
            let (conn, reused) = self.connection(id, shared_alpn, single_alpn).await?;
            match conn.open_bi().await {
                Ok(halves) => return Ok(halves),
                Err(err) if reused && !retried => {
                    tracing::debug!(peer = %id, "a kept connection could not open a stream: {err}");
                    self.forget(id, shared_alpn, &conn);
                    retried = true;
                }
                Err(err) => {
                    return Err(Error::Peer(format!(
                        "could not open a stream to {id}: {err}"
                    )));
                }
            }
        }
    }

    /// A connection to `id`, and whether it is one an earlier open made.
    ///
    /// The shared connection when there is a live one; otherwise the current attempt, joined
    /// when another open has started it, started here when none has.
    async fn connection(
        &self,
        id: EndpointId,
        shared_alpn: &'static [u8],
        single_alpn: &'static [u8],
    ) -> Result<(::iroh::endpoint::Connection, bool)> {
        let key = (id, shared_alpn);
        let attempt = {
            let mut attempts = self.attempts.lock().expect("dial attempts");
            match attempts.get(&key) {
                // Still running, or a connection that is still up: join it.
                Some(attempt)
                    if match attempt.get() {
                        None => true,
                        Some(Dialed::Shared(conn)) => conn.close_reason().is_none(),
                        Some(Dialed::Single | Dialed::Failed(_)) => false,
                    } =>
                {
                    Arc::clone(attempt)
                }
                // Nothing yet, or an attempt that is over: start the next one.
                _ => {
                    let attempt = Arc::new(Attempt::new());
                    attempts.insert(key, Arc::clone(&attempt));
                    attempt
                }
            }
        };

        // The connection this open dialed itself, when the peer would not share it.
        let mut mine = None;
        let mut started_here = false;
        let dialed = attempt
            .get_or_init(|| async {
                started_here = true;
                match self.dial(id, shared_alpn, single_alpn).await {
                    Ok(conn) if conn.alpn() == shared_alpn => Dialed::Shared(conn),
                    Ok(conn) => {
                        mine = Some(conn);
                        Dialed::Single
                    }
                    Err(err) => Dialed::Failed(err.to_string()),
                }
            })
            .await
            .clone();
        match dialed {
            Dialed::Shared(conn) => Ok((conn, !started_here)),
            Dialed::Failed(message) => Err(Error::Peer(message)),
            // A peer that takes one stream per connection gets a connection per open, as
            // it always did.
            Dialed::Single => match mine {
                Some(conn) => Ok((conn, false)),
                None => Ok((self.dial(id, shared_alpn, single_alpn).await?, false)),
            },
        }
    }

    /// Drop `conn` from the cache, if it is still the one kept for `id`.
    fn forget(
        &self,
        id: EndpointId,
        shared_alpn: &'static [u8],
        conn: &::iroh::endpoint::Connection,
    ) {
        let mut attempts = self.attempts.lock().expect("dial attempts");
        let key = (id, shared_alpn);
        if let Some(attempt) = attempts.get(&key)
            && let Some(Dialed::Shared(kept)) = attempt.get()
            && kept.stable_id() == conn.stable_id()
        {
            attempts.remove(&key);
        }
    }

    /// Dial `id`, offering `shared_alpn` and falling back to `single_alpn`.
    async fn dial(
        &self,
        id: EndpointId,
        shared_alpn: &'static [u8],
        single_alpn: &'static [u8],
    ) -> Result<::iroh::endpoint::Connection> {
        self.wait_until_reachable().await?;
        let options = ::iroh::endpoint::ConnectOptions::new()
            .with_additional_alpns(vec![single_alpn.to_vec()]);
        let conn = async {
            self.endpoint
                .connect_with_opts(id, shared_alpn, options)
                .await
                .map_err(|e| e.to_string())?
                .await
                .map_err(|e| e.to_string())
        }
        .await
        .map_err(|e| Error::Peer(format!("could not reach {id}: {e}")))?;
        let count = self.connects.fetch_add(1, Ordering::Relaxed) + 1;
        tracing::debug!(
            peer = %id,
            alpn = %String::from_utf8_lossy(conn.alpn()),
            connects = count,
            "dialed a peer"
        );
        Ok(conn)
    }

    /// Wait, up to [`OFFLINE_WAIT`], for this host to have an address or a relay.
    ///
    /// iroh's own view of the host's addresses is what decides: with no interface address
    /// and no relay connection there is no path to anybody, and a dial started anyway would
    /// only spend an address lookup — and the DNS queries behind it — on finding that out.
    /// The wait itself sends nothing.
    async fn wait_until_reachable(&self) -> Result<()> {
        use ::iroh::Watcher as _;

        if !self.endpoint.addr().is_empty() {
            return Ok(());
        }
        let mut addr = self.endpoint.watch_addr();
        let reachable = tokio::time::timeout(self.offline_wait, async {
            loop {
                match addr.updated().await {
                    Ok(now) if !now.is_empty() => return true,
                    Ok(_) => continue,
                    // The endpoint closed: nothing will change any more.
                    Err(_) => return false,
                }
            }
        })
        .await
        .unwrap_or(false);
        if reachable {
            Ok(())
        } else {
            Err(Error::Peer(
                "this host is offline: it has no network address and no relay".to_owned(),
            ))
        }
    }

    /// The endpoint id behind `node_id`, if it is one.
    fn endpoint_id(&self, node_id: &str) -> Result<::iroh::EndpointId> {
        node_id
            .parse()
            .map_err(|e| Error::Peer(format!("{node_id}: not a node id: {e}")))
    }

    /// What iroh currently knows about `id`, fetched synchronously from its actor.
    ///
    /// `remote_info` is async because it asks the remote-state actor over a oneshot; the
    /// answer is ready immediately — the actor holds it in memory — so a short block on
    /// the calling thread is the cheapest way to serve a sync trait method. How that
    /// block is done depends on the runtime this call runs on: a multi-thread runtime's
    /// worker parks its task with [`tokio::task::block_in_place`], a current-thread
    /// runtime cannot do that (the answer would never be driven), so its handle's own
    /// `block_on` runs the future to completion, and a call from outside any runtime
    /// builds a throwaway one. The one-shot is answered as soon as the actor is
    /// scheduled, so every path here returns in microseconds; none of them waits on the
    /// network.
    fn info_of(&self, id: ::iroh::EndpointId) -> Option<::iroh::endpoint::RemoteInfo> {
        let ask = async { self.endpoint.remote_info(id).await };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => match handle.runtime_flavor() {
                tokio::runtime::RuntimeFlavor::MultiThread => {
                    tokio::task::block_in_place(|| handle.block_on(ask))
                }
                // `RuntimeFlavor` is `#[non_exhaustive]`; anything that is not the
                // multi-thread runtime treats like the current-thread one.
                _ => handle.block_on(ask),
            },
            Err(_) => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .ok()?;
                rt.block_on(ask)
            }
        }
    }
}

/// Take every connection that arrives at `endpoint`, until it closes, each on its own task.
///
/// Any host on the network may send a packet here, so a failed handshake must not end the
/// loop: it is one caller going away, not this bridge stopping.
async fn accept_connections(endpoint: Endpoint, inbound: tokio::sync::mpsc::Sender<Inbound>) {
    while let Some(incoming) = endpoint.accept().await {
        let inbound = inbound.clone();
        tokio::spawn(async move {
            let accepting = match incoming.accept() {
                Ok(accepting) => accepting,
                Err(err) => {
                    tracing::debug!("a peer could not be accepted: {err}");
                    return;
                }
            };
            accept_streams(accepting, inbound).await;
        });
    }
}

/// Finish one connection's handshake, then take every stream the peer opens on it.
///
/// A peer that dialed one of the shared names keeps its connection and opens the next
/// stream on it; one that dialed a one-stream name opens a single stream and lets the
/// connection go, which ends this loop the same way. Each stream is read on its own task,
/// so a request line that is slow to come does not hold up the streams behind it.
async fn accept_streams(
    mut accepting: ::iroh::endpoint::Accepting,
    inbound: tokio::sync::mpsc::Sender<Inbound>,
) {
    // Which ALPN the caller dialed decides what may arrive on the stream: a workspace
    // request on the pairing ALPN, or a pairing exchange on the data ALPN, is a peer that is
    // not speaking this protocol. Reading the ALPN first is also what keeps a pairing
    // connection out of the code that reads a workspace request line.
    let alpn = match accepting.alpn().await {
        Ok(alpn) => alpn,
        Err(err) => {
            tracing::debug!("a peer's protocol could not be read: {err}");
            return;
        }
    };
    let protocol = match alpn.as_slice() {
        a if a == SHARED_ALPN || a == ALPN => Protocol::Workspace,
        a if a == SHARED_HELLO_ALPN || a == HELLO_ALPN => Protocol::Hello,
        a if a == PAIR_ALPN => Protocol::Pairing,
        _ => {
            tracing::debug!("a peer dialed a protocol this host does not speak");
            return;
        }
    };
    let conn = match accepting.await {
        Ok(conn) => conn,
        Err(err) => {
            tracing::debug!("a peer's handshake failed: {err}");
            return;
        }
    };
    let from = conn.remote_id();
    loop {
        let (send, recv) = match conn.accept_bi().await {
            Ok(halves) => halves,
            // The peer closed the connection, or it timed out: either way there is nothing
            // more to take from it.
            Err(err) => {
                tracing::debug!(peer = %from, "a peer's connection ended: {err}");
                return;
            }
        };
        let conn = conn.clone();
        let inbound = inbound.clone();
        tokio::spawn(async move {
            if let Some(stream) = accept_stream(protocol, from, conn, send, recv).await {
                // The receiver is gone only when the transport is, and then so is the
                // bridge that would have answered.
                let _ = inbound.send(stream).await;
            }
        });
    }
}

/// The protocol a connection was accepted for, from the ALPN it negotiated.
#[derive(Clone, Copy)]
enum Protocol {
    Workspace,
    Hello,
    Pairing,
}

/// Turn one accepted stream into what [`IrohTransport::accept`] reports, or `None` for a
/// stream that is not speaking the protocol.
async fn accept_stream(
    protocol: Protocol,
    from: EndpointId,
    conn: ::iroh::endpoint::Connection,
    send: ::iroh::endpoint::SendStream,
    recv: ::iroh::endpoint::RecvStream,
) -> Option<Inbound> {
    match protocol {
        // The pairing gate is the invite secret, not the ledger, and the ledger lookup
        // `authorize` runs would fail every joiner by definition. This is the one
        // connection that arrives before membership exists.
        Protocol::Pairing => Some(Inbound::Pairing(
            from.to_string(),
            Box::new(AcceptedStream {
                io: tokio::io::join(recv, send),
                conn,
            }),
        )),
        Protocol::Hello => Some(Inbound::Hello(
            from.to_string(),
            Box::new(AcceptedStream {
                io: tokio::io::join(recv, send),
                conn,
            }),
        )),
        Protocol::Workspace => {
            let (request, recv) = match read_request(recv).await {
                Ok(read) => read,
                Err(err) => {
                    tracing::debug!(peer = %from, "a peer sent an unreadable request: {err}");
                    return None;
                }
            };
            if request.node_id != from.to_string() {
                // Reported, not enforced: this transport reports who called, and the bridge
                // decides what a disagreement means. The name below is the authenticated one,
                // because that is the identity the far side cannot forge.
                tracing::debug!(
                    authenticated = %from,
                    claimed = %request.node_id,
                    "a peer's request named a different node id than the one that called"
                );
            }
            Some(Inbound::Workspace(
                from.to_string(),
                request.workspace_id,
                // `read_request` split the request line off `recv`; put the halves back
                // together for the caller, which sees one stream starting at the payload.
                Box::new(AcceptedStream {
                    io: tokio::io::join(recv, send),
                    conn,
                }),
            ))
        }
    }
}

/// Read the request line, leaving the stream positioned at the first byte of the payload.
async fn read_request(
    mut recv: ::iroh::endpoint::RecvStream,
) -> Result<(StreamRequest, ::iroh::endpoint::RecvStream)> {
    // Capacity 1: a larger buffer would read past the newline and swallow payload bytes the
    // caller is about to be handed.
    let mut reader = BufReader::with_capacity(1, &mut recv);
    let mut line = String::new();
    // `read_line` grows its buffer until it sees a newline, so it is bounded here rather than
    // checked afterwards: a peer that never sends one must not be able to make the bridge
    // allocate without limit. One byte over, so a line that fits exactly still reads.
    let read = (&mut reader)
        .take(MAX_REQUEST_LINE + 1)
        .read_line(&mut line)
        .await
        .map_err(|e| Error::Protocol(format!("could not read the request line: {e}")))?;
    if read == 0 {
        return Err(Error::Protocol("the peer sent no request line".to_owned()));
    }
    if read as u64 > MAX_REQUEST_LINE {
        return Err(Error::Protocol(format!(
            "the request line is longer than {MAX_REQUEST_LINE} bytes"
        )));
    }
    let request = serde_json::from_str(line.trim())
        .map_err(|e| Error::Protocol(format!("bad peer request: {e}")))?;
    Ok((request, recv))
}

/// The relay mode [`RelayConfig`] asks for.
///
/// A custom relay map **replaces** iroh's relay set rather than adding to it, so with
/// `use_default` on, the public relays are folded back in first. The map is keyed by URL,
/// which is what keeps this fold and the merge in [`relays`](crate::relays) from producing
/// duplicates. Naming no relay is not an error either: a host may want direct connections
/// only, and an endpoint told "no relays" gets [`RelayMode::Disabled`].
fn relay_mode(config: &RelayConfig) -> Result<RelayMode> {
    if !config.use_default && config.urls.is_empty() {
        return Ok(RelayMode::Disabled);
    }
    let map = if config.use_default {
        default_relay_map()
    } else {
        RelayMap::empty()
    };
    for url in &config.urls {
        let url: RelayUrl = url
            .parse()
            .map_err(|e| Error::Config(format!("{url}: not a relay URL: {e}")))?;
        map.insert(
            url.clone(),
            Arc::new(::iroh::RelayConfig::from(url.clone())),
        );
    }
    Ok(RelayMode::Custom(map))
}

/// Read the secret key, or create one at `0600`.
///
/// This file **is** the device's identity: losing it means rejoining every workgroup as a new
/// device, so it is created once and never regenerated on a parse failure.
fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    match std::fs::read(path) {
        Ok(bytes) => parse_key(path, &bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes)
                .map_err(|e| Error::Config(format!("no system random source: {e}")))?;
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)?;
            }
            match create_private(path, &bytes)? {
                Created::Yes => Ok(SecretKey::from_bytes(&bytes)),
                // Another process won the race and wrote *its* key. Use that one, not the
                // bytes generated here: the file is the identity, and two processes that
                // disagreed about it would each think the other was a stranger.
                Created::AlreadyExisted => load_or_create_key(path),
            }
        }
        Err(e) => Err(Error::Io(e)),
    }
}

/// This host's node id, from the key at `key_path`, creating the key if it is absent.
///
/// Derived without binding an endpoint, which is what a one-shot command needs: founding a
/// workgroup records this host's node id, and that happens before — sometimes long before —
/// a bridge is running to ask.
pub fn load_or_create_node_id(key_path: &Path) -> Result<String> {
    Ok(load_or_create_key(key_path)?.public().to_string())
}

/// Interpret the bytes of a key file.
fn parse_key(path: &Path, bytes: &[u8]) -> Result<SecretKey> {
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
        Error::Config(format!(
            "{}: a node key is 32 bytes, found {}; move it aside rather than \
             letting a new identity be minted",
            path.display(),
            bytes.len()
        ))
    })?;
    Ok(SecretKey::from_bytes(&bytes))
}

/// Whether [`create_private`] wrote the file or found one already there.
#[derive(Debug, PartialEq, Eq)]
enum Created {
    Yes,
    AlreadyExisted,
}

/// Write a new key file that nobody else can read.
///
/// `create_new`, so a key that appeared between the read and here is never overwritten: the
/// device that already holds an identity keeps it.
fn create_private(path: &Path, bytes: &[u8; 32]) -> Result<Created> {
    use std::io::Write;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // The mode is masked by the umask, so it is set again below: the key must be private
        // whatever the user's umask is.
        options.mode(0o600);
    }

    match options.open(path) {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.flush()?;
        }
        // Another process created the identity first.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Ok(Created::AlreadyExisted);
        }
        Err(e) => return Err(Error::Io(e)),
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(Created::Yes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host that neither discovers peers nor uses a relay: local addresses only.
    fn offline() -> NetConfig {
        NetConfig {
            wake_on_sync: false,
            discovery: false,
            relays: vec![],
            use_default_relays: false,
            ..NetConfig::default()
        }
    }

    async fn transport(tmp: &tempfile::TempDir, name: &str) -> IrohTransport {
        let net = offline();
        IrohTransport::new(
            &tmp.path().join(name),
            &net,
            &crate::relays(&net, None).unwrap(),
        )
        .await
        .unwrap()
    }

    /// A dials B: two transports that know each other's local address.
    async fn pair(tmp: &tempfile::TempDir) -> (IrohTransport, Arc<IrohTransport>) {
        let a = transport(tmp, "a.key").await;
        let b = transport(tmp, "b.key").await;
        a.add_known_address(&b.node_addr().await.unwrap()).unwrap();
        (a, Arc::new(b))
    }

    /// Take `n` workspace streams at `b`, and read `len` bytes from each.
    fn receive(
        b: &Arc<IrohTransport>,
        n: usize,
        len: usize,
    ) -> tokio::task::JoinHandle<Vec<Vec<u8>>> {
        let b = Arc::clone(b);
        tokio::spawn(async move {
            let mut got = Vec::new();
            for _ in 0..n {
                let inbound = tokio::time::timeout(Duration::from_secs(10), b.accept())
                    .await
                    .expect("a stream in time")
                    .unwrap();
                let Inbound::Workspace(_, _, mut stream) = inbound else {
                    panic!("expected a workspace stream");
                };
                let mut buf = vec![0; len];
                stream.read_exact(&mut buf).await.unwrap();
                got.push(buf);
            }
            got
        })
    }

    async fn send(stream: &mut BoxedStream, bytes: &[u8]) {
        stream.write_all(bytes).await.unwrap();
        stream.flush().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_opens_to_a_peer_share_one_connection() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = pair(&tmp).await;
        let received = receive(&b, 2, 3);

        let mut first = a.open(&b.node_id(), GrainId::random()).await.unwrap();
        send(&mut first, b"one").await;
        let mut second = a.open(&b.node_id(), GrainId::random()).await.unwrap();
        send(&mut second, b"two").await;

        // Both streams arrive, each read on its own: the far side takes more than the
        // first stream on a connection.
        let mut got = received.await.unwrap();
        got.sort();
        assert_eq!(got, [b"one".to_vec(), b"two".to_vec()]);
        assert_eq!(
            a.connects.load(Ordering::Relaxed),
            1,
            "one connection, reused"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_opens_share_one_dial() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = pair(&tmp).await;
        let a = Arc::new(a);
        const OPENS: usize = 8;
        let received = receive(&b, OPENS, 1);

        let mut opens = tokio::task::JoinSet::new();
        for n in 0..OPENS {
            let (a, to) = (Arc::clone(&a), b.node_id());
            opens.spawn(async move {
                let mut stream = a.open(&to, GrainId::random()).await.unwrap();
                send(&mut stream, &[n as u8]).await;
                stream
            });
        }
        // Held until every byte is read, so no stream closes ahead of the read.
        let streams = opens.join_all().await;

        let mut got: Vec<u8> = received.await.unwrap().concat();
        got.sort();
        assert_eq!(got, (0..OPENS as u8).collect::<Vec<_>>());
        assert_eq!(
            a.connects.load(Ordering::Relaxed),
            1,
            "opens that arrive together wait on one dial"
        );
        drop(streams);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_closed_connection_is_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = pair(&tmp).await;
        let received = receive(&b, 2, 1);

        let mut first = a.open(&b.node_id(), GrainId::random()).await.unwrap();
        send(&mut first, b"1").await;
        let kept = {
            let attempts = a.attempts.lock().unwrap();
            let attempt = attempts.values().next().expect("an attempt");
            let Some(Dialed::Shared(conn)) = attempt.get() else {
                panic!("expected a shared connection");
            };
            conn.clone()
        };
        // Read before closing, so the close cannot overtake the byte.
        tokio::time::sleep(Duration::from_millis(200)).await;
        kept.close(0u32.into(), b"test");
        kept.closed().await;

        let mut second = a.open(&b.node_id(), GrainId::random()).await.unwrap();
        send(&mut second, b"2").await;
        assert_eq!(received.await.unwrap().concat(), b"12");
        assert_eq!(
            a.connects.load(Ordering::Relaxed),
            2,
            "a connection that closed is dialed again"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_peer_that_takes_one_stream_gets_a_connection_per_open() {
        // A bridge from before the shared names: one stream per connection, then it lets
        // the connection go. Reusing a connection with it would send a stream nobody reads.
        let tmp = tempfile::tempdir().unwrap();
        let a = transport(&tmp, "a.key").await;
        let old = Endpoint::builder(presets::Minimal)
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .unwrap();
        a.known.add_endpoint_info(old.addr());
        let serving = tokio::spawn({
            let old = old.clone();
            async move {
                let mut lines = Vec::new();
                for _ in 0..2 {
                    let conn = old.accept().await.unwrap().await.unwrap();
                    assert_eq!(conn.alpn(), ALPN, "the old name is the one agreed on");
                    let (_send, recv) = conn.accept_bi().await.unwrap();
                    let (request, _) = read_request(recv).await.unwrap();
                    lines.push(request.workspace_id);
                }
                lines
            }
        });

        let (one, two) = (GrainId::random(), GrainId::random());
        let first = a.open(&old.id().to_string(), one).await.unwrap();
        let second = a.open(&old.id().to_string(), two).await.unwrap();
        let served = tokio::time::timeout(Duration::from_secs(10), serving)
            .await
            .expect("both requests in time")
            .unwrap();
        assert_eq!(served, [one, two]);
        assert_eq!(a.connects.load(Ordering::Relaxed), 2);
        drop((first, second));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_host_with_no_address_is_offline() {
        // No IP transport, and a relay that never answers (the discard port on this host):
        // iroh has no address for this host at all, as after a wake with nothing up yet.
        let tmp = tempfile::tempdir().unwrap();
        let silent = crate::relay::RelayConfig {
            urls: vec!["http://127.0.0.1:9/".to_owned()],
            use_default: false,
        };
        let mut a = IrohTransport::new_relay_only(&tmp.path().join("a.key"), &offline(), &silent)
            .await
            .unwrap();
        a.offline_wait = Duration::from_millis(200);
        let b = transport(&tmp, "b.key").await;

        let err = match a.open(&b.node_id(), GrainId::random()).await {
            Ok(_) => panic!("an offline host must not reach anybody"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("offline"), "{err}");
        assert_eq!(
            a.connects.load(Ordering::Relaxed),
            0,
            "nothing is dialed while offline"
        );
    }

    #[test]
    fn no_relays_means_no_relay_transport() {
        let config = crate::relay::RelayConfig {
            urls: vec![],
            use_default: false,
        };
        assert!(matches!(relay_mode(&config).unwrap(), RelayMode::Disabled));
    }

    #[test]
    fn a_named_relay_is_used_and_a_bad_one_is_refused() {
        let config = crate::relay::RelayConfig {
            urls: vec!["https://relay.example/".to_owned()],
            use_default: false,
        };
        assert!(matches!(relay_mode(&config).unwrap(), RelayMode::Custom(_)));

        let config = crate::relay::RelayConfig {
            urls: vec!["not a url".to_owned()],
            use_default: false,
        };
        let err = relay_mode(&config).unwrap_err();
        assert!(err.to_string().contains("not a url"), "{err}");
    }

    #[test]
    fn the_public_relays_are_folded_back_into_a_custom_map() {
        // `RelayMode::Custom` replaces iroh's relay set rather than adding to it, so
        // `use_default` must fold the public relays in itself or they would be lost.
        let config = crate::relay::RelayConfig {
            urls: vec!["https://relay.example/".to_owned()],
            use_default: true,
        };
        let mode = relay_mode(&config).unwrap();
        let RelayMode::Custom(map) = mode else {
            panic!("expected a custom relay map");
        };
        let urls = map.urls::<Vec<::iroh::RelayUrl>>();
        assert!(urls.contains(&"https://relay.example/".parse().unwrap()));
        assert!(
            urls.len() > 1,
            "the public relays must be in the map too: {urls:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_node_id_follows_from_the_key_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("node.key");

        // Derived without an endpoint, and stable across calls.
        let id = load_or_create_node_id(&path).unwrap();
        assert_eq!(
            id.len(),
            64,
            "a node id is 64 lowercase hex characters: {id}"
        );
        assert_eq!(load_or_create_node_id(&path).unwrap(), id);

        // The same identity the transport binds.
        let net = NetConfig {
            wake_on_sync: false,
            discovery: false,
            relays: vec![],
            use_default_relays: false,
            ..NetConfig::default()
        };
        let transport = IrohTransport::new(&path, &net, &crate::relays(&net, None).unwrap())
            .await
            .unwrap();
        assert_eq!(transport.node_id(), id);
    }

    #[test]
    fn a_key_of_the_wrong_length_is_not_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("node.key");
        std::fs::write(&path, b"short").unwrap();

        let err = load_or_create_key(&path).unwrap_err();
        assert!(err.to_string().contains("32 bytes"), "{err}");
        // The bytes are still there: a damaged key is a human decision, not a fresh identity.
        assert_eq!(std::fs::read(&path).unwrap(), b"short");
    }

    #[test]
    fn an_existing_key_is_never_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("node.key");

        let first = load_or_create_key(&path).unwrap();
        assert_eq!(
            create_private(&path, &[7u8; 32]).unwrap(),
            Created::AlreadyExisted
        );

        // Loading again gives the identity that is on disk, not the one just offered.
        let second = load_or_create_key(&path).unwrap();
        assert_eq!(second.to_bytes(), first.to_bytes());
        assert_eq!(std::fs::read(&path).unwrap(), first.to_bytes());
    }
}
