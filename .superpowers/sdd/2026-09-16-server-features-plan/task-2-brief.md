### Task 2: Live propagation in the app server

**Files:**
- Create: `crates/sapphire-framework-server/src/sync/live.rs`
- Modify: `crates/sapphire-framework-server/src/sync/mod.rs`
- Test: `crates/sapphire-framework-server/tests/live.rs`

**Interfaces:**
- Produces:
  - `LivePeers` — the open sessions for one workspace, keyed by device id
  - `SyncRuntime::after_commit(&self, root: &Path, updates: Vec<PathUpdate>)` — push to every
    open session
  - `SyncRuntime::dial_loop(self: Arc<Self>)` — keeps a session open to every peer, with
    exponential backoff capped at 5 minutes
  - `DIAL_BACKOFF_MAX: Duration = 5 min`

**The two rules that keep this from becoming a storm, both tested:**

1. **Never send back to where it came from.** The session an update arrived on is excluded
   when forwarding it.
2. **Never send what the peer already has.** `LiveSession::peer_vv` is consulted first.

Together they make A → S → B work — S forwards A's entries to B — while three connected hosts
do not pass one edit round in a circle for ever.

- [ ] **Step 1: Write the failing tests**

`crates/sapphire-framework-server/tests/live.rs`:

```rust
//! Live propagation between app servers, including through a middle host.

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
```

`LoopbackNetwork::partitioned`, `Host::drop_connections`, `common::frames_sent`,
`common::settle` and `synced_triple` go in the shared fixtures. `frames_sent` needs the
loopback transport to count frames — add a counter to `LoopbackTransport` behind `test-util`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sapphire-framework-server --all-features --test live`
Expected: FAIL — live propagation does not exist.

- [ ] **Step 3: Implement it**

```rust
/// The open sessions for one workspace.
#[derive(Default)]
pub(crate) struct LivePeers {
    sessions: Mutex<HashMap<GrainId, LiveSession>>,
}

impl LivePeers {
    /// Push `updates` to every peer that does not already have them, except `from`.
    ///
    /// The two exclusions are what keep this from becoming a broadcast storm: an entry goes
    /// nowhere it came from, and nowhere it already is.
    pub(crate) async fn fan_out(&self, updates: &[PathUpdate], from: Option<GrainId>) {
        let targets: Vec<(GrainId, Vec<PathUpdate>)> = {
            let sessions = self.sessions.lock().expect("live peers");
            sessions
                .iter()
                .filter(|(device, _)| Some(**device) != from)
                .filter_map(|(device, session)| {
                    if !session.is_open() {
                        return None;
                    }
                    let peer_vv = session.peer_vv();
                    let wanted: Vec<PathUpdate> = updates
                        .iter()
                        .filter(|u| !peer_vv.covers(&u.seen))
                        .cloned()
                        .collect();
                    (!wanted.is_empty()).then_some((*device, wanted))
                })
                .collect()
        };
        for (device, wanted) in targets {
            let session = {
                let sessions = self.sessions.lock().expect("live peers");
                sessions.get(&device).map(|_| device)
            };
            let Some(device) = session else { continue };
            let sessions = self.sessions.lock().expect("live peers");
            if let Some(session) = sessions.get(&device)
                && let Err(err) = futures_lite_block(session.push(wanted))
            {
                tracing::debug!(%device, "dropping a closed session: {err}");
            }
        }
        self.reap();
    }

    /// Forget sessions that have closed.
    fn reap(&self) {
        self.sessions.lock().expect("live peers").retain(|_, s| s.is_open());
    }
}
```

> The sketch above holds a `std::sync::Mutex` across an `await` (`session.push`), which does
> not compile and would be a deadlock if it did. Use `tokio::sync::Mutex` for `sessions`, or —
> better — collect `(device, updates)` under the lock, drop it, and then await the pushes.
> Write the second form; the `futures_lite_block` name is deliberately not a real function so
> this cannot be copied by accident.

`dial_loop` walks `bridge.peers()` every interval and opens a live session to each peer that
has none, with per-peer exponential backoff capped at `DIAL_BACKOFF_MAX`. `after_commit` calls
`fan_out` with `from: None`; the reader side of each session calls it with `from: Some(peer)`
after applying, which is what makes A → S → B work.

- [ ] **Step 4: Run the tests to verify they pass, then commit**

```bash
cargo test -p sapphire-framework-server --all-features --test live
git commit -m "feat(server): propagate commits live, and through a host in the middle"
```

---

