//! Content travels as `BLOB_START`, chunks and `BLOB_END`, straight into staging.
//!
//! Real two-replica sessions follow `tests/converge.rs`. A "fake peer" is raw frames on the
//! other end of a duplex, written with the crate's exported frame functions, for what a
//! compliant peer would never send.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sapphire_framework_session::{
    CHUNK, Error, Frame, Message, SESSION_FORMAT_VERSION, read_frame, run_session,
    write_blob_chunk, write_blob_end, write_blob_start, write_control,
};
use sapphire_sync::{
    ContentHash, DEFAULT_MAX_FILE_SIZE, PathUpdate, Replica, ReplicaConfig, ReplicaId, SystemClock,
    VersionVector,
};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::Mutex;

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    staging: PathBuf,
    replica: Mutex<Replica>,
}

fn replica(name: &str, max_file_size: u64) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join(".test-app")).unwrap();
    let state = tmp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let mut config = ReplicaConfig::new(
        "test-app",
        root.clone(),
        grain_id::GrainId::random(),
        &state,
    );
    config.max_file_size = max_file_size;
    let staging = config.staging_dir.clone();
    let replica = Replica::open(config, Arc::new(SystemClock)).unwrap();
    Fixture {
        _tmp: tmp,
        root,
        staging,
        replica: Mutex::new(replica),
    }
}

/// Bytes that are not one repeated value, so a chunk out of place changes the hash.
fn patterned(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

async fn scan(fixture: &Fixture) {
    fixture.replica.lock().await.scan().unwrap();
}

/// Run one session between `a` and `b` over an in-process duplex.
async fn sync(a: &Fixture, b: &Fixture, ws: grain_id::GrainId) {
    let (left, right) = tokio::io::duplex(64 * 1024);
    let (x, y) = tokio::join!(
        run_session(left, &a.replica, ws),
        run_session(right, &b.replica, ws)
    );
    x.unwrap();
    y.unwrap();
}

/// Write `bytes` as `rel` on `a`, sync it to `b`, and check it arrived whole.
async fn sync_file(a: &Fixture, b: &Fixture, rel: &str, bytes: &[u8]) {
    let ws = grain_id::GrainId::random();
    std::fs::write(a.root.join(rel), bytes).unwrap();
    scan(a).await;
    sync(a, b, ws).await;
    let got = std::fs::read(b.root.join(rel)).unwrap();
    assert_eq!(got.len(), bytes.len());
    assert!(got == bytes, "{rel} arrived with different content");
}

fn hello(ws: grain_id::GrainId, format: u32, vv: VersionVector) -> Message {
    Message::Hello {
        format,
        workspace_id: ws,
        replica_id: ReplicaId::new(),
        vv,
        max_file_size: DEFAULT_MAX_FILE_SIZE,
    }
}

fn partials(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".partial"))
                .collect()
        })
        .unwrap_or_default()
}

/// The fake peer's halves of a stream whose other end is returned.
fn fake_peer() -> (
    DuplexStream,
    ReadHalf<DuplexStream>,
    WriteHalf<DuplexStream>,
) {
    let (mine, theirs) = tokio::io::duplex(64 * 1024);
    let (r, w) = tokio::io::split(theirs);
    (mine, r, w)
}

