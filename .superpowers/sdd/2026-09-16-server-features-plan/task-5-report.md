# Task 5 report: `status.json`

## Status

DONE. Commit `86d3a26` — `feat(bridge): publish status.json atomically` (5 files, +871/−59).

## What I implemented

**`crates/sapphire-framework-bridge/src/status.rs`** (new, public API re-exported from `lib.rs`):

- `StatusFile { version, pid, started_at, node_id, workgroup: Option<WorkgroupStatus>,
  peers: Vec<PeerStatus>, routes: Vec<RouteStatus>, relays: Vec<String> }` — serde, JSON via
  `to_string_pretty`.
- `PeerStatus { device_id, name, node_id, connected, last_seen: Option<DateTime<Utc>>,
  last_error: Option<String> }` — exactly the brief's shape. `last_seen`/`last_error` are
  always `None` today (the bridge does not track either yet); the fields are there for when
  it does, as the plan specifies them.
- `StatusSource` trait (`snapshot() -> StatusFile`) and
  `StatusWriter::start(path: PathBuf, source: Arc<dyn StatusSource>) -> StatusWriter` with
  `stop(self)`; `STATUS_INTERVAL: Duration = 5s`.
- `StatusFile::load(&Path) -> Result<Option<StatusFile>>` — the reader side the CLI
  fallback needs. Missing file = `None` (the bridge never ran here — normal); present but
  unparsable = error (guessing at a broken snapshot is worse than reporting it), matching
  the conventions of `Workgroup::published_net`.

**Write cadence** (plan global constraint: "every 5 seconds and on change", atomically):
the writer snapshots the source every 1 s (`POLL_INTERVAL`), writing when content changed
**or** `STATUS_INTERVAL` elapsed since the last write. First write is immediate, so the
file exists as soon as the bridge is up. Poll-then-diff keeps "on change" from meaning
"rewrite the file every second even when nothing changed". Failure to write is logged and
retried at the next poll — a stale file for a moment beats an error that kills the loop.

**Atomicity**: reuses the crate's existing `routes::write_atomic(path, header, body)`
(temp file + `sync_all` + rename; rename failure removes the temp file). JSON gets an empty
header. Two brief tests pin the behavior directly: a 2000-peer file rewritten continuously
is read in a loop for 5 s and every read parses (123 reads); after stop, no file except
`status.json` remains in the directory.

**Stop semantics**: `StatusWriter` holds an mpsc sender; the task ends when the handle is
dropped (or `stop(self)` is called). The task only exits between writes, so a stop never
interrupts one — no half-written temp file, and the last snapshot survives (pinned by
tests).

**Production wiring** (`lib.rs`, `serve_loops`): `StatusWriter::start(dir.status_json(),
BridgeSource::new(bridge, net, started_at))` right before `tokio::select!` over the three
serving loops. `BridgeSource` (pub(crate)) reads the live tables; its handle is dropped
when the loops end, which stops the writer and leaves the last snapshot on disk. It is
built in `serve_loops` rather than `run`/`run_shared` because that is where the resolved
`NetConfig` is still in scope (the bridge itself doesn't keep it — it hands it to the
loops), so relay URLs come from exactly the configuration the endpoint was built with.

**No double source of truth**: `control.rs`'s `peers()`/`status()` handlers were factored
into `pub(crate)` helpers — `peer_infos(bridge, &workgroup)`, `workgroup_status(workgroup)`,
`route_statuses(bridge)` — and both the RPC handlers and `BridgeSource::snapshot` now call
the same three. One fact, one place. Handler behavior is unchanged (the extracted code is
byte-identical).

