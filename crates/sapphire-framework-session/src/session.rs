//! One replication session over one stream.
//!
//! Both sides run this same function. It is symmetric: each sends what the other lacks and
//! applies what it is sent, and neither is a client.
//!
//! The exchange, in order:
//!
//! 1. `Hello` — format, workspace, replica id and the sender's version vector. A format
//!    mismatch, or a different workspace, ends the session with `Refused` before any state
//!    is exchanged.
//! 2. Every path state the peer's version vector does not cover, in pages of [`PAGE`], each
//!    page preceded by the content it inlines (64 KiB or less, and within the peer's
//!    `max_file_size`), then `Done`.
//! 3. `Want` for content that was not inlined, answered with the content or `Missing`. Once
//!    the peer's `Done` is in, also `Want` for content earlier sessions never got
//!    ([`Replica::unavailable`]).
//! 4. `Settled`, once the sender will ask for nothing more.
//! 5. `commit_session` last. An interrupted session leaves the version vector where it was,
//!    and everything is simply resent next time. A completed one commits even when some
//!    content was `Missing`: the paths are recorded, and step 3 of every later session asks
//!    for their content again, from whichever peer it meets.
//!
//! # Locking and giving up
//!
//! The replica is locked per step, not across the exchange: to build this side's pages, to
//! apply each page that arrives (and work out what it still needs), to answer each `Want`,
//! and to materialise at the end. Content frames take no lock at all — they go straight to
//! the staging directory — so a scan, or another session, runs while a large file is in
//! flight. That is safe because `apply` joins path states, which commutes with a scan's own
//! records, and committing only merges the peer's version vector.
//!
//! With one file able to take minutes, the exchange is bounded by silence, not by total
//! time: it gives up once nothing has been read or written for [`IDLE_TIMEOUT`].
//!
//! The exchange is one function, [`initial_exchange`], because two callers start here:
//! `run_session` finishes a session from what it returns, and `open_live_session` (in
//! `live.rs`) keeps the stream and applies `Live` pushes on it.
//!
//! # Why there are two end markers
//!
//! `Done` says "my pages are complete" and is sent as soon as they are queued, so the peer
//! can start asking for content without waiting for anything. That leaves a race worth
//! naming: a peer that has just applied a page and is about to send `Want` looks exactly
//! like a peer that needs nothing. A side that stopped reading at `Done` would therefore
//! drop the `Want` for every file over [`INLINE_LIMIT`] — and, with it, the file. `Settled`
//! closes the race. It is sent once the sender's want list can no longer grow (the peer's
//! pages are in and every hash asked for has been answered), so the receiver knows no `Want`
//! can follow, and both sides exit only after seeing the peer's `Done` *and* `Settled`.
//!
//! Content is addressed by its hash, so a page's inline blobs may travel in front of the page
//! that references them. They do: a receiver that had already applied the page would
//! otherwise ask for content that is on the wire, turning every inlined file into a `Want`
//! and a second copy of its bytes.
//!
//! # Writers
//!
//! Everything this side sends goes through one task (`writer.rs`), so the read loop never
//! blocks on a write, and its queues are unbounded, so queueing never waits on that task
//! either. Two `run_session` calls facing each other would otherwise deadlock the moment
//! both wrote more than the socket buffer, or filled a queue, while each waited to read.
//!
//! # Content
//!
//! Content is streamed from the file that holds it and received straight into the staging
//! directory (`incoming.rs`), so neither side holds a whole file in memory. Applying a page
//! finds content there, which is why `apply` and `fetch_missing` are given [`NoContent`].

use std::collections::HashSet;

use grain_id::GrainId;
use sapphire_sync::{
    Content, ContentHash, ContentSource, PathUpdate, Replica, Report, VersionVector,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadHalf};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::error::{Error, Result};
use crate::frame::{Frame, read_frame};
use crate::incoming::Incoming;
use crate::message::Message;
use crate::progress::Progress;
use crate::writer::{Out, Outbox, spawn_writer};
use crate::{IDLE_TIMEOUT, INLINE_LIMIT, SESSION_FORMAT_VERSION};

/// How many path states travel in one `Updates` message.
const PAGE: usize = 256;

