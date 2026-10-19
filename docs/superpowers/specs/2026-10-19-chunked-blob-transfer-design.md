# Chunked content transfer, up to 1 GiB

- Issue: #292
- Crates: `sapphire-framework-session`, `sapphire-framework-sync`, `sapphire-framework-bridge`
  (`wgsync.rs` only)
- Status: accepted

## Why

Workspaces should carry images and audio: attachments in sapphire-journal and
sapphire-ledger, and sapphire-agent's reference voices, face photos and wake-word models.
The target is files up to **1 GiB**. Resuming an interrupted transfer is not required.

The replication path already treats every file as opaque bytes, so files up to 64 MiB sync
today. What stops larger ones:

- `DEFAULT_MAX_FILE_SIZE` is 64 MiB (`sapphire-framework-sync/src/replica.rs`).
- Content is read whole into memory to send (`read_content` → `fs::read`), and a file's
  content is one frame, read whole before it is handled (`frame.rs`).
- A session keeps everything it received in a `HashMap<ContentHash, Vec<u8>>` until it
  materialises (`Received` in `session.rs`).
- `MAX_FRAME_LEN` is also 64 MiB, and a content frame carries a 32-byte hash, so a file of
  exactly the size limit passes the size check but cannot be sent.
- The bridge's workgroup driver gives a whole session 60 seconds (`SESSION_TIMEOUT` in
  `wgsync.rs`). With no resume, a file that takes longer than that never arrives.
- The replica's lock is held for the whole initial exchange; until now the 60-second bound
  kept that short.

## Decisions

1. **Own implementation, not iroh-blobs.** Session streams reach the app server through
   the bridge, which splices one `sapphire/ws/1` stream opaquely (`bridge/data.rs`,
   `splice`). Content that travels inside that stream needs no bridge change. iroh-blobs
   needs its own ALPN and connection, which the bridge would have to route — and it is not
   at 1.0 either.
2. **Content is chunked on the existing session stream.** Not one stream per blob, and not
   a second session for content: both need the bridge to route more than one stream, and
   a second session splits "commit only once content is in" across two sessions.
3. **`SESSION_FORMAT_VERSION` becomes 2, with no v1 compatibility.** A v1 peer is refused
   at `Hello` as any format mismatch is today. Devices on mixed versions stop syncing (with
   an error, losing nothing) until all are upgraded.
4. **The replica lock is taken per control message, not across the exchange.** Chunks only
   touch the staging directory, so a scan is not held up for the length of a transfer.
5. **The session gives up on no progress, not on total time.** 60 seconds with nothing
   read *and* nothing written ends the initial exchange; the bridge's whole-session
   timeout goes.
6. **SHA-256 stays.** Content is verified after the last chunk, which is enough without
   resume; no bao/BLAKE3 verified streaming.

## 1. Wire format (`sapphire-framework-session`)

A frame keeps its shape: one tag byte, a four-byte big-endian length, then the payload.

| Tag | Name          | Payload                          |
|-----|---------------|----------------------------------|
| 0   | `CONTROL`     | a JSON `Message` (unchanged)     |
| 1   | `BLOB_START`  | `hash` (32 bytes) + `len` (u64 BE) |
| 2   | `BLOB_CHUNK`  | content bytes, at most `CHUNK` (256 KiB) |
| 3   | `BLOB_END`    | empty                            |

`Message` gains `Aborted(ContentHash)`: the sender started a blob it could not finish
(see §2).

Every piece of content travels as `BLOB_START`, chunks, `BLOB_END` (or `Aborted`),
whatever its size. Inlining — content of `INLINE_LIMIT` (64 KiB) or less sent ahead of the
page that references it — is kept; only its framing changes.

`MAX_FRAME_LEN` drops to **16 MiB** and now bounds control frames only (a page of 256 path
updates is far below it); `BLOB_CHUNK` frames are bounded by `CHUNK`. A blob's size is
bounded separately: `BLOB_START`'s `len` is checked against the receiver's
`max_file_size` before anything is written. The "exactly 64 MiB cannot be sent" problem
goes away with the separation.