**`bridge status` CLI** (`command.rs`): live control-plane call preferred (`client.status()`
+ `client.peers()`); when nothing answers (`connect` → `None`, never starts a bridge), it
falls back to the last `status.json` snapshot and prints **the same format**, prefixed with
`— last seen before the bridge stopped`, exit 0. No bridge and no snapshot keeps the
existing `no bridge is running` + exit 1 pattern (step 8's convention). An unreadable
snapshot is an error, not a guess.

## Tests

- **Brief's 4 tests verbatim** in `status.rs`'s `#[cfg(test)] mod tests`, plus:
  - `a_change_is_written_without_waiting_for_the_interval` — pins "and on change" (the plan
    global constraint); a `Flipping` source's change must appear well inside the interval.
  - `a_missing_file_is_no_snapshot_and_not_an_error`, `an_unreadable_snapshot_is_an_error_not_a_guess`
    — `StatusFile::load`'s contract.
- **`tests/status_file.rs`** (new integration test): production wiring through
  `Bridge::run_shared` via the `common` fixture — a running bridge writes `status.json`
  with the real node id/pid/workgroup/routes and repeated reads all parse;
  a registration appears in the file; `a_registration_reaches_the_status_file_faster_than_the_tick`
  bounds the appearance inside `STATUS_INTERVAL` so it can only have been written by the
  change path; direct `StatusWriter` use from outside the crate.
- **CLI fallback** (`command.rs::status_fallback_tests`): with a stopped bridge's snapshot
  on disk and nothing listening → exit 0; without a snapshot → exit 1.

### TDD evidence

**RED** — `cargo test -p sapphire-framework-bridge --all-features status` after writing
only the brief's tests: `could not compile sapphire-framework-bridge (lib test) due to 16
previous errors` — `E0405 cannot find trait StatusSource`, `E0422 cannot find struct
PeerStatus/StatusFile`, `E0433 cannot find type StatusWriter/Arc`, `E0425 StatusFile` —
i.e. exactly the not-yet-existing interface, as expected.

**GREEN** — same command after implementation: `9 passed; 0 failed` (5 brief/tests + 2
load + change + no-bridge-exits), and `--test status_file`: `4 passed`.

**Full suite**: `cargo test --workspace --all-features --locked` run twice —
**671 passed, 0 failed** both times (61 test binaries). The known pre-existing flake
`server::converge::a_host_that_was_offline_catches_up_when_it_returns` did not fire in
either run; not chased.

**Lint**: `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets
--all-features -- -D warnings` clean (also verified for `-p sapphire-framework-bridge`
before the workspace run).

## Files changed

- Created: `crates/sapphire-framework-bridge/src/status.rs`,
  `crates/sapphire-framework-bridge/tests/status_file.rs`
- Modified: `crates/sapphire-framework-bridge/src/{lib.rs,command.rs,control.rs}`

## Design decisions worth flagging

1. **Poll-then-compare (1 s) instead of an event bus.** "Rewritten on change" is satisfied
   by re-snapshotting cheaply and writing when the serialized body differs. An event bus
   into the routing table/ledger would have threaded hooks through four modules for a
   visibility file; the poll keeps `StatusSource` the only seam the plan defines. Cost: a
   change is visible within 1 s, not instantly — well inside the 5 s interval the spec
   mandates for everything anyway. Pinned by `a_registration_reaches_the_status_file_faster_than_the_tick`.
2. **`BridgeSource` holds the resolved `NetConfig`** rather than re-reading `net.toml` per
   snapshot: `serve_loops` is the one place that still holds the resolved config the
   endpoint was built from, and a snapshot must agree with the endpoint actually running.
3. **Degrade-per-block, not fail-whole-file.** If the ledger can't be read at snapshot
   time, the file loses its device list, not its existence — the status file exists to
   answer "what is the bridge doing", and half an answer still answers more than no file.
   Each block logs a `warn`.
4. **Test-env serialization in `command.rs`.** The two new fallback tests needed
   `SAPPHIRE_RUNTIME_DIR`/`SAPPHIRE_BRIDGE_DIR`, which are process-global and raced with
   the two pre-existing env-setting tests under `cargo test`'s parallel runner (observed
   once as a real failure in the full workspace run). Added `test_env::with_dirs` — a
   tokio-mutex-guarded helper that sets both vars and hands the body an isolated
   `bridge-<n>` directory — and converted all four env-touching CLI tests (mine two, plus
   the pre-existing `status_against_no_bridge_exits_non_zero` and
   `joining_without_a_running_bridge_says_so_rather_than_starting_one`) onto it. That
   closes a pre-existing latent race rather than adding one; the two old tests' assertions
   are unchanged.
5. **`PeerStatus.last_seen/last_error` stay `None`.** No bridge code tracks per-peer
   sessions yet; inventing a value would be worse than an honest `null`. Fields are there
   per the interface spec, documented as reserved.

## Self-review findings

- All 4 brief tests present as written (atomism / last-snapshot-survives / no-temp-files /
  parses-promptly) — verbatim, not adapted.
- Atomic write is `routes::write_atomic` (temp + rename), not a new implementation.
- CLI: live preferred, file fallback, exit-1+message pattern unchanged for the nothing-at-all case.
- First commit attempt accidentally swept in the controller's pre-staged `progress.md`;
  reset and recommitted so `86d3a26` contains only Task 5 files. `progress.md` is back
  staged (A) in the index, as I found it.

## Concerns

- `write_atomic` names its temp file by pid only; two writers in one process to the same
  path would collide. Safe today under the single-instance lock (and `publish_workspace` /
  `publish_net` share the property); worth a per-call uniquifier if ever reused outside
  the bridge. Not changed here — out of scope.
- The status writer is wired in `serve_loops`, so a bridge built for tests
  (`Bridge::new` without `run`) writes nothing — by design; the brief's wiring is the
  running process's. `test-util` consumers get the writer through the public API (covered
  by `the_writer_writes_to_the_bridge_directory_it_is_handed`).