/// How long the exchange waits for the peer's `Hello`.
///
/// The one wait in a session with nothing behind it. Every later read is answering something
/// the peer asked for or was told, and a peer that stops mid-exchange ends the read with end
/// of file. The `Hello` is different: the stream is open, the peer has its end, and it may
/// simply never speak — a bridge that parked the stream for an app server which never
/// claimed it, or a peer that went away between the dial and its first frame on a transport
/// with no half-close to report it with.
///
/// Waiting for ever is not an option: one silent peer would pin a session, its stream and
/// whatever drives it for the rest of the process's life. The same reasoning
/// [`IDLE_TIMEOUT`] applies to the rest of the exchange.
///
/// Generous on purpose, and a bound rather than a pace: a peer's bridge may hold a stream
/// parked for about a minute while it starts a stopped app server, which is waiting that is
/// working as intended, and nothing waits this out when the peer answers.
pub const HELLO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// What a session did.
#[derive(Clone, Debug, Default)]
pub struct SessionOutcome {
    /// Path updates sent.
    pub sent: usize,
    /// Path updates received.
    pub received: usize,
    /// What applying them did.
    pub report: Report,
}

/// The content source a session hands the replica: none.
///
/// Received content is already in the staging directory by the time a page is applied,
/// and the replica looks there first.
pub(crate) struct NoContent;

impl ContentSource for NoContent {
    fn locate(&self, _: &ContentHash) -> Option<std::path::PathBuf> {
        None
    }
}

/// Why the read loop stopped.
pub(crate) enum Stop {
    /// The peer's `Done` and `Settled` are in and this side wants nothing more.
    Complete,
    /// The stream ended; the peer is gone.
    Gone,
    /// The peer refused to continue, and why.
    Refused(String),
    /// The peer sent something this session does not allow.
    Protocol(String),
}

/// Everything the initial exchange left behind, for the caller that decides what happens
/// next.
pub(crate) struct Exchange<S> {
    /// The peer's half of the stream, still to be read.
    pub(crate) reader: ReadHalf<S>,
    /// Everything this side sends goes through here, and one task writes it.
    pub(crate) out: Outbox,
    /// The task owning the write half.
    pub(crate) writer: JoinHandle<()>,
    /// When the session last moved bytes, marked by the writer task and the read loop.
    pub(crate) progress: Progress,
    /// Everything the peer said it has, as of its `Hello`.
    pub(crate) peer_vv: VersionVector,
    /// Everything this side had when it worked out what to send: the peer has all of it
    /// once the exchange completes.
    pub(crate) sent_vv: VersionVector,
    /// The content being received, if any, carried on into the live phase.
    pub(crate) incoming: Incoming,
    /// What the exchange did.
    pub(crate) outcome: SessionOutcome,
    /// How the read loop stopped.
    pub(crate) stop: Stop,
}

/// Run a session to completion.
///
/// Returns once both sides have said `Done` and `Settled` and every piece of content this
/// side asked for has been answered — with the bytes, or with `Missing`. A stream that ends
/// first returns without an error, but commits nothing.
///
/// `replica` is locked for each step that touches it, never across a wait on the peer — not
/// for its `Hello` (issue #163), and not while content is in flight. A caller that used to
/// pre-lock a `Mutex<Replica>` around the whole call should stop: passing the mutex itself
/// is what leaves the replica free for a scan or another session meanwhile.
///
/// A peer that stops making progress is given up on after [`IDLE_TIMEOUT`] with
/// [`Error::Protocol`]; nothing bounds the session's total length.
///
/// `S` must be `'static` because the write half is moved into a task; a stream owned by the
/// caller (a socket, a spliced pipe, a duplex) is.
pub async fn run_session<S>(
    stream: S,
    replica: &Mutex<Replica>,
    workspace_id: GrainId,
) -> Result<SessionOutcome>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let Exchange {
        out,
        writer,
        progress,
        peer_vv,
        outcome,
        stop,
        ..
    } = initial_exchange(stream, replica, workspace_id).await?;
    match stop {
        Stop::Complete => {
            // The peer is still reading: it exits only once it has seen this side's
            // `Settled`, which is queued but not yet written. Draining the writer delivers
            // `Done`, `Settled` and every pending answer.
            let result = {
                let mut locked = replica.lock().await;
                materialise(&mut locked, &peer_vv, outcome)
            };
            close(out, writer, &progress, result).await
        }
        // The peer is gone or has misbehaved, so there is nothing to deliver and a write
        // may never complete. Abort: nothing is committed either way, and everything is
        // resent next session.
        Stop::Gone => {
            abort(out, writer);
            Ok(outcome)
        }
        Stop::Refused(why) => {
            abort(out, writer);
            Err(Error::Refused(why))
        }
        Stop::Protocol(why) => {
            abort(out, writer);
            Err(Error::Protocol(why))
        }
    }
}

