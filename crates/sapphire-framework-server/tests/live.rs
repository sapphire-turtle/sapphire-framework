//! Live propagation between app servers, including through a middle host.
//!
//! The claim under test is the app server's half of the "sync feels immediate" goal: a
//! commit made through one host's IPC reaches every other host over sessions that are
//! already open, a host in the middle forwards what it receives, and the two storm rules —
//! never back to the source, never what the peer already has — hold when three hosts are
//! connected in a triangle.

mod common;

use std::path::PathBuf;

use sapphire_backend::protocol as proto;
use sapphire_framework_bridge::LoopbackNetwork;

/// Wait for `rel` to appear on `host`, with a deadline rather than a sleep.
async fn await_file(host: &common::Host, rel: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Ok(text) = std::fs::read_to_string(host.ws.join(rel)) {
            return text;
        }
        assert!(std::time::Instant::now() < deadline, "{rel} never arrived");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn write(host: &common::Host, rel: &str, content: &str) {
    let _: proto::Ack = host
        .client
        .call(
            proto::WRITE_FILE,
            proto::ContentParams {
                ws: host.ws.clone(),
                path: PathBuf::from(rel),
                content: content.into(),
            },
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_arrives_without_waiting_for_the_next_dial() {
    let net = LoopbackNetwork::new();
    let (a, b) = common::synced_pair(&net).await;
    common::settle(&[&a, &b]).await;

    let started = std::time::Instant::now();
    write(&a, "quick.md", "now").await;
    assert_eq!(await_file(&b, "quick.md").await, "now");

    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "a live session should deliver in well under the dial interval, took {:?}",
        started.elapsed()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_propagates_through_a_host_in_the_middle() {
    // A and S are connected; S and B are connected; A and B are not. This is the topology a
    // self-hosted server exists for.
    let net = LoopbackNetwork::partitioned(&[
        (common::NODE_A, common::NODE_S),
        (common::NODE_S, common::NODE_B),
    ]);
    let (a, s, b) = common::synced_triple(&net).await;
    common::settle(&[&a, &s, &b]).await;

    write(&a, "relayed.md", "from a").await;
    assert_eq!(await_file(&b, "relayed.md").await, "from a");
    let _ = s;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_entry_is_not_forwarded_back_to_the_peer_it_came_from() {
    let net = LoopbackNetwork::new();
    let (a, b) = common::synced_pair(&net).await;
    common::settle(&[&a, &b]).await;

    let before = common::frames_sent(&b);
    write(&a, "one-way.md", "x").await;
    await_file(&b, "one-way.md").await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    let after = common::frames_sent(&b);
    assert!(
        after - before < 5,
        "B sent {} frames for one incoming file; it is echoing",
        after - before
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn three_connected_hosts_do_not_loop_an_edit_between_them() {
    let net = LoopbackNetwork::new(); // fully connected
    let (a, s, b) = common::synced_triple(&net).await;
    common::settle(&[&a, &s, &b]).await;

    write(&a, "triangle.md", "x").await;
    await_file(&b, "triangle.md").await;
    await_file(&s, "triangle.md").await;

    // Let any loop run for a while, then check nobody is still talking.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let quiet_a = common::frames_sent(&a);
    let quiet_b = common::frames_sent(&b);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    assert_eq!(common::frames_sent(&a), quiet_a, "A is still sending");
    assert_eq!(common::frames_sent(&b), quiet_b, "B is still sending");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_that_goes_away_is_redialled() {
    let net = LoopbackNetwork::new();
    let (a, b) = common::synced_pair(&net).await;
    common::settle(&[&a, &b]).await;

    b.drop_connections().await;
    write(&a, "after-drop.md", "x").await;

    assert_eq!(
        await_file(&b, "after-drop.md").await,
        "x",
        "a dropped connection must be redialled, not mourned"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unreachable_peer_does_not_stall_the_others() {
    let net = LoopbackNetwork::partitioned(&[(common::NODE_A, common::NODE_B)]);
    let (a, s, b) = common::synced_triple(&net).await;
    // S is listed in the workgroup but reachable by nobody.
    common::settle(&[&a, &b]).await;

    write(&a, "despite-s.md", "x").await;
    assert_eq!(await_file(&b, "despite-s.md").await, "x");
    let _ = s;
}
