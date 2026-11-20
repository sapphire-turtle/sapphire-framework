//! Hearing the app server's `workspace.event`s: the deliveries the status bar names.

use std::sync::{Arc, Mutex};

use sapphire_backend::BackendEvent;
use sapphire_backend::protocol as proto;
use sapphire_ipc::Client;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::delivery::Delivered;

/// Follows one app connection's events at a time, keeping the latest delivery heard.
pub(crate) struct Listener {
    /// The latest `SyncDelivered`, shared with [`FrameworkClient`](super::FrameworkClient).
    last: Arc<Mutex<Option<Delivered>>>,
    /// Woken on each delivery, so the snapshot refreshes at once.
    wake: mpsc::UnboundedSender<()>,
    /// The connection subscribed to, and the task reading it.
    following: Option<(Arc<Client>, tokio::task::JoinHandle<()>)>,
}

impl Listener {
    pub(crate) fn new(
        last: Arc<Mutex<Option<Delivered>>>,
        wake: mpsc::UnboundedSender<()>,
    ) -> Listener {
        Listener {
            last,
            wake,
            following: None,
        }
    }

    /// Follow `client`, the app connection as of this refresh: subscribe when it is not the
    /// one already followed. A server without `workspace.subscribe` is let be.
    pub(crate) async fn follow(&mut self, client: Option<Arc<Client>>) {
        let Some(client) = client else {
            // No connection now; the next one is new, whichever it is.
            if let Some((_, task)) = self.following.take() {
                task.abort();
            }
            return;
        };
        if self
            .following
            .as_ref()
            .is_some_and(|(followed, _)| Arc::ptr_eq(followed, &client))
        {
            return;
        }
        if let Some((_, task)) = self.following.take() {
            task.abort();
        }
        // Taken before subscribing, so an event sent at once is not missed.
        let mut notifications = client.notifications();
        // An older server does not know the method: nothing is heard, the poll still says
        // whether an edit is pending. Remembered all the same, so it is not asked again.
        let _ = client
            .call::<_, proto::Ack>(proto::SUBSCRIBE, serde_json::json!({}))
            .await;
        let last = Arc::clone(&self.last);
        let wake = self.wake.clone();
        let task = tokio::spawn(async move {
            loop {
                let notification = match notifications.recv().await {
                    Ok(n) => n,
                    // Missed ones are missed: the poll catches up on whether one is pending.
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return,
                };
                if notification.method != proto::EVENT {
                    continue;
                }
                let Ok(params) = serde_json::from_value::<proto::EventParams>(notification.params)
                else {
                    continue;
                };
                if let BackendEvent::SyncDelivered { device_name, .. } = params.event {
                    *last.lock().unwrap_or_else(|e| e.into_inner()) = Some(Delivered {
                        root: params.ws,
                        device_name,
                        at: jiff::Timestamp::now(),
                    });
                    let _ = wake.send(());
                }
            }
        });
        self.following = Some((client, task));
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Some((_, task)) = self.following.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use sapphire_backend::protocol::{Ack, EventParams};
    use sapphire_ipc::{ClientInfo, Connection, ManagedBy, Router, ServerInfo};

    /// A client of an app server that, when subscribed (and `subscribes`), sends one
    /// `SyncDelivered` from `peer` for `/ws`.
    async fn server(subscribes: bool, peer: &'static str) -> Arc<Client> {
        let mut router = Router::new();
        if subscribes {
            router = router.method(proto::SUBSCRIBE, move |ctx| async move {
                let params = EventParams {
                    ws: "/ws".into(),
                    event: BackendEvent::SyncDelivered {
                        device_id: grain_id::GrainId::random(),
                        device_name: peer.into(),
                    },
                };
                let peer = ctx.peer.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    let _ = peer
                        .notify(proto::EVENT, serde_json::to_value(params).unwrap())
                        .await;
                });
                Ok(serde_json::to_value(Ack {}).unwrap())
            });
        }
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
        Arc::new(client)
    }

    /// Wait up to 5 s for `last` to name `peer`.
    async fn heard(last: &Mutex<Option<Delivered>>, peer: &str) -> bool {
        for _ in 0..100 {
            if last
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|d| d.device_name == peer)
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    #[tokio::test]
    async fn hears_sync_delivered() {
        let last = Arc::new(Mutex::new(None));
        let (wake, mut woken) = mpsc::unbounded_channel();
        let mut listener = Listener::new(Arc::clone(&last), wake);
        listener.follow(Some(server(true, "laptop").await)).await;
        assert!(heard(&last, "laptop").await);
        assert_eq!(
            last.lock().unwrap().as_ref().unwrap().root,
            std::path::Path::new("/ws")
        );
        assert!(woken.try_recv().is_ok(), "a delivery wakes the refresh");
    }

    #[tokio::test]
    async fn resubscribes_after_reconnect() {
        let last = Arc::new(Mutex::new(None));
        let (wake, _woken) = mpsc::unbounded_channel();
        let mut listener = Listener::new(Arc::clone(&last), wake);
        let first = server(true, "laptop").await;
        listener.follow(Some(Arc::clone(&first))).await;
        assert!(heard(&last, "laptop").await);
        // The same connection again: no second subscription.
        listener.follow(Some(first)).await;
        listener.follow(None).await;
        listener.follow(Some(server(true, "phone").await)).await;
        assert!(heard(&last, "phone").await);
    }

    #[tokio::test]
    async fn old_server_without_subscribe_is_fine() {
        let last = Arc::new(Mutex::new(None));
        let (wake, _woken) = mpsc::unbounded_channel();
        let mut listener = Listener::new(Arc::clone(&last), wake);
        let client = server(false, "laptop").await;
        listener.follow(Some(Arc::clone(&client))).await;
        assert!(!client.is_closed(), "the refusal leaves the connection be");
        assert!(last.lock().unwrap().is_none());
    }
}