/// Run the initial exchange: `Hello`, this side's pages, and reading until the peer's
/// `Done` and `Settled` are in and every hash this side asked for has been answered.
///
/// Both session kinds start here. Nothing is committed: the caller decides, so that an
/// interrupted exchange leaves the version vector where it was. An error means the exchange
/// never reached its end markers, and `run_session`'s own error mapping is the caller's job.
///
/// `replica` is locked per step, never across a wait on the peer: briefly at the very start,
/// to read the two values this side's own `Hello` needs (issue #163); once the peer's `Hello`
/// is in, to build this side's pages; then for each page that arrives (`apply` and
/// `needed`) and each `Want` (`content_path`). Content frames go to staging and take no
/// lock, so a scan or another session gets through while a large file is in flight.
///
/// After the `Hello`, the read loop gives up once nothing has been read or written for
/// [`IDLE_TIMEOUT`].
pub(crate) async fn initial_exchange<S>(
    stream: S,
    replica: &Mutex<Replica>,
    workspace_id: GrainId,
) -> Result<Exchange<S>>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (mut reader, writer) = tokio::io::split(stream);

    // The writer task owns the write half, so the read loop is never blocked by a peer that
    // is slow to read. It marks `progress` as it writes, and the read loop as it reads.
    let progress = Progress::new();
    let (out_tx, writer_task) = spawn_writer(writer, progress.clone());

    // 1. Hello, before anything else. Reading `replica_id`/`vv` is instantaneous, so the
    // lock is released again immediately — nothing below here needs the replica until the
    // peer's own `Hello` is in.
    let (replica_id, vv, max_file_size) = {
        let locked = replica.lock().await;
        (
            locked.replica_id(),
            locked.vv().clone(),
            locked.config().max_file_size,
        )
    };
    send(
        &out_tx,
        Out::Control(Message::Hello {
            format: SESSION_FORMAT_VERSION,
            workspace_id,
            replica_id,
            vv,
            max_file_size,
        }),
    )?;

    // A peer that never speaks is given up on rather than waited out: see [`HELLO_TIMEOUT`].
    // The queue is dropped rather than drained, because a peer that did not send a `Hello`
    // cannot be expected to be reading either.
    let hello = match tokio::time::timeout(HELLO_TIMEOUT, read_frame(&mut reader)).await {
        // Progress like any other frame read: the idle clock starts here, not at the
        // dial, so a `Hello` that took its time does not count against `IDLE_TIMEOUT`.
        Ok(frame) => {
            progress.touch();
            frame
        }
        Err(_) => {
            abort(out_tx, writer_task);
            return Err(Error::Protocol(format!(
                "the peer sent no Hello within {HELLO_TIMEOUT:?}"
            )));
        }
    };

    let (peer_vv, peer_max) = match hello? {
        Some(Frame::Control(Message::Hello {
            format,
            workspace_id: theirs,
            vv,
            max_file_size,
            ..
        })) => {
            if format != SESSION_FORMAT_VERSION {
                let _ = send(
                    &out_tx,
                    Out::Control(Message::Refused(format!(
                        "session format {format}, expected {SESSION_FORMAT_VERSION}"
                    ))),
                );
                return refuse(
                    out_tx,
                    writer_task,
                    Error::VersionMismatch {
                        ours: SESSION_FORMAT_VERSION,
                        theirs: format,
                    },
                )
                .await;
            }
            if theirs != workspace_id {
                let _ = send(
                    &out_tx,
                    Out::Control(Message::Refused(format!(
                        "this stream is for workspace {workspace_id}, not {theirs}"
                    ))),
                );
                return refuse(
                    out_tx,
                    writer_task,
                    Error::Protocol(format!(
                        "workspace mismatch: the peer is syncing {theirs}, this session is for \
                     {workspace_id}"
                    )),
                )
                .await;
            }
            (vv, max_file_size)
        }
        Some(Frame::Control(Message::Refused(why))) => {
            return refuse(out_tx, writer_task, Error::Refused(why)).await;
        }
        other => {
            return refuse(
                out_tx,
                writer_task,
                Error::Protocol(format!("expected Hello, got {other:?}")),
            )
            .await;
        }
    };

    // 2. Build the outgoing side as owned data, then hand it to a task. Computing it up
    //    front, in one short hold of the lock, keeps the read loop free while it is written.
    let (mut incoming, sent_vv, sent, outgoing) = {
        let replica = replica.lock().await;
        let incoming = Incoming::new(replica.stager());
        let delta = replica
            .delta_for(&peer_vv)
            .map_err(|e| Error::Sync(e.to_string()))?;
        // Small content goes inline, ahead of the page that needs it — but none over the
        // peer's size limit, which it would refuse. Looked up in one batch: a lookup per
        // hash would rebuild the store index each time.
        let inline_max = (INLINE_LIMIT as u64).min(peer_max);
        let inline = replica
            .content_paths(&inline_hashes(&delta, inline_max))
            .unwrap_or_default();
        let mut outgoing: Vec<Out> = Vec::new();
        for page in delta.chunks(PAGE) {
            for hash in inline_hashes(page, inline_max) {
                if let Some((path, len)) = inline.get(&hash) {
                    outgoing.push(Out::Blob {
                        hash,
                        path: path.clone(),
                        len: *len,
                    });
                }
            }
            outgoing.push(Out::Control(Message::Updates(page.to_vec())));
        }
        outgoing.push(Out::Control(Message::Done));
        (incoming, replica.vv().clone(), delta.len(), outgoing)
    };

    // 3. Read until the peer is done and settled and every hash we asked for has been
    //    answered.
    let mut wanted: HashSet<ContentHash> = HashSet::new();
    // Hashes the peer answered without the content this session; not asked for again.
    let mut lacked: HashSet<ContentHash> = HashSet::new();
    let mut report = Report::default();
    let mut received_count = 0usize;
    let mut peer_done = false;
    let mut peer_settled = false;
    let mut settled = false;

    // Every error out of the loop comes out of this block, so that one place below takes the
    // writer down with it: the peer cannot be expected to read what is still queued.
    let looped: Result<Stop> = async {
        for item in outgoing {
            send(&out_tx, item)?;
        }
        Ok(loop {
            if peer_done && peer_settled && wanted.is_empty() {
                break Stop::Complete;
            }
            let Some(frame) = read_or_idle(&mut reader, &progress).await? else {
                break Stop::Gone;
            };
            match frame {
                Frame::Control(Message::Updates(updates)) => {
                    received_count += updates.len();
                    // The join is idempotent, so a redelivered page costs nothing.
                    let need = {
                        let mut replica = replica.lock().await;
                        let page = replica
                            .apply(&updates, &NoContent)
                            .map_err(|e| Error::Sync(e.to_string()))?;
                        merge_report(&mut report, page);
                        needed(&replica, &updates)?
                    };
                    // Ask for content the page needs that this session does not have.
                    for hash in need {
                        if wanted.insert(hash) {
                            send(&out_tx, Out::Control(Message::Want(hash)))?;
                        }
                    }
                }
                Frame::Control(Message::Done) => {
                    peer_done = true;
                    // Content an earlier session never got: its paths were committed all the
                    // same, so no page carries them again (#297). Asked for once the peer's
                    // pages and the content they inline are all in, so nothing on the wire is
                    // asked for twice.
                    let again = replica
                        .lock()
                        .await
                        .unavailable()
                        .map_err(|e| Error::Sync(e.to_string()))?;
                    for hash in again {
                        if !lacked.contains(&hash) && wanted.insert(hash) {
                            send(&out_tx, Out::Control(Message::Want(hash)))?;
                        }
                    }
                }
                Frame::Control(Message::Settled) => peer_settled = true,
                Frame::Control(Message::Want(hash)) => {
                    let reply = answer(&*replica.lock().await, hash);
                    send(&out_tx, reply)?;
                }
                Frame::Control(Message::Missing(hash)) => {
                    // The peer does not have it either. Another peer may: the path stays
                    // unmaterialised, and later sessions ask for it again.
                    wanted.remove(&hash);
                    lacked.insert(hash);
                }
                Frame::Control(Message::Aborted(hash)) => {
                    // The peer's file changed under it: as good as `Missing`.
                    if let Err(e) = incoming.aborted(hash) {
                        break Stop::Protocol(reason(e));
                    }
                    wanted.remove(&hash);
                    lacked.insert(hash);
                }
                Frame::Control(Message::Live(_)) => {
                    // A `Live` push travels only after both sides have left the exchange, and
                    // frames from one peer arrive in order, so a compliant peer cannot send one
                    // here.
                    break Stop::Protocol(
                        "a Live push before the initial exchange finished".to_owned(),
                    );
                }
                Frame::Control(Message::Refused(why)) => {
                    break Stop::Refused(why);
                }
                Frame::Control(Message::Hello { .. }) => {
                    break Stop::Protocol("a second Hello".to_owned());
                }
                // Content is verified before it is staged: it is addressed by hash, so a peer that
                // sent the wrong bytes under one must not be able to plant them.
                Frame::BlobStart { hash, len } => {
                    if let Err(e) = incoming.start(hash, len) {
                        break Stop::Protocol(reason(e));
                    }
                }
                Frame::BlobChunk(bytes) => {
                    if let Err(e) = incoming.chunk(&bytes) {
                        break Stop::Protocol(reason(e));
                    }
                }
                Frame::BlobEnd => match incoming.end() {
                    Ok(hash) => {
                        wanted.remove(&hash);
                    }
                    Err(e) => break Stop::Protocol(reason(e)),
                },
            }
            // The peer's pages are all in, so this side's want list can no longer grow: settling
            // tells the peer it may close as soon as it wants nothing more either.
            if peer_done && wanted.is_empty() && !settled {
                settled = true;
                send(&out_tx, Out::Control(Message::Settled))?;
            }
        })
    }
    .await;
    let stop = match looped {
        Ok(stop) => stop,
        // The writer failed to write, so the stream is gone: the same end as the read side
        // seeing it close, whichever of the two notices first.
        Err(Error::Closed) => Stop::Gone,
        Err(e) => {
            abort(out_tx, writer_task);
            return Err(e);
        }
    };

    Ok(Exchange {
        sent_vv,
        reader,
        out: out_tx,
        writer: writer_task,
        progress,
        peer_vv,
        incoming,
        outcome: SessionOutcome {
            sent,
            received: received_count,
            report,
        },
        stop,
    })
}

