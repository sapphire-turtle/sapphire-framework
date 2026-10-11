# Task 1 report: the crate, the spec type, and scope detection

**Status:** DONE (with one plan-ordered dependency decision, documented below)
**Commit:** `99d9481` — `feat(service): decide the scope and the target user`
**Branch:** `feat/p2p-sync-iroh` (base `f33ce1e`)

## What I implemented

Task 1 of the service-install plan: the `sapphire-framework-service` crate, its error type,
the `ServiceSpec` type, and the scope / target-user decision logic.

- `crates/sapphire-framework-service/Cargo.toml` — verbatim per the brief, with two
  deviations forced by the escape hatch (below): the `sapphire-server` dependency removed
  and `serde` added.
- `src/error.rs` — `Error::{Io, Unsupported, MissingUser, Manager, Config}` exactly as the
  brief's Produces list; `Unsupported` and `MissingUser` carry messages, `Io` is
  `#[from]`. `Result` alias. Every variant documented.
- `src/scope.rs` — `Scope`, `RunAs`, `Environment` (with its `Os`), `ServiceSpec` with the
  brief's exact fields (`app_name: &'static str`, `system_run_as`, `privileges:
  Option<PrivilegeConfig>`, `post_install: Option<PostInstall>`), `PostInstall`,
  `InstallContext`, `resolve_scope`, `resolve_target_user`. All public items documented.
  A hand-written `Debug` impl names the `post_install` closure `"set"` instead of
  formatting a function pointer.
- Inline `#[cfg(test)] mod tests` in `scope.rs` — the brief's 11 tests verbatim (no tests
  shell out to any service manager; nothing here touches one).

## The dependency decision (plan Step 1's escape hatch)

The brief says `-service` depends on `-server` "only for `PrivilegeConfig`", and if that
drags in tokio/the workspace stack, to "move `PrivilegeConfig`, `UserSpec` and `HelperSpec`
… into `-service` itself and re-export from `-server`. Decide when you see the dependency
graph; do not leave `-service` pulling in redb to describe a unit file."

