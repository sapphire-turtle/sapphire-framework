//! The one task that writes a session's half of the stream.
//!
//! Everything a side sends goes through an [`Outbox`] into two queues:
//!
//! - **bulk**, in order: content, `Updates` pages and `Done`. Order matters — an inline blob
//!   must reach the peer before the page that references it, and `Done` after the last page.
//! - **urgent**: every other control message (`Hello`, `Want`, `Missing`, `Settled`, `Live`,
//!   `Refused`).
//!
//! The writer prefers urgent, and drains it between the chunks of a blob, so a `Want` or a
//! `Settled` overtakes a large transfer instead of waiting behind it. One blob is in flight
//! at a time, so the receiver never has more than one partial file open.
//!
//! Content is streamed from the file named in [`Out::Blob`], one [`CHUNK`] at a time and
//! hashed as it goes: memory does not grow with the file, and a file that changed between
//! being located and being sent is answered with `Aborted` rather than wrong bytes.
//!
//! # Why the queues are unbounded
//!
//! Queueing never waits, so a session's read loop never waits on its own writer. A bound
//! would let a peer that stops reading stall the read loop behind a full queue, out of reach
//! of the idle timeout — and two honest peers each answering a long run of `Want`s could
//! deadlock, both readers waiting on full queues and both writers on the wire. What waits
//! in a queue is a page already in memory or the path of a file, never content, so the cost
//! is small; a peer that stops reading altogether is given up on by the idle timeout.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use sapphire_sync::{ContentHash, ContentHasher};
use tokio::io::AsyncWrite;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::CHUNK;
use crate::error::{Error, Result};
use crate::frame::{write_blob_chunk, write_blob_end, write_blob_start, write_control};
use crate::message::Message;
use crate::progress::Progress;

/// One thing to put on the wire.
pub(crate) enum Out {
    /// A control message.
    Control(Message),
    /// The content `hash`, read from `path`, which held `len` bytes when it was located.
    Blob {
        hash: ContentHash,
        path: PathBuf,
        len: u64,
    },
}

/// Where a session queues what it sends. Cloned freely; the writer exits once every clone
/// is gone and both queues are drained.
#[derive(Clone)]
pub(crate) struct Outbox {
    urgent: mpsc::UnboundedSender<Message>,
    bulk: mpsc::UnboundedSender<Out>,
}

impl Outbox {
    /// Queue `out` on the queue it belongs to, or report that the writer is gone.
    ///
    /// Never waits: the queues are unbounded (see the module docs).
    pub(crate) fn send(&self, out: Out) -> Result<()> {
        let sent = match out {
            Out::Control(msg @ (Message::Updates(_) | Message::Done)) => {
                self.bulk.send(Out::Control(msg)).is_ok()
            }
            Out::Control(msg) => self.urgent.send(msg).is_ok(),
            blob @ Out::Blob { .. } => self.bulk.send(blob).is_ok(),
        };
        if sent {
            Ok(())
        } else {
            Err(Error::Protocol("the stream closed".to_owned()))
        }
    }
}

/// Start the task that owns `w`, and return what feeds it.
///
/// The task ends once every [`Outbox`] clone is dropped and both queues are drained, or at
/// the first write that fails. `progress` is touched after everything it writes.
pub(crate) fn spawn_writer<W>(w: W, progress: Progress) -> (Outbox, JoinHandle<()>)
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (urgent_tx, urgent) = mpsc::unbounded_channel();
    let (bulk_tx, bulk) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        // A write error means the peer is gone; there is nobody to report it to.
        let _ = run(w, urgent, bulk, progress).await;
    });
    (
        Outbox {
            urgent: urgent_tx,
            bulk: bulk_tx,
        },
        task,
    )
}

async fn run<W>(
    mut w: W,
    mut urgent: mpsc::UnboundedReceiver<Message>,
    mut bulk: mpsc::UnboundedReceiver<Out>,
    progress: Progress,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    loop {
        tokio::select! {
            biased;
            Some(msg) = urgent.recv() => write_control(&mut w, &msg).await?,
            Some(out) = bulk.recv() => match out {
                Out::Control(msg) => write_control(&mut w, &msg).await?,
                Out::Blob { hash, path, len } => {
                    stream_blob(&mut w, &mut urgent, &progress, hash, &path, len).await?
                }
            },
            // Both queues closed and drained.
            else => break,
        }
        progress.touch();
    }
    Ok(())
}

