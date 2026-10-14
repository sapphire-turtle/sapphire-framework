//! A bridge that records what it was told, for testing `SyncRuntime` without a real one.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sapphire_bridge_api::{
    Ack, BRIDGE_DATA_NAME, GrainId, IncomingParams, PeerInfo, PeersResult, RegisterParams,
    RegisterResult, StatusResult, UnregisterParams, WORKSPACES, WorkgroupWorkspaceInfo,
    WorkspacesResult,
};
use sapphire_bridge_client::BridgeClient;
use sapphire_ipc::{ClientInfo, Connection, Endpoint, ManagedBy, Router, ServerInfo, serve};

/// What a `StubBridge` saw.
#[derive(Clone, Debug, Default)]
pub struct Seen {
    /// Every `bridge.register` it received, in order.
    pub registrations: Vec<RegisterParams>,
    /// Every `bridge.unregister` it received.
    pub unregistrations: Vec<GrainId>,
}

/// A stand-in for the bridge's control plane.
pub struct StubBridge {
    /// What it has been told.
    pub seen: Arc<Mutex<Seen>>,
    /// The device id it answers registrations with.
    pub device_id: GrainId,
    /// The workgroup id it answers registrations with.
    pub workgroup_id: GrainId,
    /// The workspaces it answers `bridge.workspaces` with.
    pub listed: Arc<Mutex<Vec<WorkgroupWorkspaceInfo>>>,
    /// The devices it answers `bridge.peers` with.
    peers: Arc<Mutex<Vec<PeerInfo>>>,
    /// Where the client looks for the bridge's data endpoint.
    runtime_dir: PathBuf,
    /// How many `bridge.peers` it has been asked for.
    ///
    /// Dials and status reports both ask; a test that watches this grow after touching a
    /// file has proof the runtime was driven, without reaching into a replica.
    peers_queries: Arc<AtomicUsize>,
    /// The server's end of the connection, for announcements; `None` once disconnected,
    /// since a sender left alive would hold the connection open.
    announcer: Mutex<Option<sapphire_ipc::Sender>>,
    /// The task serving the control connection; aborting it is the bridge going away.
    serving: tokio::task::JoinHandle<()>,
}

impl StubBridge {
    /// Start a stub on one end of an in-process connection and return a client for the other.
    pub async fn start() -> (StubBridge, Arc<BridgeClient>) {
        StubBridge::start_in(std::env::temp_dir()).await
    }

    /// [`start`](Self::start), with the client looking for the data endpoint in
    /// `runtime_dir`: for a test that serves one there, with [`hang_streams`].
    ///
    /// [`hang_streams`]: Self::hang_streams
    pub async fn start_in(runtime_dir: PathBuf) -> (StubBridge, Arc<BridgeClient>) {
        let (stub, client) = StubBridge::start_client_in(runtime_dir).await;
        (stub, Arc::new(client))
    }

    /// [`start_in`](Self::start_in), handing the client over as it is: for a
    /// [`BridgeConnector`](super::BridgeConnector) to return.
    pub async fn start_client_in(runtime_dir: PathBuf) -> (StubBridge, BridgeClient) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let device_id = GrainId::random();
        let workgroup_id = GrainId::random();
        let peers_queries = Arc::new(AtomicUsize::new(0));
        let listed = Arc::new(Mutex::new(Vec::new()));
        let peers = Arc::new(Mutex::new(Vec::new()));

        let (client_conn, server_conn) = Connection::pair();
        let announcer = server_conn.sender();

        let router = Arc::new(
            Router::new()
                .method(sapphire_bridge_api::REGISTER, {
                    let seen = Arc::clone(&seen);
                    move |ctx| {
                        let seen = Arc::clone(&seen);
                        async move {
                            let params: RegisterParams = serde_json::from_value(ctx.params)
                                .map_err(|e| {
                                    sapphire_ipc::RpcError::invalid_params(e.to_string())
                                })?;
                            seen.lock().expect("stub").registrations.push(params);
                            serde_json::to_value(RegisterResult {
                                device_id,
                                node_id: "stub".into(),
                                workgroup_id,
                            })
                            .map_err(|e| sapphire_ipc::RpcError::internal(e.to_string()))
                        }
                    }
                })
                .method(sapphire_bridge_api::UNREGISTER, {
                    let seen = Arc::clone(&seen);
                    move |ctx| {
                        let seen = Arc::clone(&seen);
                        async move {
                            let params: UnregisterParams = serde_json::from_value(ctx.params)
                                .map_err(|e| {
                                    sapphire_ipc::RpcError::invalid_params(e.to_string())
                                })?;
                            seen.lock()
                                .expect("stub")
                                .unregistrations
                                .push(params.workspace_id);
                            serde_json::to_value(Ack {})
                                .map_err(|e| sapphire_ipc::RpcError::internal(e.to_string()))
                        }
                    }
                })
                .method(sapphire_bridge_api::PEERS, {
                    let peers_queries = Arc::clone(&peers_queries);
                    let peers = Arc::clone(&peers);
                    move |_| {
                        let peers_queries = Arc::clone(&peers_queries);
                        let peers = peers.lock().expect("stub").clone();
                        async move {
                            peers_queries.fetch_add(1, Ordering::Relaxed);
                            serde_json::to_value(PeersResult {
                                peers,
                                roles: vec![],
                            })
                            .map_err(|e| sapphire_ipc::RpcError::internal(e.to_string()))
                        }
                    }
                })
                .method(WORKSPACES, {
                    let listed = Arc::clone(&listed);
                    move |_| {
                        let listed = Arc::clone(&listed);
                        async move {
                            serde_json::to_value(WorkspacesResult {
                                workspaces: listed.lock().expect("stub").clone(),
                            })
                            .map_err(|e| sapphire_ipc::RpcError::internal(e.to_string()))
                        }
                    }
                })
                .method(sapphire_bridge_api::STATUS, |_| async move {
                    serde_json::to_value(StatusResult {
                        version: "stub".into(),
                        node_id: "stub".into(),
                        workgroup: None,
                        routes: vec![],
                        embedding: None,
                    })
                    .map_err(|e| sapphire_ipc::RpcError::internal(e.to_string()))
                }),
        );

