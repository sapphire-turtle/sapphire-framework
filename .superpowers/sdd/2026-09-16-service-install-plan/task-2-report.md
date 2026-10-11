# Task 2 report: systemd units, against golden files

**Status:** DONE
**Commit:** `92a01f6` — `feat(service): render systemd units`
**Branch:** `feat/p2p-sync-iroh` (base `53a7280`)

## What I implemented

- `crates/sapphire-framework-service/src/systemd.rs` — the four functions from the brief's
  Produces list, all with doc comments:
  - `unit_path(app_name, scope, home)` — `~/.config/systemd/user/<app>.service` for
    `Scope::User`, `/etc/systemd/system/<app>.service` for `Scope::System`.
  - `render_unit(spec, ctx)` — the three goldens' exact text. Scope decides the shape: a
    user unit has no network ordering, `WantedBy=default.target`; a system unit has
    `After=/Wants=network-online.target`, `WantedBy=multi-user.target`, and either a
    `User=` line (named target user) or none at all with the privilege-separation comment
    plus `SAPPHIRE_RUN_AS` / `SAPPHIRE_HELPER_AS` environment lines.
  - `activation(app_name, scope)` — `[["systemctl","--user","daemon-reload"],
    ["systemctl","--user","enable","--now",<app>]]` (user) / same without `--user`
    (system), exactly the brief's command shapes.
  - `linger_hint(scope, user)` — the `loginctl enable-linger` sentence for a user unit
    (`sudo` prefix when a user is named), `None` for system.
- `crates/sapphire-framework-service/src/lib.rs` — module declaration and
  `pub use systemd::{activation, linger_hint, render_unit, unit_path};`.
- `tests/golden.rs` + `tests/golden/{user,system-user,system-privsep}.service` — the
  brief's three golden files and eight tests, created deliberately from the brief's text
  (no snapshot tooling involved).

## The one deliberate deviation from the brief's literal text

The brief's `golden.rs` references `sapphire_server::PrivilegeConfig` and
`sapphire_server::HelperSpec`. Task 1 (per dispatch context and its report) **moved** those
types into this crate, with `-server` re-exporting them for public-API compatibility. So
the test imports them from `sapphire_framework_service::{HelperSpec, PrivilegeConfig, …}`
instead — the types live here; referencing `sapphire_server::` still compiles but would
drag the whole server runtime stack (tokio, redb) in as a dev-dependency of the service
crate, exactly what the move was made to avoid. Nothing else in the brief's test text was
changed.

## TDD evidence

**RED** — with the golden files and `tests/golden.rs` in place and `systemd.rs` not yet
implemented (it did not exist; the module was not yet wired):

```
error[E0432]: unresolved import `sapphire_framework_service::render_unit`
  --> crates/sapphire-framework-service/tests/golden.rs:9:77
   |
9  |     … Scope, ServiceSpec, render_unit,
   |                           ^^^^^^^^^^^ no `render_unit` in the root
error: could not compile `sapphire-framework-service` (test "golden") due to 1 previous error
```

Exactly the brief's expected failure ("`render_unit` does not exist").

**GREEN** — after implementing `src/systemd.rs` and wiring the module:

```
running 8 tests
test a_privilege_separated_unit_has_no_user_line ... ok
test a_user_unit ... ok
test a_system_unit_running_as_a_named_user ... ok
test a_user_unit_has_no_network_ordering ... ok
test exec_start_is_absolute_and_carries_the_arguments ... ok
test a_system_unit_that_drops_its_own_privileges ... ok
test a_system_unit_waits_for_the_network ... ok
test a_privilege_separated_unit_names_both_users ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Verification

| Command | Result |
| --- | --- |
| `cargo test -p sapphire-framework-service --test golden --locked` | 8 passed, 0 failed |
| `cargo test -p sapphire-framework-service --locked` | 23 unit (incl. 7 new `systemd` unit tests for `unit_path`/`activation`/`linger_hint`) + 8 golden + 0 doc-tests, all pass |
| `cargo test --all-features --locked --no-fail-fast` | 708 passed, 1 failed, 9 ignored — the 1 is the pre-existing converge flake (below) |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy -p sapphire-framework-service --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` (workspace) | clean |
| byte-level `cmp` of each rendered unit vs its golden | byte-identical (see below) |

## Byte-level golden check (beyond the brief's `trim_end()`)

The golden tests compare with `trim_end()`, which could hide a trailing-newline
difference. I rendered the three units through a temporary example binary and `cmp`-ed
them byte-for-byte against the checked-in goldens: **all three byte-identical**, so the
renderer emits exactly the golden text with a trailing newline and no hidden whitespace
drift. The temporary example was removed before commit (not part of the commit; verified
`git status` clean of it).

## The full-suite flake (pre-existing, not this task's)

`cargo test --all-features --locked` (fail-fast) failed on
`a_host_that_was_offline_catches_up_when_it_returns` (`-server` `converge` integration
test) with `Database already open. Cannot acquire lock` (redb file-lock) in each of three
full-suite runs; an earlier run flaked a sibling test (`host::tests::reopening_after_eviction_works`,
a tantivy `IndexWriter` lock) the same way. Both:

- pass in isolation (`--test converge` alone: 5/5, twice; the host test alone: ok);
- pass on the **stashed pre-task tree** run the same way (`git stash -u` → converge 5/5),
  so this is not introduced by this task.

This matches the flake Task 1's report documented for the ledger (same converge suite,
same redb-lock mechanism, load-dependent). With `--no-fail-fast` the whole workspace runs
to completion: **708 passed, 1 failed (only that flake), 9 ignored**, no other failure
anywhere, including everything in `-service`.

## Self-review findings

- The brief's test file references `sapphire_server::` types (deviation documented above —
  the only change to the brief's literal test text; assertions, test names and golden
  bodies are verbatim).
- No test invokes `systemctl`, `launchctl` or `schtasks`: the eight golden tests only
  render strings and compare; the seven inline `systemd.rs` unit tests only assert on
  returned `PathBuf`/`Vec<Vec<String>>`/`Option<String>` values.
- `render_unit` ignores `RunAs` in favour of `ctx.target_user` (the resolver already
  collapsed them in Task 1: `RunAs::Root` resolves to `None`), which is why the brief's
  Produces signature takes only `(spec, ctx)`. `ctx.scope` drives the branch; `ctx.exe`
  and `spec.args` build `ExecStart`. `app_name`/`unit_path` in the ctx are not needed for
  rendering (the path function is a separate Produces item); noted for Task 4's wiring.
- fmt had to rewrap three spots (one in `systemd.rs` tests, two in `golden.rs`) after the
  initial write; committed tree is fmt-clean.
- A first draft of `systemd.rs` was cut off mid-write by a tool-budget interruption; the
  committed file is a complete clean rewrite (verified by reading it back and by the full
  passing suite).

## Concerns

- The renderer matches the golden contract exactly, but it is minimal: helper `args`/`program`
  beyond the two environment lines the goldens fix, and anything else a hardened unit
  would want (`ProtectSystem` etc.), are simply not emitted — consistent with the goldens
  and this task's scope. If a later task or reviewer expects more, that is a golden
  regeneration decision, which per the plan is made deliberately by reading diffs.
- The converge/lock flakes are worth the ledger entry Task 1 already flagged: two sibling
  tests in `-server` can flake under full-suite parallelism (redb file lock; tantivy
  `IndexWriter` lock), and they can mask real failures in fail-fast runs.
