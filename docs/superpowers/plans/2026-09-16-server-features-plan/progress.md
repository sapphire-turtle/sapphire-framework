# Task 2 — live propagation in the app server (2026-09-16)

Status: implementation complete, verification green, commit pending terminal
availability (session shell slots saturated by earlier test loops at report
time; commit + final parallel-suite re-run are the only outstanding steps).

Root cause of the flaky settle (30 s timeout at tests/common/mod.rs:500):
`open_live_session` holds the replica mutex across the whole exchange; when
two hosts dial each other simultaneously (dial loop + watcher-triggered
sync_now racing), each dialler waits for the peer's hello while holding its
own lock — circular wait. Fixed by dialing in one direction only: a host
dials only peers with a greater device id (rule applied in both `dial_loop`
and `sync_now`). Waits-for graph is now a DAG (low-id → high-id); the
tie-break (`LivePeers::insert`, keep the stream initiated by the lower
device id) remains as the safety net. `sync_now` also switched from one-shot
`run_session` (closed the stream, killing inbound sessions) to
`sync_one` → `adopt_live_session`.

Verification at last full run: live 6/6 (8 consecutive runs incl. full
parallelism, ~5 s each), bridge 101 pass, fmt --check clean, clippy
--workspace --all-targets --all-features -D warnings clean. Full workspace
suite green in repeated runs; two pre-existing fixture races surfaced only
while several cargo suites ran concurrently (`converge` redb "Database
already open" restart race; `host::tests::reopening_after_eviction_works`
tantivy lock) — not reproducible on a quiet machine, out of scope, noted in
task-2-report.md.

Report: .superpowers/sdd/2026-09-16-server-features-plan/task-2-report.mdUpdate (final): committed as 852dc21 on feat/p2p-sync-iroh (7 files,
+1162/−60). Full workspace suite --locked: 59/59 result lines ok, zero
failures. live 6/6 in 5.0 s; fmt --check clean; clippy -D warnings clean.
Report updated with the commit and final evidence.