/// Read the peer's next frame, or fail once the session has made no progress — nothing
/// read and nothing written — for [`IDLE_TIMEOUT`].
///
/// The writer task marks `progress` as it writes, so a side busy sending a large file is
/// not cut off for hearing nothing back. Each frame read marks it too.
pub(crate) async fn read_or_idle<R: AsyncRead + Unpin>(
    r: &mut R,
    progress: &Progress,
) -> Result<Option<Frame>> {
    // `read_frame` is not cancel-safe: dropping it mid-frame would lose the bytes it has
    // read. One pinned future survives every re-armed deadline below.
    let read = read_frame(r);
    tokio::pin!(read);
    loop {
        let deadline = Instant::now() + IDLE_TIMEOUT.saturating_sub(progress.idle_for());
        tokio::select! {
            frame = &mut read => {
                progress.touch();
                return frame;
            }
            // The writer may have moved since the deadline was set; only a full
            // `IDLE_TIMEOUT` of nothing ends the session.
            _ = tokio::time::sleep_until(deadline) => {
                if progress.idle_for() >= IDLE_TIMEOUT {
                    return Err(Error::Protocol(format!("no progress for {IDLE_TIMEOUT:?}")));
                }
            }
        }
    }
}

/// Materialise what was waiting on content, then commit.
///
/// Committing is what makes this replica claim it has what the peer had. It runs last, and
/// only when the peer said `Done` and `Settled`; an interrupted session leaves the version
/// vector where it was.
pub(crate) fn materialise(
    replica: &mut Replica,
    peer_vv: &VersionVector,
    mut outcome: SessionOutcome,
) -> Result<SessionOutcome> {
    let fetched = replica
        .fetch_missing(&NoContent)
        .map_err(|e| Error::Sync(e.to_string()))?;
    merge_report(&mut outcome.report, fetched);
    replica
        .commit_session(peer_vv)
        .map_err(|e| Error::Sync(e.to_string()))?;
    tracing::debug!(
        sent = outcome.sent,
        received = outcome.received,
        changed = outcome.report.changed,
        "session complete"
    );
    Ok(outcome)
}