`Frame::Blob { hash, bytes }` is replaced by `Frame::BlobStart { hash, len }`,
`Frame::BlobChunk(Vec<u8>)` and `Frame::BlobEnd`. `write_blob` is replaced by a writer
that streams a file (§2). `read_framed`/`write_framed` (used by pairing) keep their
signatures and the 16 MiB bound.

## 2. Sending

`Out::Blob(ContentHash, Vec<u8>)` becomes `Out::Blob { hash, path, len }`. The writer task
opens `path`, reads `CHUNK` bytes at a time, hashes as it reads, and writes `BLOB_START`,
the chunks, then:

- `BLOB_END` if what it read hashes to `hash` and is `len` bytes long;
- `Aborted(hash)` otherwise — the file changed between being located and being sent.

A file that cannot be opened is answered with `Missing(hash)` instead of a `BLOB_START`.

The writer has **two queues**: control and blob. It selects with control first, and checks
the control queue between chunks, so `Want`, `Settled` and `Live` overtake a large
transfer instead of waiting behind it. Exactly one blob is in flight at a time: the
receiver never has more than one partial file open.

Memory per session no longer depends on file size: one `CHUNK` buffer on each side, plus
the queues.

## 3. Receiving

The read loop keeps at most one blob open:

- `BLOB_START { hash, len }`: refused as a protocol error if a blob is already open or
  `len > max_file_size`. Otherwise opens a `StagingWriter` (§4).
- `BLOB_CHUNK`: refused if no blob is open or the running total would exceed `len`;
  otherwise appended and hashed.
- `BLOB_END`: refused if no blob is open. The writer's `finish` checks length and hash; a
  mismatch is a protocol error (the peer claimed bytes it did not send — the existing rule
  that a peer must not plant content under the wrong hash). On success `hash` leaves
  `wanted`.
- `Aborted(hash)`: the open blob's partial file is discarded and `hash` is treated as
  `Missing` — the path stays unmaterialised until a later session.
- `CONTROL` frames are handled as usual while a blob is open: the sender writes them whole
  between chunk frames, and that is what lets control messages overtake a transfer.

If the stream ends or the session errors with a blob open, the partial file is removed and
nothing is committed, as today.

`Received` goes. Content lands in the staging directory as it arrives, and `stage()` (§4)
finds it there, so `apply` and `fetch_missing` are passed an empty source by the session.
`needed()` returns the hashes that neither `content_path` nor the staging directory has.

`live.rs` changes the same way: its `Want` answer and blob handling use the same writer
and read-loop code as the exchange.

## 4. Replication core (`sapphire-framework-sync`)

### Locating content by path

`read_content(&hash) -> Result<Option<Vec<u8>>>` is replaced by
`content_path(&hash) -> Result<Option<PathBuf>>`: a workspace file or staged file that
holds `hash`.

- A workspace file is trusted when its current length and mtime match the path's
  recorded `DiskState` and the stamp is not racy; otherwise it is re-hashed with the
  streaming `ContentHash::of_file`. The sender's hash-while-sending (§2) is the final
  check, so no whole-file hash is needed here.
- A staged file (`staging/<hash>`) is trusted on existence: only `StagingWriter::finish`
  puts one there, and only after verifying it (rename is atomic).

`ContentSource` becomes:

```rust
pub trait ContentSource {
    /// A file holding `hash`, if this source has one.
    fn locate(&self, hash: &ContentHash) -> Option<PathBuf>;
}
```

`Replica` implements it with `content_path`. The test helper `MapSource` writes its entries
to a temporary directory and returns their paths.

### `StagingWriter`

One place writes into the staging directory:

