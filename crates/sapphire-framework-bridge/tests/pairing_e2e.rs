//! A second host joins a workgroup and starts syncing.

mod common;

use sapphire_framework_bridge::{BridgeDir, LoopbackNetwork, Workgroup};

/// Invite `joiner_name` from `a`, and join from a fresh host on `net`.
///
/// Returns the joiner's bridge directory and the workgroup it ended up in.
async fn pair_in(
    net: &LoopbackNetwork,
    a: &common::Host,
    node: &str,
    joiner_name: &str,
) -> (tempfile::TempDir, BridgeDir, Workgroup) {
    let ticket = common::invite(a, joiner_name).await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
    let wg = Workgroup::join(&dir, &ticket, joiner_name, &net.transport(node))
        .await
        .expect("the join");
    (tmp, dir, wg)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_joined_device_appears_on_the_inviter() {
    let net = LoopbackNetwork::new();
    let a = common::start(&net, common::NODE_A, "host-a").await;
    let (_tmp, _dir, _wg) = pair_in(&net, &a, common::NODE_B, "phone").await;

    let wg_a = Workgroup::open(&BridgeDir::at(a.tmp.path().join("bridge")).unwrap())
        .unwrap()
        .expect("a workgroup");
    let devices = wg_a.devices().unwrap();
    let phone = devices
        .by_node_id(common::NODE_B)
        .expect("the joiner's record");
    assert_eq!(phone.name, "phone");
    assert!(!phone.is_retired());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_joined_device_can_open_a_sync_stream() {
    // The whole point: after pairing, the ordinary authorization path admits it.
    let net = LoopbackNetwork::new();
    let a = common::start(&net, common::NODE_A, "host-a").await;
    let (_tmp, _dir, wg_b) = pair_in(&net, &a, common::NODE_B, "phone").await;

    let wg_a = Workgroup::open(&BridgeDir::at(a.tmp.path().join("bridge")).unwrap())
        .unwrap()
        .unwrap();
    assert!(
        wg_a.authorize(common::NODE_B).is_ok(),
        "a paired device must pass the normal check"
    );
    assert_eq!(wg_b.id, wg_a.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_third_device_learns_about_the_second_through_the_workgroup_workspace() {
    // A invites B, then A invites C. C never spoke to B, but the ledger replicates, so C
    // admits B's connection. Without this, a workgroup of four devices would need six
    // pairings instead of three.
    let net = LoopbackNetwork::new();
    let a = common::start(&net, common::NODE_A, "host-a").await;
    let (_tb, dir_b, _wg_b) = pair_in(&net, &a, common::NODE_B, "phone").await;
    let (_tc, dir_c, _wg_c) = pair_in(&net, &a, common::NODE_C, "tablet").await;

    // Let the workgroup workspace reach C.
    common::sync_workgroup(&a, &dir_c, &net).await;

    let wg_c = Workgroup::open(&dir_c).unwrap().unwrap();
    assert!(
        wg_c.authorize(common::NODE_B).is_ok(),
        "C must admit B without ever having paired with it"
    );
    let _ = dir_b;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retired_device_stops_being_admitted_everywhere() {
    let net = LoopbackNetwork::new();
    let a = common::start(&net, common::NODE_A, "host-a").await;
    let (_tb, _dir_b, _) = pair_in(&net, &a, common::NODE_B, "phone").await;
    let (_tc, dir_c, _) = pair_in(&net, &a, common::NODE_C, "tablet").await;
    common::sync_workgroup(&a, &dir_c, &net).await;
    assert!(
        Workgroup::open(&dir_c)
            .unwrap()
            .unwrap()
            .authorize(common::NODE_B)
            .is_ok()
    );

    // A retires the phone, and the change reaches C.
    let wg_a = Workgroup::open(&BridgeDir::at(a.tmp.path().join("bridge")).unwrap())
        .unwrap()
        .unwrap();
    wg_a.devices().unwrap().retire("phone").unwrap();
    common::sync_workgroup(&a, &dir_c, &net).await;

    let err = Workgroup::open(&dir_c)
        .unwrap()
        .unwrap()
        .authorize(common::NODE_B)
        .unwrap_err();
    assert!(err.to_string().contains("retired"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_that_was_never_invited_is_refused() {
    let net = LoopbackNetwork::new();
    let a = common::start(&net, common::NODE_A, "host-a").await;

    let wg_a = Workgroup::open(&BridgeDir::at(a.tmp.path().join("bridge")).unwrap())
        .unwrap()
        .unwrap();
    let err = wg_a.authorize(common::NODE_B).unwrap_err();
    assert!(
        err.to_string().contains("not a device of this workgroup"),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ticket_that_was_already_used_does_not_admit_a_second_device() {
    let net = LoopbackNetwork::new();
    let a = common::start(&net, common::NODE_A, "host-a").await;
    let ticket = common::invite(&a, "phone").await;

    let tmp_b = tempfile::tempdir().unwrap();
    let dir_b = BridgeDir::at(tmp_b.path().join("bridge")).unwrap();
    Workgroup::join(&dir_b, &ticket, "phone", &net.transport(common::NODE_B))
        .await
        .expect("the first join");

    let tmp_c = tempfile::tempdir().unwrap();
    let dir_c = BridgeDir::at(tmp_c.path().join("bridge")).unwrap();
    assert!(
        Workgroup::join(&dir_c, &ticket, "intruder", &net.transport(common::NODE_C))
            .await
            .is_err(),
        "a ticket is good once"
    );
}