/// Queue one frame, or report that the writer is gone.
pub(crate) fn send(out: &Outbox, item: Out) -> Result<()> {
    out.send(item)
}

/// The answer to the peer's `Want(hash)`: the content, streamed from the file that holds
/// it, or `Missing`.
pub(crate) fn answer(replica: &Replica, hash: ContentHash) -> Out {
    match replica.content_path(&hash) {
        Ok(Some((path, len))) => Out::Blob { hash, path, len },
        _ => Out::Control(Message::Missing(hash)),
    }
}

/// What a [`Stop::Protocol`] says, from the error that stopped the read loop.
pub(crate) fn reason(err: Error) -> String {
    match err {
        Error::Protocol(why) => why,
        other => other.to_string(),
    }
}

/// Close the outgoing side, let the writer flush what is queued, and return `result`.
///
/// Dropping the sender is what ends the writer task; it drains first, so a `Refused` or a
/// `Settled` queued moments ago still reaches the peer before the stream goes away.
///
/// The wait is bounded like the exchange: a peer that holds the stream open but stops
/// reading would otherwise keep the writer, and this call, waiting for ever. Once the
/// writer has moved nothing for [`IDLE_TIMEOUT`] it is aborted; `result` stands either
/// way, since the session's work is already done.
async fn close(
    out_tx: Outbox,
    mut writer: JoinHandle<()>,
    progress: &Progress,
    result: Result<SessionOutcome>,
) -> Result<SessionOutcome> {
    drop(out_tx);
    loop {
        let deadline = Instant::now() + IDLE_TIMEOUT.saturating_sub(progress.idle_for());
        tokio::select! {
            _ = &mut writer => break,
            // The writer may have moved since the deadline was set; only a full
            // `IDLE_TIMEOUT` of nothing gives up on it.
            _ = tokio::time::sleep_until(deadline) => {
                if progress.idle_for() >= IDLE_TIMEOUT {
                    tracing::debug!("the peer stopped reading; dropping what was still queued");
                    writer.abort();
                    break;
                }
            }
        }
    }
    result
}

