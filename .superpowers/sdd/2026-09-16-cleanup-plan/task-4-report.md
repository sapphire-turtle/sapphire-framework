# Task 4 Report: The facade

Plan: `docs/superpowers/plans/2026-09-16-cleanup-plan.md` §Task 4
Brief: `.superpowers/sdd/2026-09-16-cleanup-plan/task-4-brief.md` (did not exist on disk; the plan §Task 4 is its source — treated as the brief, verbatim)
Commit: `9ef7e4f` on `feat/p2p-sync-iroh` (HEAD after Task 3's `aa028dd`, work tree was clean except the gitignored progress ledger)

## What I implemented

1. **`crates/sapphire-framework/tests/features.rs`** — the plan's three tests, verbatim
   (only re-formatted by `cargo fmt`, which expands the line-broken array and wraps
   one assert):

   - `the_old_features_are_gone` — asserts the manifest contains no `remote-client`,
     `remote-server`, `\nrpc =` (feature-table position of the old `rpc` feature) or `\nblob =`.
   - `every_module_feature_has_a_matching_optional_dependency` — for each of the 13 module
     features, asserts a `sapphire-framework-<feature> =` dependency entry exists.
   - `native_includes_what_a_host_needs` — finds the `native =` line and asserts it
     contains `workspace`, `backend`, `server`, `bridge`.

2. **`crates/sapphire-framework/Cargo.toml`** — one-line change: `native` drops `keys`
   (Task 2's interim set had `["workspace","backend","server","bridge","registry","keys","service"]`;
   the plan's target set omits `keys` and the test's assertion list matches that target).
   Everything else in the facade already matched the plan's spec exactly after Task 2; I
   verified the 13-line module-feature table against the plan's list mechanically
   (`diff` of extracted lines: identical, order included).

3. **`.github/workflows/ci.yml`** — added the step "Each facade feature builds on its own"
   at the end of the `test` job (after `Test`, before the `privileged` job), running the
   plan's loop over the 12 features with `cargo check -p sapphire-framework
   --no-default-features --features "$f"`. The plan's snippet does not say `--locked`;
   since the rest of the job is locked (`Test` step), adding `--locked` keeps the whole
   job lockfile-gated for free. Verified with `yaml.safe_load` (PyYAML) that the step
   landed in the `test` job and the other jobs are untouched.

`lib.rs` needed no changes: every module feature already re-exports
`sapphire_framework::<name>` (the `backend` module correctly rides the backend crate's
custom lib name `sapphire_backend`). The manifest's dep table already listed all 13
optional deps.

## TDD evidence

All three tests passed on first run (Task 2 pre-cleaned the manifest), so RED had to be
produced by mutation — each test was proven to detect its intended regression:

- **Baseline (RED attempted, already green):** `cargo test -p sapphire-framework --test features`
  → `3 passed; 0 failed`. Because of this, the real RED phase would arrive only via
  regression; I verified detection by mutating the manifest and re-running (each mutation
  reverted before the next):

  | mutation | expected detector | result |
  |---|---|---|
  | added a `rpc = []` feature line | `the_old_features_are_gone` | FAILED: `the feature "\nrpc =" still exists` ✅ |
  | removed the `sapphire-framework-bridge` dep entry **and** its `dep:` reference | `every_module_feature_has_a_matching_optional_dependency` | FAILED: `the feature bridge has no dependency behind it` ✅ |
  | removed `backend` from the `native` list | `native_includes_what_a_host_needs` | FAILED: `native is missing backend: native = ["workspace", "server", "registry", "keys", "service"]` ✅ |

  Two detection-boundary findings (worth recording, not plan deviations): (a) removing
  only the dep *entry* while keeping the feature's `dep:` reference fails at manifest
  parse time (cargo rejects a `dep:` to a missing dependency) rather than at the test —
  the test catches the realistic regression, a feature whose backing dep is gone; (b)
  removing a feature's *own* entry (`bridge = []`) while keeping the dep entry passes the
  dep test — that check keys on the dependency line, as written in the plan. The compile
  loop in CI covers the opposite side (a feature that exists but doesn't build).

- **GREEN (after the `native` change):** `cargo test -p sapphire-framework --all-features`
  → facade tests `3 passed; 0 failed`; surface tests `2 passed`; workspace-crate suite
  green.

## Verification (all at HEAD `9ef7e4f` unless noted)

- **Per-feature standalone compile — the plan's failure mode:** ran the exact CI loop
  command (with `--locked`) for all 12 features: all green, and no cargo `-W warnings`
  noise (the `unused variable: dim` in `workspace_state.rs` is a cargo-level warning at
  HEAD itself, i.e. pre-existing; clippy `-D warnings` does not flag it, so CI is unaffected).
- **Passthroughs / composite:** `--no-default-features --features workspace,redb-store`,
  `workspace,fastembed-embed`, `native`, `--no-default-features`, default, `--all-features` —
  all compile.
- **fmt / clippy:** `cargo fmt --all -- --check` clean; `cargo clippy --all-targets
  --all-features -- -D warnings` clean.
- **Full suite:** `cargo test --all-features --locked --no-fail-fast`: 690 tests run.
  First run: 1 failed (`sapphire-framework-server` converge `a_host_that_was_offline_catches_up_when_it_returns`,
  "replica store error: Database already open. Cannot acquire lock.") — a listed known
  flake family (`converge`, `reopening_after_eviction_works`); it passed standalone
  (1/5 of the file's tests, filtered out 4) and the **immediately-following full rerun was
  green: exit 0, 690 tests, 58 binaries, 0 failed**. `reopening_after_eviction_works` passed
  in all three runs.
- **Mutation reverting:** after each mutation the manifest was restored from a backup and
  byte-verified clean before continuing; `Cargo.lock` self-heals to its committed state
  (`git status` clean on it at commit time).

## Files changed

- `crates/sapphire-framework/tests/features.rs` (new, 49 lines after fmt)
- `crates/sapphire-framework/Cargo.toml` (one line: `native` without `keys`)
- `.github/workflows/ci.yml` (+10: one step in the `test` job)
- `.superpowers/sdd/2026-09-16-cleanup-plan/progress.md` (ledger line, gitignored, uncommitted)

## Self-review findings

- The brief file referenced by the dispatch (`task-4-brief.md`) does not exist on disk;
  I worked from the plan §Task 4 (the briefs for tasks 1–3 are extracts of it). Verbatim
  tests and the CI loop were taken from the plan exactly.
- The plan's verbatim test ordering is preserved; formatting is the only difference from
  the plan snippet, applied by `cargo fmt` (project convention: fmt must pass).
- CI loop gained `--locked` (rationale above). If the controller prefers the plan's exact
  snippet, dropping the flag is a one-word revert with no other change.
- Dispatch context said "verify lib.rs module re-exports match the feature set" — done
  by compile: each module feature's standalone build compiles `lib.rs`'s
  `pub use … as <name>` for that module (all 12 in the loop, plus `gui` separately).

## Concerns

- None blocking. The two informational notes above (RED-via-mutation because Task 2
  pre-cleaned; `--locked` addition to the CI loop) are judgment calls documented here.
- Pre-existing, out of scope: `unused variable: dim` warning (cargo-level, HEAD) and the
  two known flaky server tests.
