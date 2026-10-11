### Task 1: A session that stays open

**Files:**
- Create: `crates/sapphire-framework-session/src/live.rs`
- Modify: `crates/sapphire-framework-session/src/{lib.rs,session.rs,message.rs}`
- Test: `crates/sapphire-framework-session/tests/live.rs`

**Interfaces:**
- Consumes: `run_session`'s pieces (step 7, Task 2)
- Produces:
  - `Message::Live(Vec<PathUpdate>)` — a push after the initial exchange
  - `LiveSession`: `push(&self, updates: Vec<PathUpdate>) -> Result<()>`,
    `updates(&self) -> broadcast::Receiver<Vec<PathUpdate>>`,
    `peer_vv(&self) -> VersionVector`, `is_open(&self) -> bool`, `close(self)`
  - `async open_live_session<S>(stream: S, replica: Arc<Mutex<Replica>>, workspace_id: GrainId) -> Result<(SessionOutcome, LiveSession)>`

**What changes from step 7:** `run_session` still exists, unchanged, for a one-shot catch-up
and for the tests that use it. `open_live_session` does the same initial exchange and then,
instead of returning, keeps the reader running: `Live` messages are applied as they arrive, and
`push` sends. It reports the initial outcome as soon as both sides have said `Done`, so a
caller can tell "we are caught up" from "we are still connected".

`peer_vv` is the key to not shouting: it advances as the peer acknowledges, and the caller
consults it before forwarding anything.

- [ ] **Step 1: Write the failing tests**

`crates/sapphire-framework-session/tests/live.rs`:

```rust
//! A session that keeps going after the initial exchange.

use std::sync::Arc;

use sapphire_framework_session::open_live_session;
use sapphire_sync::{Replica, ReplicaConfig, SystemClock};
use tokio::sync::Mutex;

struct Side {
    _tmp: tempfile::TempDir,
    root: std::path::PathBuf,
    replica: Arc<Mutex<Replica>>,
}

fn side(name: &str) -> Side {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join(".test-app")).unwrap();
    let state = tmp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let config =
        ReplicaConfig::new("test-app", root.clone(), grain_id::GrainId::random(), &state);
    let replica = Replica::open(config, Arc::new(SystemClock)).unwrap();
    Side { _tmp: tmp, root, replica: Arc::new(Mutex::new(replica)) }
}

async fn scan(side: &Side) -> Vec<sapphire_sync::PathUpdate> {
    let mut replica = side.replica.lock().await;
    let outcome = replica.scan().unwrap();
    match outcome {
        sapphire_sync::ScanOutcome::Scanned(report) => report
            .recorded
            .into_iter()
            .map(|entry| sapphire_sync::PathUpdate {
                path: entry.path.clone(),
                versions: vec![entry],
                seen: replica.vv().clone(),
            })
            .collect(),
        sapphire_sync::ScanOutcome::Paused(_) => vec![],
    }
}

/// Wait until `rel` exists on `side`, or fail.
async fn await_file(side: &Side, rel: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Ok(text) = std::fs::read_to_string(side.root.join(rel)) {
            return text;
        }
        assert!(std::time::Instant::now() < deadline, "{rel} never arrived");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_initial_exchange_still_happens() {
    let ws = grain_id::GrainId::random();
    let a = side("a");
    let b = side("b");
    std::fs::write(a.root.join("first.md"), "before the session").unwrap();
    scan(&a).await;

    let (left, right) = tokio::io::duplex(64 * 1024);
    let (x, y) = tokio::join!(
        open_live_session(left, Arc::clone(&a.replica), ws),
        open_live_session(right, Arc::clone(&b.replica), ws)
    );
    let (outcome_a, _live_a) = x.unwrap();
    let (_outcome_b, _live_b) = y.unwrap();

    assert_eq!(outcome_a.sent, 1);
    assert_eq!(await_file(&b, "first.md").await, "before the session");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_after_the_exchange_arrives_without_a_new_session() {
    let ws = grain_id::GrainId::random();
    let a = side("a");
    let b = side("b");

    let (left, right) = tokio::io::duplex(64 * 1024);
    let (x, y) = tokio::join!(
        open_live_session(left, Arc::clone(&a.replica), ws),
        open_live_session(right, Arc::clone(&b.replica), ws)
    );
    let (_, live_a) = x.unwrap();
    let (_, _live_b) = y.unwrap();

    std::fs::write(a.root.join("later.md"), "after the session started").unwrap();
    let updates = scan(&a).await;
    live_a.push(updates).await.unwrap();

    assert_eq!(await_file(&b, "later.md").await, "after the session started");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_subscriber_sees_what_arrived() {
    let ws = grain_id::GrainId::random();
    let a = side("a");
    let b = side("b");

    let (left, right) = tokio::io::duplex(64 * 1024);
    let (x, y) = tokio::join!(
        open_live_session(left, Arc::clone(&a.replica), ws),
        open_live_session(right, Arc::clone(&b.replica), ws)
    );
    let (_, live_a) = x.unwrap();
    let (_, live_b) = y.unwrap();

    let mut received = live_b.updates();
    std::fs::write(a.root.join("watched.md"), "x").unwrap();
    live_a.push(scan(&a).await).await.unwrap();

    let updates = tokio::time::timeout(std::time::Duration::from_secs(10), received.recv())
        .await
        .expect("an update within ten seconds")
        .unwrap();
    assert!(updates.iter().any(|u| u.path.ends_with("watched.md")), "{updates:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_peers_version_vector_advances_as_pushes_are_taken() {
    let ws = grain_id::GrainId::random();
    let a = side("a");
    let b = side("b");

    let (left, right) = tokio::io::duplex(64 * 1024);
    let (x, y) = tokio::join!(
        open_live_session(left, Arc::clone(&a.replica), ws),
        open_live_session(right, Arc::clone(&b.replica), ws)
    );
    let (_, live_a) = x.unwrap();
    let (_, _live_b) = y.unwrap();

    let before = live_a.peer_vv();
    std::fs::write(b.root.join("from-b.md").clone(), "x").unwrap();
    let updates = scan(&b).await;
    let (_, live_b) = (0, {
        // Re-take B's live handle from the join above.
        y_handle_placeholder()
    });
    live_b.push(updates).await.unwrap();
    await_file(&a, "from-b.md").await;

    assert_ne!(
        live_a.peer_vv(),
        before,
        "the peer's version vector must advance, or every entry is forwarded for ever"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_one_side_is_visible_on_the_other() {
    let ws = grain_id::GrainId::random();
    let a = side("a");
    let b = side("b");

    let (left, right) = tokio::io::duplex(64 * 1024);
    let (x, y) = tokio::join!(
        open_live_session(left, Arc::clone(&a.replica), ws),
        open_live_session(right, Arc::clone(&b.replica), ws)
    );
    let (_, live_a) = x.unwrap();
    let (_, live_b) = y.unwrap();

    live_a.close();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while live_b.is_open() {
        assert!(std::time::Instant::now() < deadline, "the close was never noticed");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pushing_on_a_closed_session_fails_rather_than_hanging() {
    let ws = grain_id::GrainId::random();
    let a = side("a");
    let b = side("b");

    let (left, right) = tokio::io::duplex(64 * 1024);
    let (x, y) = tokio::join!(
        open_live_session(left, Arc::clone(&a.replica), ws),
        open_live_session(right, Arc::clone(&b.replica), ws)
    );
    let (_, live_a) = x.unwrap();
    let (_, live_b) = y.unwrap();
    live_b.close();

    // Give the close a moment to reach A, then push.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    std::fs::write(a.root.join("into-the-void.md"), "x").unwrap();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        live_a.push(scan(&a).await),
    )
    .await;
    assert!(result.is_ok(), "push must not hang on a closed session");
}
```

