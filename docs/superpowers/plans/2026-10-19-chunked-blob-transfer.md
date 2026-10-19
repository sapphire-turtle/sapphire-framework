# Chunked Content Transfer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sync files up to 1 GiB by streaming content in 256 KiB chunks on the existing
session stream, with memory per session independent of file size.

**Architecture:** The replication core (`sapphire-framework-sync`) gains a streaming hasher,
a `Stager`/`StagingWriter` that verifies content into `staging/<hash>`, and locates content
by path instead of reading it into a `Vec`. The session (`sapphire-framework-session`)
replaces the single blob frame with `BLOB_START`/`BLOB_CHUNK`/`BLOB_END`, a two-queue writer
task (urgent control first, bulk in order) and one receive state machine shared by the
exchange and the live loop. The replica is locked per control message, the initial
exchange ends after 60 s without progress, and the bridge's whole-session timeout goes.

**Tech Stack:** Rust 2024, tokio (io-util, sync, time), sha2, tempfile (tests).

**Spec:** `docs/superpowers/specs/2026-10-19-chunked-blob-transfer-design.md`

## Global Constraints

- `SESSION_FORMAT_VERSION = 2`; a v1 `Hello` is refused (no compatibility path).
- `CHUNK = 256 * 1024` bytes, max `BLOB_CHUNK` payload.
- `MAX_FRAME_LEN = 16 * 1024 * 1024`, bounds `CONTROL` frames and `read_framed`/`write_framed`.
- `DEFAULT_MAX_FILE_SIZE = 1024 * 1024 * 1024`.
- `INLINE_LIMIT` stays `64 * 1024`.
- `IDLE_TIMEOUT = 60 s`, initial exchange only; `HELLO_TIMEOUT` stays 90 s.
- Frame tags: `0 CONTROL`, `1 BLOB_START` (32-byte hash + u64 BE len, exactly 40 bytes), `2 BLOB_CHUNK`, `3 BLOB_END` (empty).
- No code path may hold a whole file's content in memory.
- Work in worktree `../sapphire-framework-wt-blob` on branch `feat/chunked-blob-transfer`. Never `git add -A`; add named files.
- Every task ends with `cargo test` for the crates it touched, `cargo clippy -p <crate> --all-targets -- -D warnings` clean, and `cargo check --workspace --all-targets` passing.

## Review Focus

- A zero-length file: `BLOB_START len=0`, no chunks, `BLOB_END` → syncs (Task 2 writer and session tests).
- A file of exactly `CHUNK` bytes, and `2*CHUNK + 1` → no empty trailing chunk, last chunk 1 byte (Task 2 writer tests).
- The sender's file is deleted after `content_path` found it and before the writer opens it → `Missing`, not a dead session (Task 2 writer test).
- A disk write error mid-blob (staging dir removed) → an error, and no `.partial` left (Task 1 `StagingWriter` test).
- A peer sends `BLOB_START` while another blob is open, or `BLOB_CHUNK`/`BLOB_END` with none open → protocol error, not a panic (Task 2 `Incoming` tests).

---

## File Structure

`sapphire-framework-sync`
- `src/hash.rs` — add `ContentHasher` (streaming SHA-256).
- `src/staging.rs` (new) — `Stager`, `StagingWriter`, `clear_partials`.
- `src/replica.rs` — `stager()`, `content_path`, path-based `ContentSource`, `stage`/conflict copies via `StagingWriter`, 1 GiB default, partial cleanup in `open`.
- `src/error.rs` — `TooLarge`, `ContentMismatch`.
- `src/testing.rs` — `MapSource` backed by files.

`sapphire-framework-session`
- `src/frame.rs` — new frame kinds and per-tag bounds.
- `src/writer.rs` (new) — `Out`, `Outbox`, `spawn_writer` (two queues, file streaming, `Aborted`/`Missing`).
- `src/incoming.rs` (new) — `Incoming`, the receive state machine over a `Stager`.
- `src/progress.rs` (new) — `Progress`, `read_or_idle`.
- `src/session.rs`, `src/live.rs` — use the above; `Received` removed; per-step locking.
- `src/message.rs` — `Message::Aborted`.
- `src/lib.rs` — constants, exports.

`sapphire-framework-bridge`
- `src/wgsync.rs` — remove `SESSION_TIMEOUT`.

---

