# Task 3 report: Undo the per-kind directory split

Plan: `docs/superpowers/plans/2026-09-16-cleanup-plan.md` §Task 3
Brief: `.superpowers/sdd/2026-09-16-cleanup-plan/task-3-brief.md`
Branch: `feat/p2p-sync-iroh` · Base: `4d413bc` (Task 2 commit `ea5fb2c` is HEAD~1's parent side)
Commit: **`aa028dd` `refactor(workspace)!: undo the per-kind directory split`** — 2 files, +328/−209

## What was implemented

`crates/sapphire-framework-workspace/src/app_dirs.rs`:

- Deleted `migrate_app_dir`; added **`unsplit_app_dir`** (brief's reference
  implementation verbatim, signature `(&Path) -> io::Result<PathBuf>`): creates
  the app dir if missing, walks `[Server, Desktop, Cli]` (server's copy wins),
  moves every entry of `<app>/<kind>/` up to `<app>/`, skips + warns when the
  target exists ("two kinds left a directory for one workspace… the other is
  left in place"), falls back to a warning only when `rename` fails (the
  caller-shared `move_item`/`copy_then_delete` still cover the keys migration's
  cross-device case), and removes the kind dir **only when it emptied out**.
- `migrate_keys_to_data` unchanged: still reads the per-kind
  `<app>/<kind>/<uuid>/keys.toml` layout, moves into `<data-app>/<kind>/<uuid>/`,
  once-per-uuid guard.
- Doc comment on the module rewritten to describe the flattened layout
  (`<platform-root>/<app-name>/…`, `AppKind` never in a path) and the two
  migrations (unsplit + keys).

`crates/sapphire-framework-workspace/src/context.rs`:

- `init` resolves each category root with **no kind in the path**
  (`<env-or-platform-root>/<app_name>`), runs the migrations in the order that
  matters — `migrate_keys_to_data` **first** (the per-kind cache layout still
  exists), then `unsplit_app_dir` on cache, data and config app dirs — then
  stores all three (`OnceLock`, first writer wins). Doc comments updated
  (English, mention both migrations and that `kind` now only steers the secrets
  migration).
- `category_root` behavior unchanged: env var replaces the platform root only.

## Deviation from the brief (1, test-only)

The brief's `the_resolved_cache_path_no_longer_contains_a_kind` set the env var
with raw `unsafe { std::env::set_var }`. This crate's `test_env.rs` requires all
env mutations in tests to go through its `TestEnv` helper, so the test uses
`TestEnv::lock()` + `TestEnv::set/remove` and additionally pins the data and
config categories to their own tempdirs so `init` never touches real platform
directories. The naming rule itself needed no adaptation:
`app_dir_env_var("sapphire-unsplit", "cache")` already produces
`SAPPHIRE_UNSPLIT_CACHE_DIR`, exactly as the brief asserted.

## TDD evidence

**RED** — appended the 9 `unsplit_tests` (brief's tests verbatim, minus the
adapted env-var one), ran before any implementation:

```
$ cargo test -p sapphire-framework-workspace --all-features unsplit
error[E0425]: cannot find function `unsplit_app_dir` in this crate   (×9, one per test)
```

Expected: `unsplit_app_dir` did not exist yet.

**GREEN** — after implementing `unsplit_app_dir` + rewriting `context.rs`:

```
$ cargo test -p sapphire-framework-workspace --lib --all-features --locked
running 45 tests …
test result: ok. 45 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

(9 `unsplit_tests` + 4 existing `app_dirs::tests` + 7 `context::tests`; the
`context::tests::init_moves_keys_toml_from_the_cache_tree_into_the_data_tree`
test from the pre-reboot session passes as-is with the new order — keys
migrate from the still-per-kind cache tree first, then `unsplit_app_dir` runs.)

## Verification

- `cargo test --all-features --locked --no-fail-fast` (57 test binaries):
  **677 passed, 1 failure — the documented pre-existing flake**
  `-p sapphire-framework-server --test converge ::
  a_host_that_was_offline_catches_up_when_it_returns` ("replica store error:
  Database already open. Cannot acquire lock." — redb lock under parallel
  load). Re-run standalone: **5/5 passed**. Same failure signature Task 2's
  report recorded; a control run with my changes stashed reproduced it at
  clean HEAD too (4/6 runs failed pre-reboot), so it is unrelated to Task 3.
- `-p sapphire-framework-workspace --lib`: 45/45, re-run 5× consecutively (0
  flakes).
- `cargo fmt --all -- --check` — clean.
- `cargo clippy --all-targets --all-features --locked -- -D warnings` — clean.
- `git status` after commit — clean.

### Environment reliability note (report context only)

The machine was rebooted mid-task (Session 2); post-reboot the toolchain was
missing — `rustup toolchain link stable ~/.local/state/beads/rust-stable` and
`rustup toolchain link nightly ~/.local/state/beads/rust-nightly` restored it
(keep in mind for future sessions). ~22k stale `~/.tmp*` tempdirs left by the
dead session were cleaned up; `/tmp` no longer contains dead test-run state
that could confuse tests.

## Self-review notes

- Brief's file list fully covered (`app_dirs.rs`, `context.rs` only); no
  scope extensions. `migrate_keys_to_data`, `move_item`, `copy_then_delete`,
  `copy_path` untouched.
- `unsplit_app_dir` is the brief's reference implementation verbatim.
- The 9 brief tests all present; only the env-var test was adapted (see
  Deviation) and its data/config pinning makes it hermetic.
- Test output pristine (no stray warnings).
- `.superpowers/sdd/` is gitignored, per the brief.

## Files changed

- `crates/sapphire-framework-workspace/src/app_dirs.rs` (modified: −
  `migrate_app_dir`, + `unsplit_app_dir`, module doc rewritten, `unsplit_tests`
  module appended)
- `crates/sapphire-framework-workspace/src/context.rs` (modified: `init`
  rewired to unsplit semantics, doc comments updated)

## Status

DONE