I checked `cargo tree -p sapphire-framework-server --no-default-features`: even without
default features the server pulls tokio, sync, workspace, ipc, session, notify and
backend — exactly what the plan told me to avoid. Worse, the plan's own **Task 6** wires
`ServerCommand::Service(ServiceCommand)` (this crate's type) into `-server`'s CLI, locking
the dependency direction as `-server` → `-service`; the brief's original direction would
be an unresolvable cycle once Task 6 lands.

So the escape hatch was taken now, not later:

- `PrivilegeConfig`, `UserSpec`, `HelperSpec` moved verbatim into
  `crates/sapphire-framework-service/src/privilege.rs` (with the five parsing/round-trip
  tests that went with them, and the TOML round-trip test — `serde_json`/`toml` added as
  dev-dependencies for it).
- `sapphire-framework-server` now depends on `sapphire-framework-service` and re-exports:
  `privilege/mod.rs` has `pub use sapphire_framework_service::privilege::{HelperSpec,
  PrivilegeConfig, UserSpec};`, and the crate root's existing `pub use
  privilege::{HelperSpec, PrivilegeConfig, UserSpec};` keeps the public API byte-identical
  — every existing path (`sapphire_framework_server::PrivilegeConfig`,
  `::privilege::PrivilegeConfig`, and `privilege_root.rs`'s
  `privilege::{self, HelperSpec, …}`) compiles unchanged.
- `-service`'s normal dependency graph is now exactly: `clap`, `serde`, `thiserror`,
  `tracing` (verified with `cargo tree -e normal --depth 1`). No tokio, no redb.

## Files changed

- `crates/sapphire-framework-service/{Cargo.toml, src/lib.rs, src/error.rs, src/scope.rs,
  src/privilege.rs}` (new crate)
- `Cargo.toml` (workspace member), `Cargo.lock`
- `crates/sapphire-framework/Cargo.toml`, `src/lib.rs` (`service` feature + re-export +
  doc-table row)
- `crates/sapphire-framework-server/Cargo.toml` (`-service` dependency),
  `src/privilege/mod.rs` (types moved out, re-export in)

## TDD evidence

**RED** — with the brief's 11 scope tests in `scope.rs` and `resolve_scope` /
`resolve_target_user` not yet implemented (earlier in this session, before the dependency
move):

```
error: test failed
failures:
    a_regular_user_gets_a_user_unit
    a_system_unit_off_linux_is_refused
    (…)
```

`a_system_unit_off_linux_is_refused` failed because `Unsupported` carried a `&str` instead
of a `String` — the failure was the message-mismatch the test exists to catch, not a
compile error, so the test did real work in both directions.

**GREEN** — `cargo test -p sapphire-framework-service --all-features --locked scope`:

```
running 11 tests
test scope::tests::a_regular_user_gets_a_user_unit ... ok
test scope::tests::root_gets_a_system_unit ... ok
test scope::tests::sudo_gets_a_system_unit ... ok
test scope::tests::the_scope_can_be_asked_for_explicitly ... ok
test scope::tests::a_regular_user_cannot_ask_for_a_system_unit ... ok
test scope::tests::a_system_unit_off_linux_is_refused ... ok
test scope::tests::a_user_unit_needs_no_target_user ... ok
test scope::tests::a_system_unit_for_an_app_that_drops_its_own_privileges_has_no_user_line ... ok
test scope::tests::a_system_unit_runs_as_the_invoking_user ... ok
test scope::tests::an_explicit_run_as_wins ... ok
test scope::tests::root_without_sudo_user_is_refused_with_an_explanation ... ok

test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 5 filtered out
```

Plus `cargo test -p sapphire-framework-service --locked` → 16 passed (11 scope + 5 moved
privilege tests), doc-tests 0; the moved privilege tests and the server's own 22 privilege
tests all pass against the re-export.

## Verification

| Command | Result |
| --- | --- |
| `cargo test -p sapphire-framework-service --all-features --locked scope` | 11 passed, 0 failed |
| `cargo test -p sapphire-framework-service --locked` | 16 passed, 0 failed |
| `cargo test -p sapphire-framework-server --all-features --locked privilege` | 22 passed, 0 failed |
| `cargo test --all-features --locked` (workspace) | all `test result: ok` lines, 0 failed, no warnings |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo check -p sapphire-framework-service --target x86_64-pc-windows-gnu --locked` | clean |
| `cargo check -p sapphire-framework-server --all-features --target x86_64-pc-windows-gnu --locked` | clean |

The `x86_64-pc-windows-gnu` checks matter here: `PrivilegeConfig` and friends moved
between crates, and both crates now compile on Windows without the `cfg(unix)` privilege
modules.

## The retrieve compile error from the in-progress state

The full-suite compile error in `sapphire-framework-retrieve` did not reproduce —
`cargo check -p sapphire-framework-retrieve --all-features` finishes clean and the
workspace suite compiles and runs. It was a stale/incremental-build artifact, not real
code damage; nothing was changed in `retrieve`.

## The converge flake (pre-existing, not this task's)

`an_edit_made_outside_the_server_is_synced` in `-server`'s `converge` integration test
failed twice during full-suite runs with "Database already open. Cannot acquire lock" — a
redb file-lock collision between test binaries running under cargo's default parallelism.
It passes in isolation and in 4 consecutive isolated runs plus stashed-tree runs; the same
failure appears on the stashed (pre-task) tree only in the parallel full-suite context.
Pre-existing, unrelated to this task's change; noted for the ledger as Minor.

## Self-review findings

- The brief's `Cargo.toml` verbatim differs from what I committed only where the escape
  hatch mandates: no `sapphire-server` dependency; `serde` (a real dependency now —
  `PrivilegeConfig` derives serde on this side); `serde_json`/`toml` as dev-dependencies
  for the moved TOML test.
- `#![warn(clippy::allow_attributes)]` was dropped from the crate root: no other crate in
  the workspace sets it, and the house lint is `#![warn(missing_docs)]`, which the crate
  satisfies.
- I did not carry over the earlier session's out-of-scope `manager.rs` scaffolding
  (a `ServiceManager` trait, `RecordingManager`, a partial `install()` with a hardcoded
  `/home/alice` path): Task 1's brief does not list `manager.rs` — the plan creates it in
  Task 4 with its own TDD tests, and nothing in Task 1's Produces list needs it. A
  dead-code placeholder would have sat untested against golden files that do not exist
  yet.
- The moved `privilege.rs` types are byte-identical to their originals apart from the
  module doc comment, which now explains the move.
- Test output pristine: no stray warnings; the only ignored tests in the workspace are
  the pre-existing root-gated ones.

## Concerns

- The escape hatch was exercised earlier than the plan's "decide when you see the
  dependency graph" phrasing suggests, but with Task 6's CLI embedding it is not a choice:
  leaving the types in `-server` would make Task 6 unbuildable. If the controller prefers
  the re-export to live elsewhere (e.g. a tiny `sapphire-framework-privilege` crate), that
  is a mechanical follow-up — nothing else would change.
- The `converge` redb-lock flake is worth a ledger entry; it can mask real failures in
  CI runs that already carry a privileged job.
