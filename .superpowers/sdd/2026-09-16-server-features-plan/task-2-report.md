# Task 2 Report — Live propagation in the app server

**Plan**: `docs/superpowers/plans/2026-09-16-server-features-plan.md`, Task 2
**Branch**: `feat/p2p-sync-iroh` **Status**: DONE

## What was implemented

Live propagation: a commit made through one host's IPC reaches every other
host over sessions that are already open (spec §4.2). Four pieces:

1. **`sync/sync/live.rs` (new)** — `LivePeers`, the open sessions of one
   workspace keyed by device id:
   - `has`, `insert` (tie-break: when both ends dial at once, the stream
     initiated by the **lower device id** is kept, the other closed; returns
     the adopted session's update channel or `None` when this stream lost),
   - `fan_out(from: Option<&GrainId>, updates: Vec<PathUpdate>)` — pushes to
     every open session except the source of the entries,
   - `drop(peer)` on session end, `begin_dial`/`end_dial` per-peer dial guard
     (only one outbound exchange per pair at a time),
   - `dial_backoff` exponential, capped at `DIAL_BACKOFF_MAX` (5 min).
2. **`SyncRuntime::dial_loop`** — walks `bridge.peers()` every
   `DIAL_INTERVAL` (5 s) and opens a live session to each peer that has none,
   adopting the stream via `adopt_live_session`. Aborted with the server.
3. **`sync_now` goes live** — the watcher's/enable's trigger used to run a
   one-shot `run_session`, which closed the stream after the exchange while
   the accepting peer kept pushing into it; every inbound session died
   instantly and settle never converged. It now dials via `sync_one` →
   `adopt_live_session`, so both ends agree the session stays open.
4. **Propagation hooks** — the per-session reader applies batches and calls
   `after_commit(from: Some(peer))` → `fan_out`, which gives A → S → B
   forwarding; local commits fan out via `scan()`'s
   `after_commit(from: None)`. Storm rules (never back to the source, never
   what the peer's known version vector covers) are enforced by `fan_out`.

## The root-cause bug, and the deadlock fix

A flaky `settle` timeout (`a_change_arrives_without_waiting_for_the_next_dial`
panicking at `tests/common/mod.rs:500`) under parallel load led to a
timeline reconstruction with temporary instrumentation. The traces showed a
**cross deadlock**, not slow convergence:

- `open_live_session` holds the replica's `tokio::sync::Mutex` across the
  whole Hello→Updates exchange.
- When two hosts dial each other at the same instant, each dialler holds its
  own replica's lock and waits for the peer's hello; each peer's acceptor
  must take that same lock to produce the hello — but the peer's lock is
  held by the peer's dialler, waiting for *this* host's hello. Four tasks,
  two locks, circular wait. Both exchanges hung until test teardown forced
  EOF (`ADOPT-EXCHANGE-FAIL` after ~30 s).

**Fix: dial in one direction only.** A host dials only peers whose device id
is greater (`me < p.device_id`), in both `dial_loop` and `sync_now`. The
waits-for chain then always goes low-id → high-id — a DAG — so the deadlock
is structurally impossible; the tie-break is left as the safety net it is.
The exchange itself is symmetric, so the higher-id host loses nothing: it
converges within one dial interval of the lower host. The spec's "every
device dials every other" intent (every pair ends up with a session) is
preserved; only the *initiator* is now deterministic.

Supporting changes:

- **`bridge/data.rs`**: restored the tracing that temporary instrumentation
  had replaced, and removed the instrumentation itself (`ts2`, eprintlns).
- **`server/lib.rs`**: `dial_loop` spawned alongside the watcher and the
  announcement loop, aborted with the server.
- **`bridge/peer.rs` (test-util)**: `LoopbackNetwork` gained frame counters
  (`frames_sent`) used by the loop tests, plus its own unit tests.
- **`tests/common/mod.rs`**: `NODE_S` (third host), host accessors
  (`node_id`, bridge client, `drop_connections`, `live_session_devices`),
  `synced_triple`, `settle`, `frames_sent`.
- **`tests/live.rs` (new)**: the six live-propagation tests from the plan.
- Two pre-existing clippy warnings in `bridge/peer.rs` fixed (`_b` keeps the
  transport alive for the unreachability assertion; `mut` removed).

## Why the direction rule matters for correctness

Before the fix, `sync_now`'s one-shot dial path also raced the dial loop:
two overlapping outbound dials between the same pair each saw their own
stream as the one to keep, and the tie-break closed **both** in the worst
interleaving (each end kept its own). `begin_dial`/`end_dial` removes that
race within one host; the direction rule removes it across hosts, and makes
the dial direction deterministic so the tie-break is exact in the common
case instead of a fallback.

## Testing

**TDD**: the six tests in `tests/live.rs` were written first (from the
plan's Task 1 continuation) and failed against the then-current code
(one-shot sessions, no dial loop): e.g. `quick.md` never reached the peer
(`settle` panicked at `tests/common/mod.rs:500`) and
`a_change_propagates_through_a_host_in_the_middle` failed because the middle
host had no forwarding hook. RED evidence predates the file-loss incident
(this report documents it; the red runs are in the session transcript of
2026-09-16, not reproducible post-hoc).

GREEN evidence, final state:

- `cargo test -p sapphire-framework-server --all-features --test live`
  → **6/6 pass**, ~5.0 s, **8 consecutive runs green** including full
  parallelism (the flaky test previously failed on nearly every parallel
  run; serial-only passes). Convergence after the fix is one dial interval.
- `cargo test -p sapphire-framework-bridge --all-features` → **101 pass**.
- `cargo test --workspace --all-features` → all suites green across
  repeated parallel runs (see Flakiness note below).
- `cargo fmt --all -- --check` → clean.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` →
  clean.

## Files changed

- `crates/sapphire-framework-server/src/sync/live.rs` (new, ~7.7 KiB)
- `crates/sapphire-framework-server/src/sync/mod.rs` (sync_now live,
  dial_loop, adopt_live_session, sync_one, direction rule, tests)
- `crates/sapphire-framework-server/src/lib.rs` (spawn/abort dial_loop)
- `crates/sapphire-framework-bridge/src/data.rs` (tracing restored)
- `crates/sapphire-framework-bridge/src/peer.rs` (loopback frame counters,
  tests; clippy fixes)
- `crates/sapphire-framework-server/tests/common/mod.rs` (fixtures)
- `crates/sapphire-framework-server/tests/live.rs` (new, 6 tests)

## Self-review findings

- The direction rule changes which host initiates; a host whose id is
  highest in the workgroup never dials. This is intended (DAG), but worth
  re-visiting if the design ever adds push-only peers that must dial out
  through NAT to a lower-id host.
- `LivePeers::fan_out` collects `(device, updates)` under the lock and
  awaits pushes after dropping it, per the plan's note against holding a
  `std::sync::Mutex` across `.await`.

## Issues / concerns

- **Flakiness in tests I did not touch, seen only under full-machine
  parallel load** (several cargo runs running concurrently from timed-out
  shells): `converge::a_host_that_was_offline_catches_up_when_it_returns`
  hit `redb: Database already open` (restart race: the new process enabled
  sync before the old process released the database), and
  `host::tests::reopening_after_eviction_works` hit `tantivy: Failed to
  acquire index lock` (two lib-test binaries sharing `TMPDIR`-scoped env
  under `lock_env` — timing, not state). Both pass consistently when the
  machine is not running three cargo suites at once (repeated runs green).
  They are pre-existing parallel-env races in test fixtures, out of Task 2's
  scope; worth a follow-up if CI parallelism is high.
- The `file_write` tool overwrote the untracked `live.rs` draft mid-task
  (no git history); it was rebuilt from `mod.rs` call sites, tests, and API
  docs. The final file is reviewed line-by-line against its callers. Lesson
  applied: commit drafts early (hence this commit includes live.rs).

## Commits

- `feat(server): propagate commits live, and through a host in the middle`
  (this task's single commit on `feat/p2p-sync-iroh`)## Final commit (verification after the report was drafted)

- Commit: **`852dc21`** `feat(server): propagate commits live, and through a
  host in the middle` — 7 files, +1162/−60, on `feat/p2p-sync-iroh`.
- Final evidence at commit time:
  - `cargo test --workspace --all-features --locked` → **59/59 test-result
    lines ok, zero failures**.
  - `cargo test -p sapphire-framework-server --all-features --test live` →
    6/6 in 5.0 s.
  - `cargo fmt --all -- --check` → clean.
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
    → clean (both `peer.rs` warnings fixed: `_b` keeps the third transport
    alive for the unreachability assertion; `mut` removed).
- The commit message body documents the sync_now live-session change, the
  per-peer dial guard, and the lower-device-id dial direction + tie-break.