/// Send a `Refused` that is already queued, close the writer, and return `err`.
///
/// The exchange's own early exits: the peer is still reading, so what was queued has to
/// reach it before the stream goes away.
async fn refuse<S>(out_tx: Outbox, writer: JoinHandle<()>, err: Error) -> Result<Exchange<S>> {
    drop(out_tx);
    let _ = writer.await;
    Err(err)
}

/// Close the outgoing side without waiting to deliver what is queued.
///
/// Used only once the peer can no longer be expected to read, where a write may otherwise
/// never complete.
pub(crate) fn abort(out_tx: Outbox, writer: JoinHandle<()>) {
    drop(out_tx);
    writer.abort();
}

/// The hashes of the content in `updates` of `max` bytes or less, to send inline.
fn inline_hashes(updates: &[PathUpdate], max: u64) -> Vec<ContentHash> {
    updates
        .iter()
        .flat_map(|update| &update.versions)
        .filter_map(|entry| match entry.content {
            Content::File { hash, len } if len <= max => Some(hash),
            _ => None,
        })
        .collect()
}

/// Hashes these updates need that neither a file on disk nor the staging directory holds.
///
/// Content over this replica's size limit is never needed: it is recorded but not
/// written, and the peer's `BLOB_START` for it would be refused as a protocol error.
pub(crate) fn needed(replica: &Replica, updates: &[PathUpdate]) -> Result<Vec<ContentHash>> {
    let max = replica.config().max_file_size;
    let hashes: Vec<ContentHash> = updates
        .iter()
        .flat_map(|update| &update.versions)
        .filter_map(|entry| match entry.content {
            Content::File { hash, len } if len <= max => Some(hash),
            _ => None,
        })
        .collect();
    // One batch per page: a lookup per hash would rebuild the store index each time.
    let have = replica
        .content_paths(&hashes)
        .map_err(|e| Error::Sync(e.to_string()))?;
    Ok(hashes
        .into_iter()
        .filter(|hash| !have.contains_key(hash))
        .collect())
}

pub(crate) fn merge_report(into: &mut Report, from: Report) {
    into.changed += from.changed;
    into.recorded.extend(from.recorded);
    into.conflicts.extend(from.conflicts);
    into.skipped.extend(from.skipped);
}
