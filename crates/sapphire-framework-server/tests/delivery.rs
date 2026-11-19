//! Telling a client that this device's edits have left it.
//!
//! A user who saves and then shuts the lid wants to know the edit reached another device
//! first. The app server says so with a `SyncDelivered` event, and in `sync.status`
//! (`pending_delivery`) for a client that asks instead.

use std::path::PathBuf;
use std::time::Duration;

use sapphire_backend::BackendEvent;
use sapphire_backend::protocol as proto;
use sapphire_framework_bridge::LoopbackNetwork;

mod common;
use common::{Host, start_host};

/// Write `content` to `rel` on `host`, through its app server.
async fn write(host: &Host, rel: &str, content: &str) {
    let _: proto::Ack = host
        .client
        .call(
            proto::WRITE_FILE,
            proto::ContentParams {
                path: PathBuf::from(rel),
                content: content.into(),
            },
        )
        .await
        .unwrap();
}

async fn subscribe(host: &Host) {
    let _: proto::Ack = host
        .client
        .call(proto::SUBSCRIBE, serde_json::json!({}))
        .await
        .unwrap();
}

async fn delivery(host: &Host) -> proto::SyncStatusResult {
    host.client
        .call(proto::SYNC_STATUS, serde_json::json!({}))
        .await
        .unwrap()
}

/// Wait until nothing `host` wrote is pending: what its first scan recorded has gone out.
async fn await_delivered(host: &Host) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while delivery(host).await.pending_delivery {
        assert!(std::time::Instant::now() < deadline, "never delivered");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// What the next `SyncDelivered` event on `notifications` says: the workspace, the peer's
/// id and its name. Other events are skipped.
async fn next_delivered(
    notifications: &mut tokio::sync::broadcast::Receiver<sapphire_ipc::Notification>,
    within: Duration,
) -> Option<(PathBuf, grain_id::GrainId, String)> {
    tokio::time::timeout(within, async {
        loop {
            let n = notifications.recv().await.expect("the connection stays up");
            if n.method != proto::EVENT {
                continue;
            }
            let params: proto::EventParams =
                serde_json::from_value(n.params).expect("well-formed params");
            if let BackendEvent::SyncDelivered {
                device_id,
                device_name,
            } = params.event
            {
                return (params.ws, device_id, device_name);
            }
        }
    })
    .await
    .ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_that_reaches_a_peer_is_announced_as_delivered() {
    let net = LoopbackNetwork::new();
    let (a, b) = common::synced_pair(&net).await;
    common::settle(&[&a, &b]).await;
    // The first exchange delivered whatever enabling recorded; this test is about the write.
    await_delivered(&a).await;
    let mut notifications = a.client.notifications();
    subscribe(&a).await;

    write(&a, "note.md", "saved before sleeping").await;

    let (ws, device_id, device_name) = next_delivered(&mut notifications, Duration::from_secs(30))
        .await
        .expect("a delivery within thirty seconds");
    assert_eq!(device_id, b.device_id().await);
    // As the workgroup lists it, which may mark the name.
    assert!(device_name.ends_with("host-b"), "{device_name}");
    assert_eq!(ws, a.ws.canonicalize().unwrap());
    // Delivered means on the peer's disk, not merely sent.
    assert_eq!(
        std::fs::read_to_string(b.ws.join("note.md")).unwrap(),
        "saved before sleeping"
    );
    let now = delivery(&a).await;
    assert!(!now.pending_delivery, "{now:?}");
    assert_eq!(now.acknowledging_peers, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_with_no_peer_to_take_it_stays_pending() {
    let net = LoopbackNetwork::new();
    let a = start_host(&net, common::NODE_A, "host-a").await;
    common::enable_sync(&a).await;
    let mut notifications = a.client.notifications();
    subscribe(&a).await;

    write(&a, "note.md", "nobody to send it to").await;

    assert!(
        next_delivered(&mut notifications, Duration::from_secs(2))
            .await
            .is_none(),
        "nothing can confirm an edit with no peer"
    );
    let now = delivery(&a).await;
    assert!(now.pending_delivery, "{now:?}");
    assert_eq!(now.acknowledging_peers, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_made_while_the_peer_was_away_is_announced_when_it_returns() {
    let net = LoopbackNetwork::new();
    let a = start_host(&net, common::NODE_A, "host-a").await;
    let b = start_host(&net, common::NODE_B, "host-b").await;
    common::introduce(&a, &b);
    common::enable_sync(&a).await;
    let mut notifications = a.client.notifications();
    subscribe(&a).await;

    write(&a, "offline.md", "written alone").await;
    assert!(delivery(&a).await.pending_delivery);

    // The peer turns up: the first exchange carries the edit, and that is a delivery too.
    common::enable_sync(&b).await;
    let (_, device_id, _) = next_delivered(&mut notifications, Duration::from_secs(30))
        .await
        .expect("a delivery once the peer is back");
    assert_eq!(device_id, b.device_id().await);
    assert!(!delivery(&a).await.pending_delivery);
}