> `the_peers_version_vector_advances_as_pushes_are_taken` as written cannot get at B's handle,
> because `y` was destructured above. Restructure it: bind `let (_, live_b) = y.unwrap();`
> alongside `live_a` at the top, and delete the `y_handle_placeholder()` line. Write it that
> way — the placeholder is there only so the mistake is not copied silently.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sapphire-framework-session --all-features --test live`
Expected: FAIL — `open_live_session` does not exist.

- [ ] **Step 3: Implement the live phase**

Add `Live(Vec<PathUpdate>)` to `Message`. Refactor `session.rs` so the initial exchange is a
private function returning the reader, the writer channel, the peer's version vector and the
outcome; `run_session` closes over it as it does today, and `open_live_session` keeps it.

```rust
/// A session that stays open after the initial exchange.
///
/// Dropping it closes the session; `close` is the same thing said out loud.
pub struct LiveSession {
    out: mpsc::Sender<Out>,
    updates: broadcast::Sender<Vec<PathUpdate>>,
    peer_vv: Arc<Mutex<VersionVector>>,
    open: Arc<AtomicBool>,
    reader: JoinHandle<()>,
}

impl LiveSession {
    /// Send updates to the peer.
    ///
    /// Fails rather than waiting once the session has closed, so a caller looping over
    /// peers does not stall on one that went away.
    pub async fn push(&self, updates: Vec<PathUpdate>) -> Result<()> {
        if !self.is_open() {
            return Err(Error::Protocol("the session is closed".to_owned()));
        }
        self.out
            .send(Out::Control(Message::Live(updates)))
            .await
            .map_err(|_| Error::Protocol("the session is closed".to_owned()))
    }

    /// Updates that arrived from the peer, after they were applied.
    pub fn updates(&self) -> broadcast::Receiver<Vec<PathUpdate>> {
        self.updates.subscribe()
    }

    /// Everything the peer is known to have.
    ///
    /// Consulted before forwarding: an entry the peer already covers must not be sent again,
    /// or three connected hosts will pass one edit round in a circle.
    pub fn peer_vv(&self) -> VersionVector {
        self.peer_vv.lock().expect("peer vv").clone()
    }

    /// Is the session still usable?
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Relaxed)
    }

    /// Close it.
    pub fn close(self) {
        drop(self);
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        self.open.store(false, Ordering::Relaxed);
        self.reader.abort();
    }
}
```

The reader task applies each `Live` batch to the replica, merges the batch's `seen` into
`peer_vv`, publishes the applied updates on the broadcast, and clears `open` when the stream
ends.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sapphire-framework-session --all-features`
Expected: PASS — the six live tests plus step 7's eight.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
git add crates/sapphire-framework-session
git commit -m "feat(session): keep a session open and push what is committed"
```

---