### Task 1: Hasher, staging writer and `content_path` (sync, additive)

**Files:**
- Modify: `crates/sapphire-framework-sync/src/hash.rs`, `src/error.rs`, `src/lib.rs`, `src/replica.rs`
- Create: `crates/sapphire-framework-sync/src/staging.rs` (unit tests inside)
- Test: `crates/sapphire-framework-sync/tests/replica_basic.rs`

**Interfaces:**
- Produces:
  - `pub struct ContentHasher` — `new() -> Self`, `update(&mut self, &[u8])`, `finish(self) -> ContentHash`. `ContentHash::of_file` is rewritten on top of it.
  - `Error::TooLarge { len: u64, max: u64 }`, `Error::ContentMismatch { expected: ContentHash }`.
  - `#[derive(Clone, Debug)] pub struct Stager` — `pub fn new(dir: PathBuf, max: u64) -> Self`; `pub fn writer(&self, hash: ContentHash, len: u64) -> Result<StagingWriter>` (`TooLarge` if `len > max`, checked before creating any file; creates `dir` if missing; creates a partial of its own, `<hex>.<pid>-<n>.partial` via `create_new`, so concurrent writers for one hash never share a file); `pub fn staged(&self, hash: &ContentHash) -> Option<PathBuf>` (existence of `<hex>` only).
  - `pub struct StagingWriter` — `write(&mut self, &[u8]) -> Result<()>` (`TooLarge` when the running total would pass `len`); `finish(self) -> Result<PathBuf>` (length then hash checked → `ContentMismatch`; renames `<hex>.partial` to `<hex>`); `impl std::io::Write` delegating to `write`; `Drop` removes the `.partial` unless finished.
  - `pub(crate) fn clear_partials(dir: &Path) -> std::io::Result<()>` — removes `*.partial`, keeps the rest; `NotFound` on `dir` is `Ok`. `Replica::open` calls it.
  - `Replica::stager(&self) -> Stager` from `config.staging_dir` and `config.max_file_size`.
  - `Replica::content_path(&self, hash: &ContentHash) -> Result<Option<(PathBuf, u64)>>`: for each path in `StoreIndex::paths_with(hash)`, the file is trusted when its current length and mtime (ns) equal the recorded `DiskState` and `!is_racy(&disk)`, otherwise trusted iff `ContentHash::of_file(..) == *hash`; then `stager().staged(hash)` with its length from metadata. `read_content` stays for now (Task 3 removes it).
  - Re-export `ContentHasher`, `Stager`, `StagingWriter` from `lib.rs`.

- [ ] **Step 1: Write failing tests**

