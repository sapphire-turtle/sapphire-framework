# Task 5 Report: `post_install` and privilege separation

**Status:** DONE
**Commit:** `5d9e921` — `feat(service): run post_install, and keep privilege separation coherent`
**Branch:** `feat/p2p-sync-iroh` (on top of `1c4b357` docs ledger through Task 4; service crate state `937d646`)

## What was implemented

### `crates/sapphire-framework-service/src/manager.rs`

- **`install()` runs `post_install` last** — after the unit write, after every activation
  command (and after the Windows hand-over cleanup), with the resolved `InstallContext`.
  A hook failure returns `Error::Manager` whose message carries the hook's own error text
  followed by "the service is installed and running, so fix what the hook needs and rerun
  the uninstall and install of your choice" — the error and the fact that the install
  succeeded and was not undone.
- **`InstallArgs::keep_helper` skips the hook** (`spec.post_install.as_ref().filter(|_| !
  args.keep_helper)`); the install still succeeds and returns the context.
- **Chown via a manager seam.** `trait ServiceManager` gained
  `fn chown_to_user(&self, path: &Path, user: &str) -> Result<()>` with a default no-op
  body. `install()` calls it — only for a Linux system install whose resolved
  `context.target_user` is `Some` — on `context.unit_path`, immediately before the hook
  runs. Rationale (grounded in spec §3.2 and the flows' ownership): the install flow writes
  exactly one file of its own, the unit file, into a root-owned directory; a hook that
  writes "into another user's directories" runs as root and must write files that are
  theirs, so the file the flow owns is handed over first and the hook's own writes then
  land inside a user-owned tree. The hook's file-writing stays the hook author's job (the
  hook receives the resolved `target_user` in the context), matching the trait's shape —
  the manager owns the machine, the spec owns intent. An error from `chown_to_user` fails
  the install: the hook's writes would otherwise be owned by root.
- **`SystemManager::chown_to_user`** resolves `user` through `getpwnam` (while root) and
  calls `chown(2)`; an unknown user is `Error::Config` ("no such user"), a failed chown is
  the OS error. Linux only: the implementations live behind
  `#[cfg(target_os = "linux")]` / `#[cfg(not(target_os = "linux"))]`
  (`hand_over_to_user`), so the crate still compiles on macOS/Windows, where ownership
  change has no counterpart and the flow never calls it. The two `unsafe` blocks carry
  SAFETY comments; `libc` was added as a `cfg(target_os = "linux")` dependency (Cargo.lock
  updated — version already in the lock graph via other crates, no new source).
- **`RecordingManager::ordered(Arc<Mutex<Vec<String>>>)`** — the manager appends one entry
  per `write_unit` (`write_unit <path>`) and per `run` (`run <words...>`) into the shared
  list, so a hook's own pushes interleave with the flow's steps in one observable order.
  Prefixed entries cannot collide with a hook's plain strings.
- Doc comments: `install()` now describes hook ordering, `--keep-helper`, the failure
  semantics, and the hand-over (including that `RunAs::Root`/privsep resolves to no user,
  so nothing changes hands); `chown_to_user` carries its contract on the trait.

### Privilege separation coherence

No production change was needed: `resolve_target_user` already returns `None` for a spec
with `privileges: Some` whatever `system_run_as` says (the `has_privileges` branch), and
`render_unit` renders a root unit with the `SAPPHIRE_RUN_AS` / `SAPPHIRE_HELPER_AS`
environment lines for a system context with no target user. The new verbatim test
`a_privilege_separated_spec_installs_as_a_root_unit_whatever_run_as_says` locks the whole
path end to end through `install()` — the hole the brief closes is now guarded at the flow
level, not only in the two functions. The doc comment stating that privilege separation
wins already lives on `RunAs::Root` (Task 1) and on `resolve_target_user`.

### `crates/sapphire-framework-service/Cargo.toml`

`[target.'cfg(target_os = "linux")'.dependencies] libc = "0.2"` with a comment saying why.

## The 5 verbatim tests (TDD)

All five from the brief are present in the test module, verbatim modulo three compiler
forced deviations, all mechanical:

1. **`Vec::<String>::new()` / `Option::<String>::None` annotations.** On this toolchain
   (rustc 1.98.0, verified with standalone `rustc` reproductions),
   `Vec::new()` followed only by `push("literal")` infers the element type as `&str` —
   `push(&str)` binds `T = &str` directly (there is no `Into`/`From` indirection to defer
   it), and the later `map(String::as_str)` then fails with E0631 (`expected fn(&&str)`).
   The brief's tests are uncompilable as written on this toolchain; a one-token turbofish
   per vec restores the intent with the recorded values unchanged.
2. **`push("post_install".to_owned())` in the ordering test.** Same cause: with the vec
   typed as `String`, a bare `&str` push is E0308. The string recorded is identical.
3. **`privileges_for` built from `crate::privilege::{PrivilegeConfig, HelperSpec}`**, not
   `sapphire_server::` — the established Task 1/2/3 deviation (the types moved into this
   crate so a unit file needs no server dependency).
4. **`privileges_for` was added now** (Task 4 omitted it as dead code; Task 5's tests use
   it), with a comment naming the reason.

## TDD evidence

- **RED:** with the five tests and support code in place and the *Task 4* `install()`
  (which knew nothing of hooks), the run was
  `cargo test -p sapphire-framework-service --lib` →
  `test result: FAILED. 41 passed; 3 failed; 0 ignored` —
  `post_install_runs_after_activation` (order ended on `run systemctl --user enable --now
  sapphire-agent`, no hook ran), `post_install_sees_the_resolved_target_user` (`left:
  None`, the hook never ran), `a_failing_post_install_fails_the_install_and_says_what_was_done`
  (`unwrap_err()` on `Ok`). The other two (keep_helper, privsep root unit) passed on the
  old code — keep_helper because the hook never ran at all, privsep because Task 1's
  `resolve_target_user` already had the branch — which is expected for contracts that were
  already true; they are the regression guards the brief asks for and stay.
- **GREEN:** after the implementation, `cargo test -p sapphire-framework-service` →
  `--lib: 44 passed; 0 failed` (44 = 39 from Task 4 + 5 new), `--test golden: 14 passed;
  0 failed`, no warnings.
- Full suite: `cargo test --all-features --locked --no-fail-fast` → **64 `test result: ok`
  lines, 0 failed**, including `sapphire_framework_service` lib (44) and golden (14).
- `cargo fmt --all -- --check` clean; `cargo clippy --all-targets --all-features -- -D
  warnings` clean.

## Deliberate deviations / notes for the reviewer

1. The three mechanical test deviations above (turbofish annotations, `to_owned()` on the
   one push, crate-local `privileges_for`), each compiler-forced or previously agreed.
2. **Chown is expressed as a manager trait method with a default no-op**, called on the
   unit file only, per the reading grounded in spec §3.2 and the plan's ownership line —
   the reasoning is in the trait's doc comment and `install()`'s. If a later task wants
   `RecordingManager` to *record* chowns, the method is already on the trait; the default
   keeps every existing and future implementor valid.
3. `libc` is a new (target-gated) dependency for the service crate; it is already in
   `Cargo.lock`'s graph via the server crate, so only the service crate's dependency list
   changed.
4. `RecordingManager` gained a private field; its `Debug` derive now shows the `order`
   option — no public API removed or changed.

## Self-review findings (fixed before commit)

- Clippy demanded the nested `if` be collapsed; collapsed once, then restored to the
  readable nested form with a local `#[allow(clippy::collapsible_if)]` and a comment — the
  tuple-let form hid three conditions behind one line.
- `RunAs` was imported for a doc-comment link (`[`RunAs::Root`]`) and clippy flagged it
  unused; intra-doc links resolve without an import, so the import was dropped.
- The Windows hand-over removal runs before the hook (the XML is not the task; the hook
  does not want a path that no longer exists on Windows) — noted here so the ordering is
  visible: unit write → activation → Windows cleanup → chown (Linux system, named user) →
  hook.

## Files changed (in commit `5d9e921`)

- `crates/sapphire-framework-service/src/manager.rs` (+228/-3)
- `crates/sapphire-framework-service/Cargo.toml` (+6)
- `Cargo.lock` (+1)