/// Send the content `hash` from `path`: `BLOB_START`, its chunks, then `BLOB_END` if what
/// was read is `len` bytes hashing to `hash`, or `Aborted` if it is not.
///
/// Urgent control messages are written between chunks. A file that cannot be opened is
/// answered with `Missing` instead.
async fn stream_blob<W>(
    w: &mut W,
    urgent: &mut mpsc::UnboundedReceiver<Message>,
    progress: &Progress,
    hash: ContentHash,
    path: &Path,
    len: u64,
) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let Ok(mut file) = File::open(path) else {
        return write_control(w, &Message::Missing(hash)).await;
    };
    write_blob_start(w, &hash, len).await?;
    progress.touch();

    let mut buf = vec![0u8; CHUNK];
    let mut hasher = ContentHasher::new();
    let mut sent = 0u64;
    while sent < len {
        let want = (len - sent).min(CHUNK as u64) as usize;
        // A read error is the file changing under us too: what was sent will not match.
        let n = fill(&mut file, &mut buf[..want]).unwrap_or(0);
        if n == 0 {
            break;
        }
        while let Ok(msg) = urgent.try_recv() {
            write_control(w, &msg).await?;
            progress.touch();
        }
        hasher.update(&buf[..n]);
        write_blob_chunk(w, &buf[..n]).await?;
        sent += n as u64;
        progress.touch();
    }

    if sent == len && hasher.finish() == hash {
        write_blob_end(w).await
    } else {
        write_control(w, &Message::Aborted(hash)).await
    }
}