        let serving = tokio::spawn(async move {
            let info = ServerInfo {
                version: "stub".into(),
                api: sapphire_bridge_api::API_VERSION,
                app_api: None,
                pid: std::process::id(),
                managed_by: ManagedBy::Service,
            };
            let _ = serve(server_conn, router, "bridge", info).await;
        });

        let info = ClientInfo {
            kind: "test".into(),
            version: "stub".into(),
            api: sapphire_bridge_api::API_VERSION,
            app_api: None,
            pid: std::process::id(),
        };
        let (client, _) = sapphire_ipc::Client::handshake(client_conn, "bridge", info)
            .await
            .expect("the stub handshake");
        let client = BridgeClient::from_client(Arc::new(client), runtime_dir.clone());

        (
            StubBridge {
                seen,
                listed,
                peers,
                runtime_dir,
                device_id,
                workgroup_id,
                peers_queries,
                announcer: Mutex::new(Some(announcer)),
                serving,
            },
            client,
        )
    }

    /// Go away, as a bridge that stops does: the client's connection closes.
    pub fn disconnect(&self) {
        self.serving.abort();
        self.announcer.lock().expect("stub").take();
    }

    /// Pretend a peer wants `workspace_id`.
    pub async fn announce(&self, workspace_id: GrainId, ticket: &str) {
        let params = IncomingParams {
            workspace_id,
            peer_device_id: GrainId::random(),
            ticket: ticket.to_owned(),
        };
        let Some(announcer) = self.announcer.lock().expect("stub").clone() else {
            return;
        };
        let _ = announcer
            .send(sapphire_ipc::Message::Notification(
                sapphire_ipc::Notification {
                    method: sapphire_bridge_api::INCOMING.to_owned(),
                    params: serde_json::to_value(params).expect("announcement"),
                },
            ))
            .await;
    }

    /// How many `bridge.peers` calls it has answered.
    pub fn peers_queries(&self) -> usize {
        self.peers_queries.load(Ordering::Relaxed)
    }

    /// Answer `bridge.peers` with `peers` from now on.
    pub fn set_peers(&self, peers: Vec<PeerInfo>) {
        *self.peers.lock().expect("stub") = peers;
    }

    /// Serve the data endpoint as a bridge does whose peer is offline: take every stream
    /// and never answer it, the way the bridge waits on iroh's connect to a node that is
    /// not there. Streams stay open until the returned task is aborted.
    pub async fn hang_streams(&self) -> tokio::task::JoinHandle<()> {
        std::fs::create_dir_all(&self.runtime_dir).expect("the runtime dir");
        let endpoint = Endpoint::in_dir(BRIDGE_DATA_NAME, self.runtime_dir.clone());
        #[cfg(unix)]
        let listener = sapphire_ipc::bind(&endpoint)
            .await
            .expect("the data endpoint");
        #[cfg(windows)]
        let mut listener = sapphire_ipc::bind(&endpoint).expect("the data endpoint");
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok(stream) = listener.accept_raw().await {
                held.push(stream);
            }
        })
    }

    /// Publish `info` as a workspace the workgroup knows about.
    pub fn list(&self, info: WorkgroupWorkspaceInfo) {
        self.listed.lock().expect("stub").push(info);
    }

    /// The last registration's workspace list.
    pub fn last_workspaces(&self) -> Vec<GrainId> {
        self.seen
            .lock()
            .expect("stub")
            .registrations
            .last()
            .map(|r| r.workspaces.iter().map(|w| w.workspace_id).collect())
            .unwrap_or_default()
    }
}