`staging.rs`:
```rust
hasher_matches_of_bytes_across_split_updates()     // update(b"ab"); update(b"c") == of_bytes(b"abc")
finished_content_lands_at_its_hash()               // finish() == dir/<hex>; contents == bytes; no .partial
a_wrong_hash_is_refused_and_leaves_nothing()       // ContentMismatch; read_dir(dir) empty
a_short_write_is_refused_and_leaves_nothing()      // len 5, write 3 bytes, finish → ContentMismatch
writing_past_len_is_refused()                      // len 3, write 4 bytes → TooLarge
a_len_over_the_max_is_refused_before_any_file()    // Stager max 10, writer(len 11) → TooLarge; dir empty
dropping_unfinished_removes_the_partial()
a_zero_length_blob_finishes()                      // of_bytes(b""), len 0
staged_reports_only_finished_content()             // .partial present → None; after finish → Some
a_vanished_staging_dir_fails_and_leaves_no_partial() // remove_dir_all(dir) after writer(); write+finish → Err; nothing at dir
clear_partials_removes_only_partials()             // keeps <hex>, removes <hex>.partial
```
`tests/replica_basic.rs`:
```rust
open_clears_leftover_partials()                    // staging/x.partial, reopen replica → gone
content_path_trusts_an_unchanged_file()            // after scan: Some((root/a.bin, len))
content_path_rehashes_a_file_whose_stamp_changed() // same-length overwrite with other bytes → None
content_path_finds_staged_content()                // stager().writer(..).finish() → Some(staging/<hex>)
```

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-sync` — compile failure (types missing).
- [ ] **Step 3: Implement** the interfaces above. `StagingWriter` holds a `std::fs::File`, a `ContentHasher`, `written`, `len`, `hash`, the `.partial` path and `finished`.
- [ ] **Step 4: Run** `cargo test -p sapphire-framework-sync` — all pass; clippy clean.
- [ ] **Step 5: Commit** `feat(sync): stream-hash content into a verified staging writer`

---

### Task 2: Wire format v2 — chunked frames, two-queue writer, receive into staging (session)

One task because sending and receiving change format together: no intermediate state speaks
a coherent protocol.

**Files:**
- Modify: `crates/sapphire-framework-session/src/frame.rs`, `src/message.rs`, `src/lib.rs`, `src/session.rs`, `src/live.rs`
- Create: `src/writer.rs`, `src/incoming.rs`, `src/progress.rs` (unit tests inside each)
- Test: `crates/sapphire-framework-session/tests/blob.rs` (new), `tests/live.rs`

**Interfaces:**
- Consumes: `ContentHasher`, `Stager`, `StagingWriter`, `Replica::stager`, `Replica::content_path` (Task 1).
- Produces:
  - `lib.rs`: `pub const CHUNK: usize = 256 * 1024;`, `MAX_FRAME_LEN = 16 * 1024 * 1024`, `SESSION_FORMAT_VERSION = 2`; update the `MAX_FRAME_LEN` doc (no longer tied to the file size limit).
  - `Message::Aborted(ContentHash)` — "The sender started this content and could not finish it: the file changed under it."
  - `pub enum Frame { Control(Message), BlobStart { hash: ContentHash, len: u64 }, BlobChunk(Vec<u8>), BlobEnd }`.
  - `pub async fn write_blob_start(w, hash: &ContentHash, len: u64) -> Result<()>`, `pub async fn write_blob_chunk(w, bytes: &[u8]) -> Result<()>` (`FrameTooLarge` over `CHUNK`), `pub async fn write_blob_end(w) -> Result<()>`; `write_blob` removed. All exported (tests play a peer with them).
  - `read_frame` checks the announced length per tag **before allocating**: `CONTROL` ≤ `MAX_FRAME_LEN` and `BLOB_CHUNK` ≤ `CHUNK` (else `FrameTooLarge`); `BLOB_START` == 40 and `BLOB_END` == 0 (else `Protocol`).
  - `progress.rs`: `#[derive(Clone)] pub(crate) struct Progress` (an `Arc<AtomicU64>` of ms since a `tokio::time::Instant` it also holds), `new()`, `touch(&self)`. Task 4 adds the rest.
  - `writer.rs`:
    - `pub(crate) enum Out { Control(Message), Blob { hash: ContentHash, path: PathBuf, len: u64 } }`
    - `#[derive(Clone)] pub(crate) struct Outbox` with `pub(crate) async fn send(&self, out: Out) -> Result<()>` (`Protocol("the stream closed")` once the writer is gone). Routing: `Out::Blob`, `Control(Updates(_))`, `Control(Done)` → bulk; every other `Control` → urgent. Bound 64 each.
    - `pub(crate) fn spawn_writer<W: AsyncWrite + Unpin + Send + 'static>(w: W, progress: Progress) -> (Outbox, JoinHandle<()>)`.
  - `incoming.rs`: `pub(crate) struct Incoming` over a `Stager`, at most one open `(ContentHash, StagingWriter)`:
    - `new(stager: Stager)`; `start(&mut self, hash, len: u64) -> Result<()>` (`Protocol` if one is open; `TooLarge` → `Protocol("content of {len} bytes exceeds the {max}-byte limit")`); `chunk(&mut self, &[u8]) -> Result<()>` (`Protocol` if none open or past `len`); `end(&mut self) -> Result<ContentHash>` (`Protocol` if none open; mismatch → `Protocol("content does not match {hash}")`); `aborted(&mut self, hash) -> Result<()>` (`Protocol` unless `hash` is the open blob; discards it). Dropping it discards any open blob.
  - `session.rs`: `Received` is deleted. A private `struct NoContent` implementing today's `ContentSource` (returns `None`) is passed to `apply` and `fetch_missing` — received content is already staged. `needed(replica, updates) -> Result<Vec<ContentHash>>` returns the hashes whose `content_path` is `None`. `materialise(replica, peer_vv, outcome)` loses its `received` parameter. Inline blobs and `Want` answers become `Out::Blob { hash, path, len }` from `content_path`; `Missing` when it is `None`.
  - Both read loops (exchange and live) dispatch `BlobStart` → `start`, `BlobChunk` → `chunk`, `BlobEnd` → `end()` then treat the hash as answered (`wanted.remove`; live: `remember` and set `wrote_content`), `Control(Aborted(h))` → `aborted(h)` then treat as `Missing(h)`. An `Incoming` error stops the exchange as `Stop::Protocol` (live: log and break, as for today's mismatch).
  - `Exchange` and `LiveSession` hold an `Outbox` instead of `mpsc::Sender<Out>`; `send`/`abort`/`close`/`refuse` take it.

Writer loop — the one algorithm the tests do not pin:

```rust
loop {
    tokio::select! {
        biased;
        Some(msg) = urgent.recv() => write_control(&mut w, &msg).await?,
        Some(out) = bulk.recv() => match out {
            Out::Control(msg) => write_control(&mut w, &msg).await?,
            Out::Blob { hash, path, len } =>
                stream_blob(&mut w, &mut urgent, &progress, hash, &path, len).await?,
        },
        else => break, // both closed and drained
    }
    progress.touch();
}
```

`stream_blob`: open `path` with `std::fs::File`; on error write `Control(Missing(hash))` and
return. Write `BLOB_START`. Read up to `CHUNK` at a time into one reused buffer, never past
`len`, hashing as it goes; before each chunk, write every control message `urgent.try_recv()`
yields; write the chunk; `progress.touch()`. Stop at EOF or `len`. If the total is `len` and
the hash matches, `BLOB_END`; otherwise `Control(Aborted(hash))`. A write error ends the task,
as today.

- [ ] **Step 1: Write failing tests**

`frame.rs` (replace the old blob tests):
```rust
a_blob_start_round_trips()                               // len 1 << 30
a_blob_chunk_round_trips()                               // CHUNK bytes
a_blob_end_round_trips()
a_chunk_over_the_limit_is_refused_before_allocating()    // announced CHUNK + 1 → FrameTooLarge
a_blob_start_of_the_wrong_size_is_refused()              // 39-byte payload → Protocol
a_control_frame_over_16_mib_is_refused_before_allocating()
```
`message.rs`: `an_abort_round_trips()`.

`writer.rs` (write into one end of a `tokio::io::duplex`, read with `read_frame` from the other until the writer ends after the `Outbox` is dropped):
```rust
a_file_is_sent_as_start_chunks_end()          // 2*CHUNK+1 bytes → Start, chunks of CHUNK, CHUNK, 1, End; concatenation == file
a_file_of_exactly_one_chunk_sends_one_chunk() // CHUNK bytes → Start, 1 chunk, End
an_empty_file_sends_start_and_end()           // Start { len: 0 }, End
a_changed_file_is_aborted()                   // hash of other bytes → Start, chunks, Control(Aborted(hash)), no End
a_vanished_file_is_answered_missing()         // nonexistent path → Control(Missing(hash)), no Start
urgent_control_overtakes_a_blob()             // duplex buffer CHUNK; Blob(8*CHUNK); after reading BlobStart send Want(h); Want precedes BlobEnd
bulk_keeps_its_order()                        // Blob(small), Updates(vec![]), Done → exactly that order
the_writer_drains_both_queues_before_exiting()
```
`incoming.rs`:
```rust
a_blob_is_staged_on_end()                     // start, chunk, end → stager.staged(&h).is_some()
a_second_start_while_open_is_a_protocol_error()
a_chunk_with_nothing_open_is_a_protocol_error()
an_end_with_nothing_open_is_a_protocol_error()
an_abort_discards_the_partial()
an_abort_for_another_hash_is_a_protocol_error()
a_mismatched_end_is_a_protocol_error_and_leaves_nothing()
dropping_mid_blob_leaves_no_partial()
```
`tests/blob.rs` — real two-replica sessions use the fixture pattern of `tests/converge.rs`; "fake peer" means raw frames on the other end of a `duplex`, written with the exported frame functions:
```rust
a_file_of_exactly_64_mib_syncs()
a_file_above_64_mib_syncs()                         // 64 MiB + 1; set max_file_size to 128 MiB on both (Task 3 raises the default)
an_empty_file_syncs()
a_blob_over_the_receivers_limit_is_refused()        // receiver max_file_size 1024; fake peer BLOB_START len 1025 → Err(Protocol)
content_that_does_not_match_its_hash_is_refused()   // START(hash of b"abc", 3), chunk b"abd", END → Err(Protocol)
a_stream_cut_mid_blob_commits_nothing_and_leaves_no_partial()
    // fake peer: Hello, START len 1000, 10 bytes, drop → Ok (Gone); vv unchanged; no *.partial in staging_dir
a_v1_hello_is_refused()                             // Hello { format: 1 } → Err(VersionMismatch { ours: 2, theirs: 1 })
an_aborted_blob_leaves_the_path_for_the_next_session()
    // replica b vs fake peer a: a sends Updates naming big.bin with hash H (> INLINE_LIMIT), Done;
    // on b's Want(H) a sends START(H), one chunk, Aborted(H), then Settled once b's Settled is in
    // → b's session Ok; big.bin absent on b. Then a real session from a replica holding big.bin delivers it.
```
`tests/live.rs`: `a_live_push_of_a_file_over_the_inline_limit_arrives_chunked()` — the existing live content test with a `3 * CHUNK + 7`-byte file.

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-session` — compile failures / failing tests.
- [ ] **Step 3: Implement** the interfaces above.
- [ ] **Step 4: Run** `cargo test -p sapphire-framework-session` (add `--release` if the 64 MiB tests are slow in debug) and `cargo test -p sapphire-framework-bridge pairing` — all pass.
- [ ] **Step 5: Commit** `feat(session)!: chunked content frames, two-queue writer, receive into staging`

---

### Task 3: Path-based content in the replication core (sync)

**Files:**
- Modify: `crates/sapphire-framework-sync/src/replica.rs`, `src/testing.rs`, `Cargo.toml`
- Modify tests: `tests/replica_basic.rs` (and any of `replica_conflict.rs`, `replica_guards.rs`, `convergence.rs` that stop compiling)
- Modify: `crates/sapphire-framework-session/src/session.rs` (`NoContent`)

**Interfaces:**
- Consumes: `Stager`, `StagingWriter`, `content_path` (Task 1).
- Produces:
  - `pub trait ContentSource { fn locate(&self, hash: &ContentHash) -> Option<PathBuf>; }`; `impl ContentSource for Replica` via `content_path`; session's `NoContent::locate` returns `None`.
  - `read_content` removed; `content_with` returns a path.
  - `stage()`: `stager.staged(hash)` returns at once (no re-hash). Otherwise `content_path` or `source.locate` → `std::io::copy` into `stager.writer(hash, len)` → `finish()`. `ContentMismatch`/`TooLarge` are logged and treated as unavailable (`Ok(None)`), like today's wrong-hash case.
  - `ensure_conflict_copies`: the loser's bytes come from `rel` on disk (when `state.disk.hash == Some(hash)`; a read error still propagates), else `content_path`, else `source.locate`; copied through a `StagingWriter` and `place`d from the finished path. The `.copy` staging name goes.
  - `DEFAULT_MAX_FILE_SIZE = 1024 * 1024 * 1024`.
  - `testing::MapSource` holds a `tempfile::TempDir` and writes each `with(bytes)` entry to `<dir>/<hex>`; `locate` returns it. `tempfile` becomes an optional dependency enabled by `test-util`. Call sites (`MapSource::default().with(..)`, `fetch_missing(&a.replica)`) keep compiling unchanged.

- [ ] **Step 1: Write failing tests** (`tests/replica_basic.rs`)

```rust
a_staged_file_is_used_without_rehashing()     // finish a staged blob; apply an update naming it with NoContent-like empty MapSource → materialised; staging/<hex> gone
a_file_above_64_mib_is_recorded_by_default()  // default config; 64 MiB + 1 file; scan records it, no SkipReason for it
a_conflict_copy_of_a_large_loser_is_written() // loser > INLINE_LIMIT only available via source.locate → copy path holds its bytes
```

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-sync` — fail.
- [ ] **Step 3: Implement.** Then `rg 'fs::read\(|fs::write\(' crates/sapphire-framework-sync/src` must show no content reads or writes.
- [ ] **Step 4: Run** `cargo test -p sapphire-framework-sync -p sapphire-framework-session` — all pass, including `golden.rs` and `convergence.rs`.
- [ ] **Step 5: Commit** `feat(sync)!: locate content by path; 1 GiB default limit`

---

### Task 4: Per-step locking and the idle timeout (session, bridge)

**Files:**
- Modify: `crates/sapphire-framework-session/src/session.rs`, `src/progress.rs`, `src/lib.rs`
- Modify: `crates/sapphire-framework-bridge/src/wgsync.rs`
- Test: `crates/sapphire-framework-session/tests/lock_scope.rs`, `tests/idle.rs` (new)

**Interfaces:**
- Consumes: `Progress` (Task 2), `Incoming` (Task 2).
- Produces:
  - `pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);` exported from `lib.rs`, its doc carrying spec §5's "both directions count" reasoning.
  - `Progress::idle_for(&self) -> Duration`.
  - `pub(crate) async fn read_or_idle<R: AsyncRead + Unpin>(r: &mut R, progress: &Progress) -> Result<Option<Frame>>` — touches on each frame; fails with `Protocol(format!("no progress for {IDLE_TIMEOUT:?}"))` when idle. `read_frame` is not cancel-safe, so one pinned future survives deadline re-arms:

```rust
let read = read_frame(r);
tokio::pin!(read);
loop {
    let deadline = Instant::now() + IDLE_TIMEOUT.saturating_sub(progress.idle_for());
    tokio::select! {
        frame = &mut read => { progress.touch(); return frame; }
        _ = tokio::time::sleep_until(deadline) => {
            if progress.idle_for() >= IDLE_TIMEOUT { return Err(/* as above */); }
        }
    }
}
```
  - `initial_exchange`'s read loop uses `read_or_idle`; the `Hello` read and the live loop do not.
  - `initial_exchange` stops holding `replica.lock()` across its loop: it locks to take `stager()` and build the delta (with `content_path` for inline blobs), then per `Updates` page (`apply` + `needed`) and per `Want` (`content_path`). Update the module docs and the `initial_exchange`/`open_live_session` doc comments that describe the lock as continuous.
  - `wgsync.rs`: delete `SESSION_TIMEOUT` and the `tokio::time::timeout` around `replica.session(stream)`, keeping the `Ok`/`Err` handling.

- [ ] **Step 1: Write failing tests**

`tests/lock_scope.rs`:
```rust
a_replica_is_not_locked_while_a_blob_is_being_received()
    // fake peer: Hello, BLOB_START(len 4*CHUNK), one chunk, then waits on a oneshot.
    // timeout(2s, shared.lock()) is Ok and scan() is Ok. Then the rest, END, Done, Settled → run_session Ok.
a_scan_between_pages_still_converges()
    // two real replicas; a has 300 files (> PAGE) and one 3*CHUNK file; while the session runs,
    // the test loops b.lock().scan(); then a second session; both roots hold identical files.
```
`tests/idle.rs` (`#[tokio::test(start_paused = true)]`):
```rust
an_exchange_with_no_progress_is_given_up_after_the_idle_timeout()
    // fake peer: Hello, then silent → Err(Protocol) containing "no progress"
a_slow_transfer_that_keeps_progressing_is_not_cut_off()
    // fake peer: Hello, BLOB_START(len 5*CHUNK), one chunk every 30 s, END, Done, Settled → Ok
a_sender_waiting_on_its_own_upload_is_not_cut_off()
    // replica under test holds a 6*CHUNK file; fake peer's Hello has an empty vv, it Wants the file,
    // reads one frame every 30 s and sends nothing until it has END, then Done + Settled → Ok
```

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-session --test lock_scope --test idle` — fail.
- [ ] **Step 3: Implement** the interfaces above.
- [ ] **Step 4: Run** `cargo test -p sapphire-framework-session -p sapphire-framework-bridge -p sapphire-framework-server` — all pass; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] **Step 5: Commit** `feat(session): lock per step and give up on no progress, not total time`

---

### Task 5: Workspace check and p2p spec note

**Files:**
- Modify: `docs/superpowers/specs/2026-09-15-p2p-sync-iroh-design.md` — a note at decision 12 and §3.4: content travels chunked on `sapphire/ws/1` (see the 2026-10-19 spec); `sapphire/blob/1` will not be added.

- [ ] **Step 1:** Add the note.
- [ ] **Step 2: Run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` — all pass.
- [ ] **Step 3: Commit** `docs: point the p2p spec at chunked content transfer`