/// Read until `buf` is full or the file ends; how many bytes were read.
fn fill(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Frame, read_frame};
    use tokio::io::DuplexStream;

    /// A frame reduced to what a test compares.
    #[derive(Debug, PartialEq)]
    enum Seen {
        Start(ContentHash, u64),
        Chunk(usize),
        End,
        Control(String),
    }

    fn seen(frame: Frame) -> Seen {
        match frame {
            Frame::BlobStart { hash, len } => Seen::Start(hash, len),
            Frame::BlobChunk(bytes) => Seen::Chunk(bytes.len()),
            Frame::BlobEnd => Seen::End,
            Frame::Control(msg) => Seen::Control(format!("{msg:?}")),
        }
    }

    fn control(msg: &Message) -> Seen {
        Seen::Control(format!("{msg:?}"))
    }

    fn file_with(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn patterned(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    /// Read every frame until the writer closes the stream, keeping chunk bytes in order.
    async fn read_all(mut r: DuplexStream) -> (Vec<Seen>, Vec<u8>) {
        let mut frames = Vec::new();
        let mut content = Vec::new();
        while let Some(frame) = read_frame(&mut r).await.unwrap() {
            if let Frame::BlobChunk(bytes) = &frame {
                content.extend_from_slice(bytes);
            }
            frames.push(seen(frame));
        }
        (frames, content)
    }

    /// Queue `items`, drop the outbox, and read back everything the writer wrote.
    async fn send_all(items: Vec<Out>) -> (Vec<Seen>, Vec<u8>) {
        let (w, r) = tokio::io::duplex(64 * 1024);
        let (outbox, task) = spawn_writer(w, Progress::new());
        let reading = tokio::spawn(read_all(r));
        for item in items {
            outbox.send(item).unwrap();
        }
        drop(outbox);
        task.await.unwrap();
        reading.await.unwrap()
    }

    fn blob(hash: ContentHash, path: PathBuf, len: usize) -> Out {
        Out::Blob {
            hash,
            path,
            len: len as u64,
        }
    }

    #[tokio::test]
    async fn a_file_is_sent_as_start_chunks_end() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = patterned(2 * CHUNK + 1);
        let hash = ContentHash::of_bytes(&bytes);
        let path = file_with(&dir, "f", &bytes);

        let (frames, content) = send_all(vec![blob(hash, path, bytes.len())]).await;
        assert_eq!(
            frames,
            [
                Seen::Start(hash, bytes.len() as u64),
                Seen::Chunk(CHUNK),
                Seen::Chunk(CHUNK),
                Seen::Chunk(1),
                Seen::End,
            ]
        );
        assert!(content == bytes);
    }

    #[tokio::test]
    async fn a_file_of_exactly_one_chunk_sends_one_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = patterned(CHUNK);
        let hash = ContentHash::of_bytes(&bytes);
        let path = file_with(&dir, "f", &bytes);

        let (frames, _) = send_all(vec![blob(hash, path, CHUNK)]).await;
        assert_eq!(
            frames,
            [
                Seen::Start(hash, CHUNK as u64),
                Seen::Chunk(CHUNK),
                Seen::End
            ]
        );
    }

    #[tokio::test]
    async fn an_empty_file_sends_start_and_end() {
        let dir = tempfile::tempdir().unwrap();
        let hash = ContentHash::of_bytes(b"");
        let path = file_with(&dir, "f", b"");

        let (frames, _) = send_all(vec![blob(hash, path, 0)]).await;
        assert_eq!(frames, [Seen::Start(hash, 0), Seen::End]);
    }

    #[tokio::test]
    async fn a_changed_file_is_aborted() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = patterned(CHUNK + 10);
        let path = file_with(&dir, "f", &bytes);
        // Located when it held other bytes of the same length.
        let hash = ContentHash::of_bytes(&vec![0u8; bytes.len()]);

        let (frames, _) = send_all(vec![blob(hash, path, bytes.len())]).await;
        assert_eq!(
            frames,
            [
                Seen::Start(hash, bytes.len() as u64),
                Seen::Chunk(CHUNK),
                Seen::Chunk(10),
                control(&Message::Aborted(hash)),
            ]
        );
    }

    #[tokio::test]
    async fn a_vanished_file_is_answered_missing() {
        let dir = tempfile::tempdir().unwrap();
        let hash = ContentHash::of_bytes(b"gone");
        let path = dir.path().join("never-written");

        let (frames, _) = send_all(vec![blob(hash, path, 4)]).await;
        assert_eq!(frames, [control(&Message::Missing(hash))]);
    }

    #[tokio::test]
    async fn urgent_control_overtakes_a_blob() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = patterned(8 * CHUNK);
        let hash = ContentHash::of_bytes(&bytes);
        let path = file_with(&dir, "f", &bytes);
        let want = Message::Want(ContentHash::of_bytes(b"other"));

        // A pipe that holds one chunk: the writer is still mid-blob when the `Want` comes.
        let (w, mut r) = tokio::io::duplex(CHUNK);
        let (outbox, task) = spawn_writer(w, Progress::new());
        outbox.send(blob(hash, path, bytes.len())).unwrap();

        let first = read_frame(&mut r).await.unwrap().unwrap();
        assert!(matches!(first, Frame::BlobStart { .. }), "got {first:?}");
        outbox.send(Out::Control(want.clone())).unwrap();
        drop(outbox);

        let (frames, _) = read_all(r).await;
        task.await.unwrap();
        let at = |s: &Seen| frames.iter().position(|f| f == s).unwrap();
        assert!(at(&control(&want)) < at(&Seen::End), "{frames:?}");
    }

    #[tokio::test]
    async fn bulk_keeps_its_order() {
        let dir = tempfile::tempdir().unwrap();
        let hash = ContentHash::of_bytes(b"small");
        let path = file_with(&dir, "f", b"small");

        let (frames, _) = send_all(vec![
            blob(hash, path, 5),
            Out::Control(Message::Updates(vec![])),
            Out::Control(Message::Done),
        ])
        .await;
        assert_eq!(
            frames,
            [
                Seen::Start(hash, 5),
                Seen::Chunk(5),
                Seen::End,
                control(&Message::Updates(vec![])),
                control(&Message::Done),
            ]
        );
    }

    #[tokio::test]
    async fn the_writer_drains_both_queues_before_exiting() {
        let dir = tempfile::tempdir().unwrap();
        let hash = ContentHash::of_bytes(b"small");
        let path = file_with(&dir, "f", b"small");
        let want = Message::Want(hash);

        let (frames, _) = send_all(vec![
            Out::Control(Message::Updates(vec![])),
            Out::Control(want.clone()),
            blob(hash, path, 5),
            Out::Control(Message::Settled),
            Out::Control(Message::Done),
        ])
        .await;
        for expected in [
            control(&Message::Updates(vec![])),
            control(&want),
            Seen::End,
            control(&Message::Settled),
            control(&Message::Done),
        ] {
            assert!(frames.contains(&expected), "{expected:?} in {frames:?}");
        }
        assert_eq!(frames.len(), 7, "{frames:?}");
    }
}
