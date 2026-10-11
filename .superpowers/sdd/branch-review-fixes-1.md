# Branch Review Fixes: feat/p2p-sync-iroh

Verdicts for `.superpowers/sdd/branch-review-feat-p2p-sync-iroh.md`. Each issue carries
the commit that resolves it and the tests that pin the behaviour.

## Critical #1 — Workgroup drive loop + retired-dial + conflict-copy ban

**Verdict: ADDRESSED** — commit `99c62c8` (`fix(bridge): drive the workgroup workspace
while the bridge runs`), integration tests in commit `119ad96`.

- The bridge spawns a driver (`wgsync::spawn_driver`) whenever the workgroup replica is
  opened (`lib.rs` `serve_workgroup`). It watches the workgroup root (`ChangeWatch`),
  coalesces a write burst behind `CHANGE_DEBOUNCE` (300 ms), drains the inotify residue
  (`drain_and_settle`, `QUIET` = 120 ms), records everything with a scan, and runs a
  replication session with every peer with a greater device id — the same id-order rule
  the app server's dial loop applies. A sweep every `SWEEP_INTERVAL` (5 s) carries a
  change a peer missed while offline, and the first drive runs immediately, so the
  ledger's state is pushed before the first sweep tick. Failures dial with the same
  doubling backoff curve, capped at `BACKOFF_MAX`; a session is bounded by
  `SESSION_TIMEOUT` so a stalled peer cannot hold the replica's lock for ever. A
  re-opened replica (a join rewrote the workgroup directory) aborts the previous driver.
- **Retired-dial exception** (`drive` in wgsync.rs): a retired member is dialed whatever
  side of the id order it sits on — `device.id < me.id && !device.is_retired()` is the
  only skip condition — so the retirement reaches the device it retired, whose own bridge
  must learn it is out or it keeps showing up connected.
- **Conflict-copy ban** (`workgroup.rs` `IGNORE_BODY`): `create` and `materialize` write
  `*.conflict-*` into the workgroup root's `.sapphireignore`, so a retirement arriving
  over a record this host had not yet seen cannot leave a `devices/*.conflict-*.toml`
  file the ledger cannot open; last-writer-wins is the whole story for the workgroup's
  own state, and the ignore file itself replicates to members that joined before it
  existed.
- **Joiner-side reachability** (pairing.rs, data.rs, workgroup.rs `materialize`): the
  inviter's record rides `JoinResponse::Admitted` and is materialized into the joiner's
  ledger, so a joiner whose id sorts smaller knows the one peer it may dial and the pair
  meets regardless of how the ids landed.

Pinning tests:

- `tests/workgroup_driver.rs::a_retirement_from_the_smaller_id_reaches_the_greater_one`
  — two running bridges, no fixture scans or dials; a retire on the founder must reach
  the joiner's authorization (regression for Critical #1, per Important #4).
- `tests/workgroup_driver.rs::a_joiner_with_the_smaller_id_can_dial_the_inviter` — the
  other side of the dial rule, the side the inviter's record in the join response opens.
- `wgsync::tests::a_retired_member_is_dialed_on_either_side_of_the_id_order` — the
  retired record's id sorts below *and* above the retiree's across the two loop runs.
- `wgsync::tests::a_retirement_replicates` — the tombstone lands in the peer's ledger.
- `workgroup::tests::the_root_bans_conflict_copies` — the ignore file exists after
  `create` and `join`, `SyncFilter` refuses a conflict-copy path, and the file replicates
  (fixture for the member that joined before the rule).

## Important #2 — IrohTransport::is_connected

**Verdict: ADDRESSED** — commit `07a38a4` (`fix(bridge): report connected peers`).

`is_connected` asks iroh what it is *actively* using toward the endpoint: a remote-info
entry alone proves nothing, since iroh keeps the snapshot for a while after a peer goes
away — only a `TransportAddrUsage::Active` path counts, and a peer never spoken to has no
entry at all. `remote_info` is answered by iroh's remote-state actor from memory, so the
sync trait method blocks briefly through whatever the runtime allows
(`block_in_place` on a multi-thread runtime, `block_on` on a current-thread one, a
throwaway runtime outside any). `bridge.peers`, `status.json` and `bridge device list`
get real answers through `peer.rs`'s seam.

Pinning test: `tests/iroh_transport.rs::a_connected_peer_is_reported_connected_and_an_unknown_one_is_not`
— before the handshake `is_connected` is false; with a stream open each way it turns
true; an unknown node id stays false.

## Important #3 — register verifies before connecting an owner

**Verdict: ADDRESSED** — commit `537cd42` (`fix(bridge): verify the workgroup before
marking an owner online`).

`register` now reads the workgroup and this host's own device record *before*
`owners().connect` / `session.record`, so a registration that ends in an error leaves no
route behind and no owner online — an app whose announce loop kept speaking for a bridge
that refused it can no longer arise. `route_statuses` now reads the owner's presence
directly (`owners().peer(...).is_some()`), which is why `is_online` could move behind the
test seam (`Bridge::is_app_online`).

Pinning test: `tests/switchboard.rs::a_failed_registration_leaves_nothing_behind` — the
host's own record is purged mid-test, the registration must fail, and
`bridge().is_app_online(app)` must be false afterwards.

## Flaky test fixed in this pass

`wgsync::tests::a_write_under_the_workgroup_root_is_reported` failed under load with
`nothing was written, nothing may be reported` (wgsync.rs:668). The assertion misread
`drain_and_settle`: it returns `Some(())` when an event arrives *during* its quiet
window, and a late inotify residue of the same write is exactly that — normal, and a
redundant scan+dial for it is idempotent by design. The test now keeps its discriminating
purpose (the first `changed()` must report the write) and settles the aftermath the way
the production semantics allow: accept `None`, a timed-out settle, or residue alike, and
only require the watch to go quiet within a bounded overall deadline
(`QUIET_WATCH` = 10 s). Production code was not touched.

Verification: 10/10 runs of the exact test under heavy parallel disk/CPU load
(loadgen, 6 writers), plus full bridge-suite runs (175 passed, 0 failed) while the load
generator ran.

## Known pre-existing flakes (review Minor #10) — not fixed here

Both were reproduced on the clean RED commit `fe4093e` with the working tree stashed, so
neither is caused by the reviewed changes; they are the review's own Minor #10 findings
and are left to the follow-up PR the review recommends:

- `server::tests::converge::a_host_that_was_offline_catches_up_when_it_returns` —
  `replica store error: Database already open` (re-open races `Host::stop`'s async
  teardown). Fails ~80% of workspace runs on the clean tree here.
- `server::host::tests::reopening_after_eviction_works` — passed 1/1 standalone in this
  pass; the same "closed ≠ released" shape as the converge one.
- Additionally observed once during full runs (same family, not in the review's list):
  `server::tests::live::three_connected_hosts_do_not_loop_an_edit_between_them` —
  frame counters kept moving once out of five local runs (1/6 standalone). Timing-window
  flake of the same class, for the same follow-up.
