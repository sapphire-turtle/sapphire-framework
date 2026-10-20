//! The initial exchange gives up on a peer once nothing has moved for `IDLE_TIMEOUT`, in
//! either direction — not after a fixed total time.
//!
//! Paused tokio time: every wait below is skipped through, not sat out.

use std::sync::Arc;
use std::time::Duration;

use sapphire_framework_session::{
    CHUNK, Error, Frame, IDLE_TIMEOUT, Message, SESSION_FORMAT_VERSION, read_frame, run_session,
    write_blob_chunk, write_blob_end, write_blob_start, write_control,
};
use sapphire_sync::{ContentHash, Replica, ReplicaConfig, ReplicaId, SystemClock, VersionVector};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::Mutex;

/// Thirty seconds: half the idle timeout, so a peer moving once per step is never idle
/// for long enough to be cut off, but two steps make a minute.
const STEP: Duration = Duration::from_secs(30);

struct Fixture {
    _tmp: tempfile::TempDir,
    root: std::path::PathBuf,
    replica: Mutex<Replica>,
}

fn replica() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
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
        replica: Mutex::new(replica),
    }
}

/// Bytes that are not one repeated value, so a chunk out of place changes the hash.
fn patterned(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn hello(ws: grain_id::GrainId) -> Message {
    Message::Hello {
        format: SESSION_FORMAT_VERSION,
        workspace_id: ws,
        replica_id: ReplicaId::new(),
        vv: VersionVector::new(),
        max_file_size: sapphire_sync::DEFAULT_MAX_FILE_SIZE,
    }
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

#[tokio::test(start_paused = true)]
async fn an_exchange_with_no_progress_is_given_up_after_the_idle_timeout() {
    let ws = grain_id::GrainId::random();
    let b = replica();
    let (mine, r, mut w) = fake_peer();
    let _drain = drain(r);

    write_control(&mut w, &hello(ws)).await.unwrap();

    // Then silence, with the stream held open. The outer bound only keeps a session that
    // never gives up from hanging the test.
    let err = tokio::time::timeout(Duration::from_secs(600), run_session(mine, &b.replica, ws))
        .await
        .expect("the session waited out ten minutes of silence")
        .unwrap_err();
    assert!(
        matches!(&err, Error::Protocol(why) if why.contains("no progress")),
        "got {err:?}"
    );
    drop(w);
}

#[tokio::test(start_paused = true)]
async fn a_slow_transfer_that_keeps_progressing_is_not_cut_off() {
    let ws = grain_id::GrainId::random();
    let b = replica();
    let (mine, r, mut w) = fake_peer();
    let _drain = drain(r);

    let content = patterned(5 * CHUNK);
    let hash = ContentHash::of_bytes(&content);
    let peer = tokio::spawn(async move {
        write_control(&mut w, &hello(ws)).await.unwrap();
        write_blob_start(&mut w, &hash, content.len() as u64)
            .await
            .unwrap();
        // Two and a half minutes in all, far past the idle timeout, but never idle for it.
        for chunk in content.chunks(CHUNK) {
            tokio::time::sleep(STEP).await;
            write_blob_chunk(&mut w, chunk).await.unwrap();
        }
        write_blob_end(&mut w).await.unwrap();
        write_control(&mut w, &Message::Done).await.unwrap();
        write_control(&mut w, &Message::Settled).await.unwrap();
        w
    });

    run_session(mine, &b.replica, ws).await.unwrap();
    drop(peer.await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn a_sender_waiting_on_its_own_upload_is_not_cut_off() {
    let ws = grain_id::GrainId::random();
    let a = replica();
    let content = patterned(6 * CHUNK);
    let hash = ContentHash::of_bytes(&content);
    std::fs::write(a.root.join("big.bin"), &content).unwrap();
    a.replica.lock().await.scan().unwrap();

    let (mine, mut r, mut w) = fake_peer();
    let peer = tokio::spawn(async move {
        write_control(&mut w, &hello(ws)).await.unwrap();
        // The session's pages and `Done` come first.
        loop {
            match read_frame(&mut r).await.unwrap() {
                Some(Frame::Control(Message::Done)) => break,
                Some(_) => {}
                None => panic!("the session closed before its Done"),
            }
        }
        write_control(&mut w, &Message::Want(hash)).await.unwrap();
        // `BLOB_START` is read as it comes. It is a few bytes already in the pipe, and
        // reading it moves nothing the writer is waiting on: a 30 s pause for it would
        // stretch the writer's first chunk to a full minute.
        match read_frame(&mut r).await.unwrap() {
            Some(Frame::BlobStart { .. }) => {}
            other => panic!("expected BLOB_START, got {other:?}"),
        }

        // A slow reader that says nothing at all while the content comes in: the session
        // hears nothing from it for three minutes, and is still moving.
        loop {
            tokio::time::sleep(STEP).await;
            match read_frame(&mut r).await.unwrap() {
                Some(Frame::BlobEnd) => break,
                Some(_) => {}
                None => panic!("the session closed mid-blob"),
            }
        }
        write_control(&mut w, &Message::Done).await.unwrap();
        write_control(&mut w, &Message::Settled).await.unwrap();
        while let Ok(Some(_)) = read_frame(&mut r).await {}
        w
    });

    run_session(mine, &a.replica, ws).await.unwrap();
    drop(peer.await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn a_hello_that_took_its_time_does_not_count_as_idleness() {
    let ws = grain_id::GrainId::random();
    let b = replica();
    let (mine, r, mut w) = fake_peer();
    let _drain = drain(r);

    // Within `HELLO_TIMEOUT`, but past the idle timeout; then thirty quiet seconds more.
    let peer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(80)).await;
        write_control(&mut w, &hello(ws)).await.unwrap();
        tokio::time::sleep(STEP).await;
        write_control(&mut w, &Message::Done).await.unwrap();
        write_control(&mut w, &Message::Settled).await.unwrap();
        w
    });

    run_session(mine, &b.replica, ws).await.unwrap();
    drop(peer.await.unwrap());
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_asks_and_never_reads_is_given_up_on() {
    let ws = grain_id::GrainId::random();
    let a = replica();
    let content = patterned(2 * CHUNK);
    let hash = ContentHash::of_bytes(&content);
    std::fs::write(a.root.join("big.bin"), &content).unwrap();
    a.replica.lock().await.scan().unwrap();

    // More `Want`s than any queue once held, and not a byte read: the session's writer
    // stalls on the full pipe, and its read loop must not stall behind it.
    let (mine, r, mut w) = fake_peer();
    write_control(&mut w, &hello(ws)).await.unwrap();
    for _ in 0..80 {
        write_control(&mut w, &Message::Want(hash)).await.unwrap();
    }

    let err = tokio::time::timeout(Duration::from_secs(600), run_session(mine, &a.replica, ws))
        .await
        .expect("the session hung behind a peer that stopped reading")
        .unwrap_err();
    assert!(
        matches!(&err, Error::Protocol(why) if why.contains("no progress")),
        "got {err:?}"
    );
    drop((r, w));
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_finishes_and_stops_reading_does_not_hang_the_close() {
    let ws = grain_id::GrainId::random();
    let a = replica();
    let content = patterned(2 * CHUNK);
    let hash = ContentHash::of_bytes(&content);
    std::fs::write(a.root.join("big.bin"), &content).unwrap();
    a.replica.lock().await.scan().unwrap();

    // The peer asks for the file and completes its side of the exchange, then holds the
    // stream open without reading: the session is done, but its writer is stuck on a tail
    // far larger than the pipe.
    let (mine, r, mut w) = fake_peer();
    write_control(&mut w, &hello(ws)).await.unwrap();
    write_control(&mut w, &Message::Want(hash)).await.unwrap();
    write_control(&mut w, &Message::Done).await.unwrap();
    write_control(&mut w, &Message::Settled).await.unwrap();

    let started = tokio::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(600), run_session(mine, &a.replica, ws))
        .await
        .expect("closing waited on a writer the peer never drains")
        .unwrap();
    assert!(
        started.elapsed() >= IDLE_TIMEOUT,
        "gave up on the writer after {:?}, while it might still have moved",
        started.elapsed()
    );
    drop((r, w));
}
