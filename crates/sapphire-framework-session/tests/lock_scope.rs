//! What a session holds the replica's lock across, and what it does not.
//!
//! Not while an exchange merely waits for a peer's `Hello` (issue #163), and not while
//! content is being received: the lock is taken per control message, so a scan can run in
//! the middle of an exchange that is moving a large file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sapphire_framework_session::{
    CHUNK, Frame, Message, SESSION_FORMAT_VERSION, read_frame, run_session, write_blob_chunk,
    write_blob_end, write_blob_start, write_control,
};
use sapphire_sync::{ContentHash, Replica, ReplicaConfig, ReplicaId, SystemClock, VersionVector};
use tokio::sync::{Mutex, oneshot};

fn replica() -> Replica {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    std::fs::create_dir_all(root.join(".test-app")).unwrap();
    let state = tmp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let config = ReplicaConfig::new("test-app", root, grain_id::GrainId::random(), &state);
    // Leaked: the test only needs the replica store to outlive the test body, and a
    // `TempDir` held by value would need threading through every closure below for no
    // benefit — this is a short-lived test process.
    std::mem::forget(tmp);
    Replica::open(config, Arc::new(SystemClock)).unwrap()
}

/// A replica whose root the test can read back.
struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    replica: Arc<Mutex<Replica>>,
}

fn fixture(name: &str) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join(".test-app")).unwrap();
    let state = tmp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let config = ReplicaConfig::new(
        "test-app",
        root.clone(),
        grain_id::GrainId::random(),
        &state,
    );
    let replica = Replica::open(config, Arc::new(SystemClock)).unwrap();
    Fixture {
        _tmp: tmp,
        root,
        replica: Arc::new(Mutex::new(replica)),
    }
}

/// Bytes that are not one repeated value, so a chunk out of place changes the hash.
fn patterned(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

/// Every file under `root` outside the app's own directory, by relative path.
fn files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if rel.starts_with(".test-app") {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

async fn sync(a: &Fixture, b: &Fixture, ws: grain_id::GrainId) {
    let (left, right) = tokio::io::duplex(64 * 1024);
    let (x, y) = tokio::join!(
        run_session(left, &a.replica, ws),
        run_session(right, &b.replica, ws)
    );
    x.unwrap();
    y.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replica_is_not_locked_while_an_exchange_waits_for_the_peers_hello() {
    let shared = Arc::new(Mutex::new(replica()));
    let ws = grain_id::GrainId::random();

    // The peer's half is held open and never written to: `run_session` will sit in its
    // `Hello` wait for the full `HELLO_TIMEOUT`, exactly like a bridge that parked this
    // stream for an owner that has not claimed it yet.
    let (mine, theirs) = tokio::io::duplex(64 * 1024);
    let waiting = Arc::clone(&shared);
    let session = tokio::spawn(async move {
        let _ = run_session(mine, &waiting, ws).await;
    });

    // While that session is stuck waiting for a `Hello` that is never coming, the same
    // replica must still be free for anything else — a scan, another session — to use.
    // Blocked on this lock is exactly the failure mode #163 describes.
    let scanned =
        tokio::time::timeout(Duration::from_secs(2), async { shared.lock().await.scan() }).await;
    assert!(
        scanned.is_ok(),
        "the replica must not be locked merely because a session is waiting for Hello"
    );

    drop(theirs);
    session.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replica_is_not_locked_while_a_blob_is_being_received() {
    let shared = Arc::new(Mutex::new(replica()));
    let ws = grain_id::GrainId::random();
    let (mine, theirs) = tokio::io::duplex(64 * 1024);
    let (mut r, mut w) = tokio::io::split(theirs);

    // Read everything the session writes; report once its `Done` is out, which it queues
    // only after it has taken the lock to work out what to send.
    let (done_tx, done_rx) = oneshot::channel();
    let reading = tokio::spawn(async move {
        let mut done_tx = Some(done_tx);
        while let Ok(Some(frame)) = read_frame(&mut r).await {
            if matches!(frame, Frame::Control(Message::Done))
                && let Some(tx) = done_tx.take()
            {
                let _ = tx.send(());
            }
        }
    });

    let session_replica = Arc::clone(&shared);
    let session = tokio::spawn(async move { run_session(mine, &session_replica, ws).await });

    // The peer: `Hello`, then one chunk of a blob, then nothing until told to go on.
    let (mid_tx, mid_rx) = oneshot::channel();
    let (rest_tx, rest_rx) = oneshot::channel::<()>();
    let peer = tokio::spawn(async move {
        let content = patterned(4 * CHUNK);
        let hash = ContentHash::of_bytes(&content);
        write_control(
            &mut w,
            &Message::Hello {
                format: SESSION_FORMAT_VERSION,
                workspace_id: ws,
                replica_id: ReplicaId::new(),
                vv: VersionVector::new(),
            },
        )
        .await
        .unwrap();
        done_rx.await.unwrap();
        write_blob_start(&mut w, &hash, content.len() as u64)
            .await
            .unwrap();
        write_blob_chunk(&mut w, &content[..CHUNK]).await.unwrap();
        mid_tx.send(()).unwrap();

        rest_rx.await.unwrap();
        for chunk in content[CHUNK..].chunks(CHUNK) {
            write_blob_chunk(&mut w, chunk).await.unwrap();
        }
        write_blob_end(&mut w).await.unwrap();
        write_control(&mut w, &Message::Done).await.unwrap();
        write_control(&mut w, &Message::Settled).await.unwrap();
        w
    });

    // Mid-blob, with the rest of the content held back: the replica is free.
    mid_rx.await.unwrap();
    let scanned =
        tokio::time::timeout(Duration::from_secs(2), async { shared.lock().await.scan() }).await;
    let scanned = scanned.expect("the replica must not be locked while a blob is being received");
    scanned.unwrap();
    rest_tx.send(()).unwrap();

    session.await.unwrap().unwrap();
    drop(peer.await.unwrap());
    reading.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scan_between_pages_still_converges() {
    let ws = grain_id::GrainId::random();
    let a = fixture("a");
    let b = fixture("b");

    // More than one page of small files, and one file large enough to be fetched in chunks.
    for i in 0..300 {
        std::fs::write(a.root.join(format!("f{i:03}.md")), format!("file {i}")).unwrap();
    }
    std::fs::write(a.root.join("big.bin"), patterned(3 * CHUNK)).unwrap();
    a.replica.lock().await.scan().unwrap();

    // Scan b over and over for as long as the first session runs.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let scanning = {
        let replica = Arc::clone(&b.replica);
        let stop = Arc::clone(&stop);
        tokio::spawn(async move {
            let mut scans = 0usize;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                replica.lock().await.scan().unwrap();
                scans += 1;
                tokio::task::yield_now().await;
            }
            scans
        })
    };
    sync(&a, &b, ws).await;
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let scans = scanning.await.unwrap();
    assert!(scans > 0);

    // Whatever a scan recorded mid-exchange, a second session settles it.
    sync(&a, &b, ws).await;
    let on_a = files(&a.root);
    let on_b = files(&b.root);
    assert_eq!(on_a.len(), 301);
    assert_eq!(
        on_a.keys().collect::<Vec<_>>(),
        on_b.keys().collect::<Vec<_>>()
    );
    assert!(on_a == on_b, "the two roots hold different content");
}
