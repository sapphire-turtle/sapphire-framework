# Task 5 Report: Rewrite `ARCHITECTURE.md`

Plan: `docs/superpowers/plans/2026-09-16-cleanup-plan.md` §Task 5. Brief file `task-5-brief.md`
does not exist; the plan's §Task 5 text plus the two specs
(`2026-09-16-process-architecture-design.md`, `2026-09-15-p2p-sync-iroh-design.md`) served as
the brief.

> **Provenance note.** A previous session wrote a `task-5-report.md` claiming commit
> `ccdd090`; that commit does not exist anywhere (not in any ref, not in `git fsck
> --unreachable`, not in any stash/WIP object). The claimed work was never committed —
> this report supersedes that file and describes the work actually done and committed
> here. Three scratch notes (`.superpowers/tmp-{retained,sync,dir}.md`) held staged
> section drafts; they were merged into the final document and deleted.

Commit: `3b05eb5` on `feat/p2p-sync-iroh` (parent = `0563f4d`, Task 4).

## What changed (Steps 1–3)

`docs/ARCHITECTURE.md` — Japanese kept, section order kept where it still makes sense:

1. **`## remote 同期 API（JSON-RPC・実装済み）` → `## ワークスペース同期（iroh・2 仕様）`**
   (Step 1). The JSON-RPC method table, the generation/cursor/LWW narrative and the
   "central server owns the change log" story are gone. The new section describes:
   - the two specs and how to read the 2026-09-15 sync spec (its §3–§5 were revised in
     place, not deleted — read them through the substitution table; decisions 8/13/14
     superseded, decision 11 extended);
   - **one writer per piece of state** (process-architecture spec §1): workspace files,
     retrieve/track/app caches and the replica store belong to the app's **app server**;
     device identity, workgroup membership and pairing belong to the **bridge**;
   - `-ipc` (UDS / named pipe / in-process channel NDJSON JSON-RPC, same-OS-user trust,
     no tokens, CLI spawns the server from `current_exe()`);
   - the bridge as host-resident switchboard (`node.key`, `routes.toml`, workgroup
     ledger, `status.json`, `logs/node.log`; control plane + data plane splice;
     `wake_on_sync`; the workgroup workspace makes the bridge an app server of one app);
   - sessions end to end between two app servers over the spliced stream (`Hello` →
     `PathUpdate` pages → `Done`/`Settled`, ≤64 KiB inline, SHA-256 content addressing,
     DVV-set join with conflict copies, external-edit detection, missing-root pause,
     64 MiB cap, file content only — no mtimes/permissions/vector indexes);
   - `sync-id` vs path-derived uuid; privilege separation (Unix-only, §3, #257).
   Both specs are cited by path.
2. **`## アプリディレクトリ構成と CLI 規約（#128 / #129）`** (Step 2) — rewritten to the
   unsplit layout (`<platform root>/<app>/`, no `<kind>` path segment; `AppKind` keeps
   existing as a process-type description but never in a path), env overrides replace
   the platform root only, bridge dir `<data root>/sapphire/bridge/` and runtime dir
   `<data root>/sapphire/run/` sit outside the per-app layout, two migrations
   (keys.toml to data tree, then per-kind unsplit), and `--workspace-dir` with the
   documented alias/resolve chain. **Kept a full paragraph** explaining that the split
   existed (#129) and why it went (process architecture removed the collision itself:
   only the server opens a DB; the split never covered the CLI-vs-stdio-MCP case),
   including what to do when someone finds `<app>/server/` on disk (the second
   migration moves it back, server copy wins on conflict, empty kind dirs may be
   deleted).
3. **`## crate 構成`** (Step 3) — table now matches the workspace exactly:
   15 crates + the `apps/sapphire-bridge` binary, `sapphire-framework-gui` added
   (existed on disk, was missing from the old table), removed crates
   (`-rpc`, `-remote-client`, `-remote-server`, `-blob`) kept as struck-through rows
   with their replacement rationale. Notes added: `-sync`/`-session` do not depend on
   `-workspace`/`-retrieve`; `-server` must not pull in iroh (spec §6). The
   ⬜/✅ status column is gone — the table describes what exists.
4. **`## 実装フェーズ` → `## 実装の現在地`** (Step 3) — the Phase 0–4 list is replaced by
   a pointer to the process-architecture spec's §9 (which itself replaced sync-spec
   §5.6), with what is done (sync core through service install), what is in progress
   (the cleanup: keys extraction, four crate removals, directory unsplit, facade
   features, this document), what remains (per-app migrations, `sapphire-sync` as one
   app, crates.io publishing), and **WASM explicitly marked out of scope, citing the
   sync spec's decision 7** (plus #86 steps D–F).