/// Read and discard everything the session writes, so it is never blocked on a full pipe.
fn drain(mut r: ReadHalf<DuplexStream>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move { while let Ok(Some(_)) = read_frame(&mut r).await {} })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_of_exactly_64_mib_syncs() {
    let a = replica("a", DEFAULT_MAX_FILE_SIZE);
    let b = replica("b", DEFAULT_MAX_FILE_SIZE);
    sync_file(&a, &b, "exact.bin", &patterned(64 * 1024 * 1024)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_above_64_mib_syncs() {
    let a = replica("a", 128 * 1024 * 1024);
    let b = replica("b", 128 * 1024 * 1024);
    sync_file(&a, &b, "above.bin", &patterned(64 * 1024 * 1024 + 1)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_file_syncs() {
    let a = replica("a", DEFAULT_MAX_FILE_SIZE);
    let b = replica("b", DEFAULT_MAX_FILE_SIZE);
    sync_file(&a, &b, "empty.bin", b"").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_over_the_receivers_limit_is_refused() {
    let ws = grain_id::GrainId::random();
    let b = replica("b", 1024);
    let (mine, r, mut w) = fake_peer();
    let _drain = drain(r);

    write_control(
        &mut w,
        &hello(ws, SESSION_FORMAT_VERSION, VersionVector::new()),
    )
    .await
    .unwrap();
    write_blob_start(&mut w, &ContentHash::of_bytes(&[0u8; 1025]), 1025)
        .await
        .unwrap();

    let err = run_session(mine, &b.replica, ws).await.unwrap_err();
    assert!(matches!(err, Error::Protocol(_)), "got {err:?}");
    assert!(partials(&b.staging).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn content_that_does_not_match_its_hash_is_refused() {
    let ws = grain_id::GrainId::random();
    let b = replica("b", DEFAULT_MAX_FILE_SIZE);
    let (mine, r, mut w) = fake_peer();
    let _drain = drain(r);

    let hash = ContentHash::of_bytes(b"abc");
    write_control(
        &mut w,
        &hello(ws, SESSION_FORMAT_VERSION, VersionVector::new()),
    )
    .await
    .unwrap();
    write_blob_start(&mut w, &hash, 3).await.unwrap();
    write_blob_chunk(&mut w, b"abd").await.unwrap();
    write_blob_end(&mut w).await.unwrap();

    let err = run_session(mine, &b.replica, ws).await.unwrap_err();
    assert!(matches!(err, Error::Protocol(_)), "got {err:?}");
    assert!(b.replica.lock().await.stager().staged(&hash).is_none());
    assert!(partials(&b.staging).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stream_cut_mid_blob_commits_nothing_and_leaves_no_partial() {
    let ws = grain_id::GrainId::random();
    let b = replica("b", DEFAULT_MAX_FILE_SIZE);
    let before = b.replica.lock().await.vv().clone();
    let (mine, r, mut w) = fake_peer();

    // A peer that has something to commit: a version vector beyond b's.
    let mut vv = VersionVector::new();
    vv.add_dot(&sapphire_sync::Dot {
        replica: ReplicaId::new(),
        counter: 1,
    });
    write_control(&mut w, &hello(ws, SESSION_FORMAT_VERSION, vv))
        .await
        .unwrap();
    write_blob_start(&mut w, &ContentHash::of_bytes(&[7u8; 1000]), 1000)
        .await
        .unwrap();
    write_blob_chunk(&mut w, &[7u8; 10]).await.unwrap();
    drop(w);
    drop(r);

    run_session(mine, &b.replica, ws).await.unwrap();
    assert_eq!(b.replica.lock().await.vv(), &before);
    assert!(
        partials(&b.staging).is_empty(),
        "{:?}",
        partials(&b.staging)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_v1_hello_is_refused() {
    let ws = grain_id::GrainId::random();
    let b = replica("b", DEFAULT_MAX_FILE_SIZE);
    let (mine, r, mut w) = fake_peer();
    let _drain = drain(r);

    write_control(&mut w, &hello(ws, 1, VersionVector::new()))
        .await
        .unwrap();

    let err = run_session(mine, &b.replica, ws).await.unwrap_err();
    assert!(
        matches!(err, Error::VersionMismatch { ours: 2, theirs: 1 }),
        "got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_aborted_blob_leaves_the_path_for_the_next_session() {
    let ws = grain_id::GrainId::random();
    let a = replica("a", DEFAULT_MAX_FILE_SIZE);
    let b = replica("b", DEFAULT_MAX_FILE_SIZE);

    // What a would send: a real replica's page for a file over the inline limit.
    let big = patterned(sapphire_framework_session::INLINE_LIMIT + 1000);
    std::fs::write(a.root.join("big.bin"), &big).unwrap();
    scan(&a).await;
    let (updates, a_vv): (Vec<PathUpdate>, VersionVector) = {
        let locked = a.replica.lock().await;
        (
            locked.delta_for(&VersionVector::new()).unwrap(),
            locked.vv().clone(),
        )
    };
    let hash = ContentHash::of_bytes(&big);

    // Played by hand: on b's `Want`, a starts the blob and gives up on it, as a sender
    // whose file changed under it does.
    let (mine, mut r, mut w) = fake_peer();
    let peer = tokio::spawn(async move {
        write_control(&mut w, &hello(ws, SESSION_FORMAT_VERSION, a_vv))
            .await
            .unwrap();
        write_control(&mut w, &Message::Updates(updates))
            .await
            .unwrap();
        write_control(&mut w, &Message::Done).await.unwrap();
        while let Some(frame) = read_frame(&mut r).await.unwrap() {
            match frame {
                Frame::Control(Message::Want(h)) => {
                    assert_eq!(h, hash);
                    write_blob_start(&mut w, &h, big.len() as u64)
                        .await
                        .unwrap();
                    write_blob_chunk(&mut w, &big[..CHUNK.min(big.len())])
                        .await
                        .unwrap();
                    write_control(&mut w, &Message::Aborted(h)).await.unwrap();
                }
                Frame::Control(Message::Settled) => {
                    write_control(&mut w, &Message::Settled).await.unwrap();
                }
                _ => {}
            }
        }
    });

    run_session(mine, &b.replica, ws).await.unwrap();
    peer.await.unwrap();
    assert!(!b.root.join("big.bin").exists());
    assert!(partials(&b.staging).is_empty());

    // The sender aborted because its file changed; its next scan records the new content,
    // and the next session delivers it.
    let rewritten = patterned(sapphire_framework_session::INLINE_LIMIT + 2000);
    std::fs::write(a.root.join("big.bin"), &rewritten).unwrap();
    scan(&a).await;
    sync(&a, &b, ws).await;
    assert!(std::fs::read(b.root.join("big.bin")).unwrap() == rewritten);
}

#[tokio::test(flavor = "multi_thread")]
async fn many_wants_both_ways_do_not_deadlock() {
    let ws = grain_id::GrainId::random();
    let a = replica("a", DEFAULT_MAX_FILE_SIZE);
    let b = replica("b", DEFAULT_MAX_FILE_SIZE);

    // Each side holds more files over the inline limit than a queue once held, so each
    // answers a long run of `Want`s while the 64 KiB pipe pushes back both ways.
    let len = sapphire_framework_session::INLINE_LIMIT + 1;
    for (fixture, side) in [(&a, 1u8), (&b, 2u8)] {
        for i in 0..80u8 {
            let mut bytes = patterned(len);
            bytes[0] = side;
            bytes[1] = i;
            std::fs::write(fixture.root.join(format!("{side}-{i:02}.bin")), bytes).unwrap();
        }
        scan(fixture).await;
    }

    tokio::time::timeout(std::time::Duration::from_secs(120), sync(&a, &b, ws))
        .await
        .expect("the session deadlocked");
    for side in [1u8, 2] {
        for i in 0..80u8 {
            let rel = format!("{side}-{i:02}.bin");
            assert_eq!(
                std::fs::read(a.root.join(&rel)).unwrap(),
                std::fs::read(b.root.join(&rel)).unwrap(),
                "{rel}"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_over_the_receivers_limit_is_not_asked_for() {
    let ws = grain_id::GrainId::random();
    let a = replica("a", DEFAULT_MAX_FILE_SIZE);
    let b = replica("b", 1024);
    // Over the inline limit, so it would travel only on b's `Want`.
    let big = patterned(sapphire_framework_session::INLINE_LIMIT + 1000);
    std::fs::write(a.root.join("big.bin"), &big).unwrap();
    scan(&a).await;

    // Asking would bring a `BLOB_START` over b's limit, which b refuses as a protocol
    // error: the session has to complete, with the path recorded but not written.
    sync(&a, &b, ws).await;
    assert!(!b.root.join("big.bin").exists());
    assert!(b.replica.lock().await.state("big.bin").unwrap().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_inlined_file_over_the_receivers_limit_does_not_fail_the_session() {
    let ws = grain_id::GrainId::random();
    let a = replica("a", DEFAULT_MAX_FILE_SIZE);
    let b = replica("b", 1024);
    // Small enough for a to send inline, unasked, and still over b's limit.
    std::fs::write(a.root.join("small.bin"), patterned(2000)).unwrap();
    std::fs::write(a.root.join("tiny.txt"), b"fits").unwrap();
    scan(&a).await;

    sync(&a, &b, ws).await;
    assert!(!b.root.join("small.bin").exists());
    assert_eq!(std::fs::read(b.root.join("tiny.txt")).unwrap(), b"fits");
    assert!(partials(&b.staging).is_empty());
}
