//! A relayed `bridge.restart` against a real bridge (#314).
//!
//! The bridge restarts in place, on the listeners it already holds, and the app server's
//! sync runtime reconnects to the new one. The relayed call answers only once it has, so a
//! client's next call finds the new bridge rather than `BRIDGE_NOT_RUNNING`.

mod common;

use common::{NODE_A, start_host};
use sapphire_bridge_api::StatusResult;
use sapphire_framework_bridge::LoopbackNetwork;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn a_relayed_restart_returns_once_the_new_bridge_is_connected() {
    let net = LoopbackNetwork::new();
    let host = start_host(&net, NODE_A, "a").await;

    let _: Value = host
        .client
        .call("bridge.restart", json!({}))
        .await
        .expect("the restart is relayed");

    // The bridge answers `bridge.restart` before it goes, so an answer passed straight on
    // would reach the client while the old bridge still serves. By the time the relay
    // answers, the old one is gone: the fixture's own connection to it is dead.
    assert!(
        host.bridge().status().await.is_err(),
        "the old bridge still answers after the relayed restart returned"
    );

    // Straight after: no polling, no sleep.
    let status: StatusResult = host
        .client
        .call("bridge.status", json!({}))
        .await
        .expect("the new bridge answers at once");
    assert_eq!(status.version, common::VERSION);
}
