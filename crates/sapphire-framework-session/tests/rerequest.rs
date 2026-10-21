//! Content that never arrived is asked for again by every later session (#297).
//!
//! Committing a session takes in the peer's version vector even when some content did not
//! arrive, so the next delta does not carry those paths again; the receiver has to ask.
//! Real sessions follow `tests/blob.rs`; the first, failed session is a fake peer playing
//! raw frames.

use std::path::PathBuf;
use std::sync::Arc;

use sapphire_framework_session::{
    CHUNK, Frame, INLINE_LIMIT, Message, SESSION_FORMAT_VERSION, read_frame, run_session,
    write_blob_chunk, write_blob_start, write_control,
};
use sapphire_sync::{
    DEFAULT_MAX_FILE_SIZE, PathUpdate, Replica, ReplicaConfig, ReplicaId, SystemClock,
    VersionVector,
};
use tokio::sync::Mutex;

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
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
    let replica = Replica::open(config, Arc::new(SystemClock)).unwrap();
    Fixture {
        _tmp: tmp,
        root,
        replica: Mutex::new(replica),
    }
}

/// Bytes over the inline limit, so they travel only on `Want`.
fn big() -> Vec<u8> {
    (0..INLINE_LIMIT + 1000).map(|i| (i % 251) as u8).collect()
}

async fn scan(fixture: &Fixture) {
    fixture.replica.lock().await.scan().unwrap();
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

/// How the fake peer answers `b`'s `Want`.
#[derive(Clone, Copy)]
enum Answer {
    Missing,
    Aborted,
}

/// One session in which `b` receives `a`'s pages but, on its `Want`, gets `answer`
/// instead of the content. `b` commits it all the same.
async fn session_without_content(a: &Fixture, b: &Fixture, ws: grain_id::GrainId, answer: Answer) {
    let (updates, a_vv): (Vec<PathUpdate>, VersionVector) = {
        let locked = a.replica.lock().await;
        (
            locked.delta_for(&VersionVector::new()).unwrap(),
            locked.vv().clone(),
        )
    };
    let (mine, theirs) = tokio::io::duplex(64 * 1024);
    let (mut r, mut w) = tokio::io::split(theirs);
    let peer = tokio::spawn(async move {
        let hello = Message::Hello {
            format: SESSION_FORMAT_VERSION,
            workspace_id: ws,
            replica_id: ReplicaId::new(),
            vv: a_vv,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
        };
        write_control(&mut w, &hello).await.unwrap();
        write_control(&mut w, &Message::Updates(updates))
            .await
            .unwrap();
        write_control(&mut w, &Message::Done).await.unwrap();
        while let Some(frame) = read_frame(&mut r).await.unwrap() {
            match frame {
                Frame::Control(Message::Want(h)) => match answer {
                    Answer::Missing => write_control(&mut w, &Message::Missing(h)).await.unwrap(),
                    Answer::Aborted => {
                        let bytes = big();
                        write_blob_start(&mut w, &h, bytes.len() as u64)
                            .await
                            .unwrap();
                        write_blob_chunk(&mut w, &bytes[..CHUNK.min(bytes.len())])
                            .await
                            .unwrap();
                        write_control(&mut w, &Message::Aborted(h)).await.unwrap();
                    }
                },
                Frame::Control(Message::Settled) => {
                    write_control(&mut w, &Message::Settled).await.unwrap();
                }
                _ => {}
            }
        }
    });
    run_session(mine, &b.replica, ws).await.unwrap();
    peer.await.unwrap();
    assert!(
        a.replica
            .lock()
            .await
            .delta_for(b.replica.lock().await.vv())
            .unwrap()
            .is_empty(),
        "b committed a's version vector, so no later delta carries the path"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn content_aborted_then_rewritten_with_the_same_bytes_arrives_next_session() {
    let ws = grain_id::GrainId::random();
    let a = replica("a", DEFAULT_MAX_FILE_SIZE);
    let b = replica("b", DEFAULT_MAX_FILE_SIZE);
    std::fs::write(a.root.join("big.bin"), big()).unwrap();
    scan(&a).await;

    session_without_content(&a, &b, ws, Answer::Aborted).await;
    assert!(!b.root.join("big.bin").exists());

    // The same bytes again: a's scan records nothing new.
    std::fs::write(a.root.join("big.bin"), big()).unwrap();
    scan(&a).await;
    sync(&a, &b, ws).await;
    assert!(std::fs::read(b.root.join("big.bin")).unwrap() == big());
}

#[tokio::test(flavor = "multi_thread")]
async fn content_a_peer_lacked_arrives_from_another_peer_with_nothing_new() {
    let ws = grain_id::GrainId::random();
    let a = replica("a", DEFAULT_MAX_FILE_SIZE);
    let b = replica("b", DEFAULT_MAX_FILE_SIZE);
    let c = replica("c", DEFAULT_MAX_FILE_SIZE);
    std::fs::write(a.root.join("big.bin"), big()).unwrap();
    scan(&a).await;
    sync(&a, &c, ws).await;

    session_without_content(&a, &b, ws, Answer::Missing).await;
    assert!(!b.root.join("big.bin").exists());

    // c has merged exactly what b has: neither side has a page to send.
    assert!(
        c.replica
            .lock()
            .await
            .delta_for(b.replica.lock().await.vv())
            .unwrap()
            .is_empty()
    );
    sync(&b, &c, ws).await;
    assert!(std::fs::read(b.root.join("big.bin")).unwrap() == big());
}

#[tokio::test(flavor = "multi_thread")]
async fn content_still_missing_everywhere_is_asked_for_again_without_failing() {
    let ws = grain_id::GrainId::random();
    let a = replica("a", DEFAULT_MAX_FILE_SIZE);
    let b = replica("b", DEFAULT_MAX_FILE_SIZE);
    let c = replica("c", DEFAULT_MAX_FILE_SIZE);
    std::fs::write(a.root.join("big.bin"), big()).unwrap();
    scan(&a).await;

    session_without_content(&a, &b, ws, Answer::Missing).await;
    // c lacks it too, and learns of it only from b.
    sync(&b, &c, ws).await;
    sync(&b, &c, ws).await;
    assert!(!b.root.join("big.bin").exists());
    assert!(!c.root.join("big.bin").exists());

    // a still has it, and both get it from a.
    sync(&a, &c, ws).await;
    sync(&b, &a, ws).await;
    assert!(std::fs::read(b.root.join("big.bin")).unwrap() == big());
    assert!(std::fs::read(c.root.join("big.bin")).unwrap() == big());
}

#[tokio::test(flavor = "multi_thread")]
async fn content_over_the_receivers_limit_is_never_asked_for() {
    let ws = grain_id::GrainId::random();
    let a = replica("a", DEFAULT_MAX_FILE_SIZE);
    let b = replica("b", 1024);
    std::fs::write(a.root.join("big.bin"), big()).unwrap();
    scan(&a).await;

    // A request would bring a `BLOB_START` over b's limit, which b refuses as a protocol
    // error: both sessions have to complete.
    sync(&a, &b, ws).await;
    sync(&a, &b, ws).await;
    assert!(!b.root.join("big.bin").exists());
    assert!(b.replica.lock().await.state("big.bin").unwrap().is_some());
}
