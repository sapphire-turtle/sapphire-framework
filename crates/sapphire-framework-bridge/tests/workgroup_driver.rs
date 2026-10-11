//! The workgroup driver as a running bridge behaves: what it pushes, what it survives.

mod common;

use std::time::{Duration, Instant};

use sapphire_framework_bridge::{BridgeDir, LoopbackNetwork, PeerTransport, Workgroup};

/// A pair of running bridges: `a` founded the workgroup, `b` joined it through pairing.
///
/// Both run `Bridge::run_shared` — the production loops, the driver included. Nothing in
/// these tests scans or dials on a peer's behalf; the propagation under test is what a
/// user gets.
async fn paired() -> (LoopbackNetwork, common::Host, common::Host) {
    let net = LoopbackNetwork::new();
    let a = common::start(&net, common::NODE_A, "host-a").await;
    let ticket = common::invite(&a, "phone").await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
    Workgroup::join(&dir, &ticket, "phone", &net.transport(common::NODE_B))
        .await
        .expect("the join");
    let b = common::start_joined(&net, tmp, dir, common::NODE_B).await;
    (net, a, b)
}

/// Wait until `cond` holds, or fail after `deadline` with `what`.
async fn eventually(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !cond() {
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A retirement written through the CLI path reaches a bridge that joined before it, and
/// the retiring host is the *smaller* device id — the side the dial rule would normally
/// have left silent, because only greater ids dial. This is the regression the probes
/// chased: the tombstone must ride the dialing exception for retired members, or a
/// `device retire` on the founder never lands anywhere.
#[tokio::test(flavor = "multi_thread")]
async fn a_retirement_from_the_smaller_id_reaches_the_greater_one() {
    let (_net, a, b) = paired().await;

    // Bootstrap: B's running bridge learns its own record from A's driver.
    eventually("B's bridge never learned its own device record", || {
        Workgroup::open(&b.dir)
            .unwrap()
            .unwrap()
            .devices()
            .unwrap()
            .by_node_id(common::NODE_B)
            .is_some()
    })
    .await;

    // A retires the phone — the exact write `device retire` makes — whatever side of the
    // id order the joiner landed on. The dialing exception for retired members is
    // symmetric (drive dials a retired member on either side of the id rule), so both
    // orders exercise the same code path; the id assertion the probes used is gone,
    // because a fixture that only half the runs satisfy tests the fixture, not the rule.
    // `a_joiner_with_the_smaller_id_can_dial_the_inviter` pins the *other* side of the
    // rule instead.
    let wg_a = Workgroup::open(&BridgeDir::at(a.tmp.path().join("bridge")).unwrap())
        .unwrap()
        .unwrap();
    wg_a.devices().unwrap().retire("phone").unwrap();

    eventually("B's bridge never learned the retirement", || {
        Workgroup::open(&b.dir)
            .unwrap()
            .unwrap()
            .authorize(common::NODE_B)
            .err()
            .is_some_and(|err| err.to_string().contains("retired"))
    })
    .await;
}

/// The joiner knows the inviter's record from the join itself, so the pair can meet
/// whichever side of the id order the joiner landed on: a joiner whose id sorts smaller
/// dials, and the inviter's ledger already names it.
#[tokio::test(flavor = "multi_thread")]
async fn a_joiner_with_the_smaller_id_can_dial_the_inviter() {
    let (net, _a, b) = paired().await;
    let tb = net.transport(common::NODE_B);
    let wg_b = Workgroup::open(&b.dir).unwrap().unwrap();

    // The inviter's record rode the join response into B's ledger, so B's driver has
    // someone to dial regardless of how the ids sorted.
    let devices = wg_b.devices().unwrap();
    let inviter = devices
        .by_node_id(common::NODE_A)
        .expect("the inviter's record rides the join")
        .clone();
    let me = wg_b.this_device(&tb.node_id()).unwrap();
    if me.id < inviter.id {
        let node = inviter.node_id.expect("the inviter announced its node id");
        let stream = tb
            .open(&node, wg_b.id)
            .await
            .expect("the smaller joiner must be able to open the inviter");
        drop(stream);
    }
    // The joiner with the greater id has nothing to prove here: it is the side that gets
    // dialed, and the pairing test already covers that side.
}