```rust
impl Replica {
    pub fn staging_writer(&self, hash: ContentHash, len: u64) -> Result<StagingWriter>;
}
impl StagingWriter {
    pub fn write(&mut self, bytes: &[u8]) -> Result<()>; // appends and hashes; errs past `len`
    pub fn finish(self) -> Result<PathBuf>;              // checks len + hash, renames to staging/<hash>
}
// Drop without `finish` removes `staging/<hash>.partial`.
```

`StagingWriter` does not borrow the replica — it holds the staging directory path and
`max_file_size` — so the session can write chunks without the replica's lock.

Users:

- the session's read loop (§3);
- `stage()`, which copies a file `locate`d in another source through it;
- `ensure_conflict_copies`, whose `fs::read` + `fs::write` become a copy through it.

After this no code path holds a whole file in memory.

`Replica::open` removes leftover `*.partial` files in the staging directory.

### Limit

`DEFAULT_MAX_FILE_SIZE` becomes **1 GiB**. It stays configurable through
`ReplicaConfig::max_file_size`.

## 5. Locking and timeouts

### Lock per control message

`initial_exchange` today locks the replica once the peer's `Hello` is in and holds it to
the end. It changes to locking around each step that touches the replica: building this
side's delta, applying an `Updates` page (and computing `needed`), answering a `Want`
(`content_path`), and `materialise` at the end. Blob frames take no lock.

This lets a scan run in the middle of an exchange. That is expected to be safe — `apply`
joins path states, which commutes with a scan's local records, and `commit_session` only
merges the peer's version vector — but it is a change to a deliberate choice, so it is
proven by a test first (§6) before the rest of the work depends on it.

### Idle timeout

`IDLE_TIMEOUT` (60 s) bounds the initial exchange: the session records the time of its last
progress — a frame read, or a frame written by the writer task — and the exchange fails
with a protocol error once nothing has progressed for `IDLE_TIMEOUT`. Both directions count:
a side sending 1 GiB receives nothing from the peer for the duration, and must not time
out for it.

The live phase after the exchange has no idle timeout, as today.

`HELLO_TIMEOUT` (90 s) is unchanged.

`wgsync.rs`'s `SESSION_TIMEOUT` is removed; the session's own idle timeout replaces it.

## 6. Testing

Frames (`frame.rs`):

- `BLOB_START`, `BLOB_CHUNK`, `BLOB_END` and `Aborted` round-trip.
- A control frame over 16 MiB and a chunk over `CHUNK` are refused before allocating.

`StagingWriter`:

- Correct content is renamed into place; a wrong hash or length fails and leaves no
  `.partial`; dropping it unfinished leaves no `.partial`; `write` past `len` fails.
- `Replica::open` removes stale `.partial` files.

Session (two replicas over `tokio::io::duplex`):

- A file of exactly 64 MiB, and one well above it, sync. Size boundaries are also tested
  with a lowered `max_file_size` to keep the suite fast.
- Constant memory is guaranteed by types — `Out::Blob` carries a path, no `Vec` of content
  exists — not measured.
- A `Want` and `Settled` from the peer arrive while a large blob is being sent (control
  overtakes chunks).
- Rewriting the sender's file mid-transfer yields `Aborted`; the path stays unmaterialised
  and the next session delivers it.
- A stream cut mid-blob commits nothing and leaves no `.partial`.
- Content that does not match its hash is a protocol error.
- A blob whose `len` exceeds the receiver's `max_file_size` is refused.
- A v1 `Hello` is refused.

Locking (first, per §5):

- A scan completes while a blob is being received, and the two replicas converge after the
  session (following the pattern in `tests/convergence.rs`).

Timeouts:

- An exchange that keeps progressing past 60 s is not cut off; one with no progress for
  `IDLE_TIMEOUT` is. Uses paused tokio time, not real waiting.

Existing tests calling `fetch_missing(&a.replica)` or using `MapSource` move to the
path-based `ContentSource`.

## Out of scope

- Resuming an interrupted transfer.
- Files over 1 GiB (model weights and the like).
- Fetching one piece of content from several peers at once.
- Retrieval over binary content (#291).
