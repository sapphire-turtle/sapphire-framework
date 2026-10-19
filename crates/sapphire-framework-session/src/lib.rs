//! The replication session two sapphire-framework replicas run over a byte stream.
//!
//! One `Replica` per side, one bidirectional stream, and a framed exchange of version
//! vectors, path updates and content. This crate is the only place that knows the wire
//! format; it neither knows nor cares what carries the bytes.
//!
//! Two shapes of session are built on that exchange:
//!
//! - [`run_session`] — a one-shot catch-up. It returns once both sides are caught up.
//! - [`open_live_session`] — the same catch-up, then a stream that stays open: each side
//!   pushes what it commits and applies what arrives.
//!
//! See `docs/superpowers/specs/2026-09-15-p2p-sync-iroh-design.md` §3.7 and §4.2.

#![warn(missing_docs)]

mod error;
mod frame;
mod incoming;
mod live;
mod message;
mod progress;
mod session;
mod writer;

pub use error::{Error, Result};
pub use frame::{
    Frame, read_frame, read_framed, write_blob_chunk, write_blob_end, write_blob_start,
    write_control, write_framed,
};
pub use live::{LiveSession, open_live_session};
pub use message::Message;
pub use session::{HELLO_TIMEOUT, SessionOutcome, run_session};

use std::time::Duration;

/// The session format this build speaks. Sent in the first message and checked.
///
/// 2 carries content as `BLOB_START`, chunks and `BLOB_END`; a format 1 peer is refused.
pub const SESSION_FORMAT_VERSION: u32 = 2;

/// Largest payload carried inline in an `Updates` page, in bytes. Anything larger is
/// fetched by hash within the same session.
pub const INLINE_LIMIT: usize = 64 * 1024;

/// Longest control frame payload accepted, in bytes. Bounds the memory one peer can make
/// the other allocate for one message.
///
/// Not tied to the largest file: content travels in chunks of at most [`CHUNK`], and a
/// whole blob is bounded by the receiver's `max_file_size` instead.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

/// Most content one `BLOB_CHUNK` frame carries, in bytes. The sender reads, and the
/// receiver holds, at most this much of a file at a time.
pub const CHUNK: usize = 256 * 1024;

/// How long the initial exchange goes without progress before it gives up on the peer.
///
/// Progress is a frame read from the peer or a frame written to it, and both directions
/// count: a side sending a 1 GiB file hears nothing from the peer for as long as that
/// takes, and must not give up on it for that. A bound on silence rather than on total
/// time is what lets one large file take minutes while a peer that has stopped altogether
/// is still let go of.
///
/// Progress counts whole frames, so a peer draining slower than about one [`CHUNK`] per
/// `IDLE_TIMEOUT` (some 4 KiB/s) is given up on even though bytes are still moving.
///
/// The `Hello` wait has its own bound, [`HELLO_TIMEOUT`], and the live phase after the
/// exchange has none.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