5. **`## GUI 向け 非同期 Backend trait`** — kept, corrected: `IpcBackend`/`LocalBackend`
   are the implementations; the `RemoteBackend` paragraph is replaced by the IPC story;
   the journal-GUI refactor stays a separate-repo follow-up.
6. **The principles list (`## 確定した方針`)** — the three remote/WASM claims that are
   now false were corrected: the JSON-RPC bullet now describes iroh p2p with API keys
   surviving only for non-sync HTTP endpoints; the `RemoteBackend`/`RemoteClient`
   bullet is replaced by the Model-B symmetry note it had grown from; the central-sync
   bullet now reads as history ("#90 の方針は p2p 同期で置き換えた" — the #90 removal
   of built-in auto-sync itself is still true and kept). The intro's two superseded
   goals gained inline pointers to the sections that superseded them (central-server
   → p2p; WASM → decision 7) rather than a rewrite, so the document still explains
   why the framework looks the way it does.
7. **`## 既知のリスク / 難所`** — WASM items (3, 4) replaced by the risks that are real
   now (platform differences, spec §12's accepted costs: slower first CLI invocation,
   second directory migration, NFS/UDS, more processes; Postgres/S3 swap-in kept).

## Step 4: the grep

Run exactly as the plan writes it (plus nothing — the exclusions are the plan's three;
`.superpowers/` is gitignored so its hits do not reach a commit):

Residual hits and judgements:

- `crates/sapphire-framework-retrieve/src/db.rs:218` — doc comment naming the removed
  `remote-server`'s `WsStore` — **fixed in this commit** to point at the app server.
- `docs/HANDOFF.md:49-52,55` — Phase 3 table naming the four removed crates — the file
  is a dated handoff note (最終更新 2026-07-15) that the plan's "does not cover" table
  does not list; rather than rewrite history, **added a 2-line annotation** after the
  table (in this commit) stating the four crates were removed in 2026-09 and pointing
  to ARCHITECTURE.md and the process-architecture spec for the current state. The
  historical table itself is untouched.
- `crates/sapphire-framework/tests/surface.rs` + `tests/features.rs` — string literals
  in the very tests Task 2 prescribed; they assert these names are *absent*. Left as
  is (intentional).
- `crates/sapphire-framework/CHANGELOG.md` — excluded by the plan. Left.
- dated specs under `docs/superpowers/specs/2026-08-*` and the process-architecture
  spec's own removed-crate table — historical records (the brief's exclusion list
  read as "dated specs and plans"). Left.

## Verification

- `cargo fmt --all -- --check` → clean.
- `cargo clippy --all-targets --all-features -- -D warnings` → clean (Finished, no warnings).
- `cargo test --all-features --locked --no-fail-fast` → **688 passed, 0 failed** across
  58 test binaries (summary `passed:688 failed:0`; no warnings in output). This run
  includes the previously-flaky convergence tests
  (`a_host_that_was_offline_catches_up_when_it_returns`,
  `two_simultaneous_cli_writers_both_succeed`) — both green in this run.
- Note: the lock-on-restart panic reported in earlier investigation did not reproduce
  in any of the three full-suite runs made today; recorded as an observation, not a
  failure (no code was changed for it).

## Files changed

- `docs/ARCHITECTURE.md` (the rewrite)
- `docs/HANDOFF.md` (annotation only, from Step 4)
- `crates/sapphire-framework-retrieve/src/db.rs` (one doc-comment line, from Step 4)
- `.superpowers/sdd/2026-09-16-cleanup-plan/progress.md` (ledger, gitignored)
- this file (gitignored)

## Self-review

- Every claim in the rewritten sections is grounded in the two specs or in code read
  this session (crate list = workspace `Cargo.toml`; bridge/status/log/pairing; ipc
  transports/trust; session framing/inline limit; sync merge/HLC/conflict-copy/cap;
  service `run_as`/`helper_as`; unsplit migration; resolve chain).
- The document keeps the old shape (same section order where meaningful), so a reader
  of the old version can navigate it; all superseded claims now say so explicitly.
- No English crept in; the doc remains Japanese per CONTRIBUTING.
- Concern carried forward (above this task's pay grade): the process-architecture spec
  §4.2 says `LocalBackend` becomes internal to `-server`, but on disk it still lives in
  `-backend`; ARCHITECTURE.md deliberately names only what is true on disk.
