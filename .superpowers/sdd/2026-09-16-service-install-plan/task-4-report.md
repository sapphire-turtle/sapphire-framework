# Task 4 Report: `ServiceManager` and `ServiceCommand`

**Status:** DONE
**Commit:** `937d646` — `feat(service): install, uninstall and report through a manager trait`
**Branch:** `feat/p2p-sync-iroh` (on top of `53b8169` docs(sdd) ledger through Task 3)

## What was implemented

### `crates/sapphire-framework-service/src/manager.rs` (new, 654 lines)

- `trait ServiceManager { write_unit, run, remove_unit }` — the one seam everything goes
  through. `run` takes a program-plus-arguments vector and never goes through a shell, so no
  word needs quoting.
- `SystemManager` — the real one: writes files (creating a user unit's directory if missing),
  runs commands via `std::process::Command` (non-zero exit becomes `Error::Manager` with the
  stderr text), and treats removing a missing file as success. **The only place in the crate
  that touches the machine.**
- `RecordingManager` + `Calls { units, commands, removed }` — records everything, answers from
  canned output. `failing_on(needle)` matches the needle against the command's own words
  (so `"enable"` fails `enable --now` alone and leaves `daemon-reload` working);
  `returning(text)` provides status output. Commands are recorded **before** the failure
  check, so a test asking what was attempted sees failed commands too.
- `ServiceCommand::{Install(InstallArgs), Uninstall, Status}` — `clap::Subcommand`.
- `InstallArgs { user, system, run_as, keep_helper }` — `clap::Args`; `requested_scope()`
  maps the flags to `Option<Scope>` and refuses both at once with a message naming `--user`.
- `install(spec, args, env, manager) -> Result<InstallContext>` — resolves scope
  (`resolve_scope`) and target user (`resolve_target_user`), renders the platform's file,
  writes it, runs the activation commands, and **removes the written unit before returning**
  if any activation command fails. This is the brief's key case: a unit written but never
  enabled is invisible to `status` and springs to life at the next reboot. If the cleanup
  itself fails, the returned error says so and names the leftover path.
- `uninstall(...)` — stop, disable, remove; idempotent (stop/disable failures are ignored
  because "not running" is the state an uninstall wants; the file removal is propagated, and
  `SystemManager` treats a missing file as success).
- `status(...)` — one platform command, its output returned verbatim.
- Platform dispatch by `env.os`: Linux uses `systemd::{unit_path, activation, render_unit}`;
  macOS user level uses `launchd::{agent_path, label, render_launch_agent}` with
  `launchctl bootstrap|bootout|print gui/<euid>`; Windows uses `windows::render_task` with
  `schtasks /create|/delete|/query`. System scope stays Linux only — `resolve_scope` refuses
  it off Linux before any of this runs.
- Windows extra: after a successful `/create` the hand-over XML under the temporary
  directory is removed (the scheduler keeps its own copy). A leftover hand-over file failing
  an otherwise successful install would be the wrong trade, so that removal is best-effort.

### `crates/sapphire-framework-service/src/lib.rs`

Registered `pub mod manager;` and re-exported `Calls, InstallArgs, RecordingManager,
ServiceCommand, ServiceManager, SystemManager, install, status, uninstall`; crate doc gained a
paragraph pointing at the flows.

## Deliberate deviations from the brief (all noted, all consistent with Tasks 1–3)

1. **`sapphire_server::PrivilegeConfig` → `crate::privilege`.** The brief's test helper
   references `sapphire_server::{PrivilegeConfig, HelperSpec}`; per Task 1's documented move
   those types live in this crate. Same deviation as Tasks 2/3.
2. **`privileges_for` helper omitted.** The brief's `privileges_for(run_as, helper)` is not
   used by any Task 4 test (only Task 5's do), so including it would trip `-D warnings` as
   dead code. It belongs in Task 5's test module.
3. **Guard-test needle assembled.** The verbatim `no_test_touches_the_real_service_manager`
   counts `"SystemManager"` occurrences in `include_str!("manager.rs")`. As written the test's
   own text contains the token three times (comment, `matches()` literal, assert message), so
   the count could never be ≤ 2 — the test was unsatisfiable verbatim. Fix preserving both
   mechanism and threshold: the needle is built as `concat!("System", "Manager")` and the
   comment/message wording avoids the bare token. The file now contains exactly 2 occurrences:
   `pub struct SystemManager;` and `impl ServiceManager for SystemManager` (verified by
   `grep -c`). The only test-code change in the whole module.
4. **`home_dir` / `install_path` signature.** The brief gives no signature for these; they are
   private. `install_path` takes `&Environment` and resolves the home directory only where it
   is used — a system unit lives in `/etc` whatever `HOME` says, so a system install must not
   fail for want of a variable it never consults.
5. **macOS/Windows command words** (`launchctl bootstrap|bootout|print`, `schtasks
   /create|/delete|/query`, `gui/<euid>` target, `/f`) are not specified in the brief or the
   plan; chosen from each tool's documented interface. Task 6 (CLI wiring) is where they will
   first run for real.

## TDD evidence

- **RED:** after writing the brief's 9 tests, the implementation was temporarily replaced by a
  declarations-only skeleton (signatures + `Err("not implemented")` bodies) and run:
  `cargo test -p sapphire-framework-service --lib` →
  `test result: FAILED. 28 passed; 8 failed` — the 8 behavioural tests panicked on
  `Manager("not implemented")`; the 9th (the static-source guard) passed, correctly, since it
  reads text rather than exercising behaviour. Expected: the flows did not exist yet.
- This first RED compile also surfaced a genuine bug in my real implementation —
  `error[E0277]: the trait bound Mutex<Calls>: Clone is not satisfied` — fixed by dropping the
  impossible `Clone` derive from `RecordingManager` (`Mutex` is not `Clone`; tests hold one
  manager and query it, which is all that is needed).
- **GREEN:** restoring the implementation, then
  `cargo test -p sapphire-framework-service` →
  `--lib: 36 passed; 0 failed`, `--test golden: 14 passed; 0 failed`.
- After the self-review additions (3 further tests, below): `--lib: 39 passed; 0 failed`,
  `--test golden: 14 passed; 0 failed`, output pristine (no warnings).

## Self-review findings (fixed before commit)

- **`Mutex<Calls>` is not `Clone`** — found by the RED compile; derive removed.
- **System install demanded `HOME` for nothing.** `install()` originally called `home_dir(env)?`
  unconditionally; a Linux system install with `HOME` unset (a plausible environment for a
  root service install) would have failed although `unit_path` ignores the home directory, and
  a Windows install would have demanded `USERPROFILE` for a temp-file hand-over.
  Fixed by resolving the home directory inside `install_path` only for the branches that use
  it. A first attempt keyed on `euid == 0` was itself wrong (root can pass `--user`); the
  final version keys on the resolved `Scope`.
- **Non-Linux dispatch was untested.** The plan mandates the macOS/Windows paths but the
  brief's tests exercise Linux only. Added 3 tests — macOS install writes a `.plist` and runs
  `launchctl bootstrap`; Windows install runs `schtasks /create`; a system install reads no
  home directory — taking the module to 12 tests. These verify behaviour that already existed;
  no implementation change accompanied them.
- **`status` shaped as a `Vec` with an unreachable fallback** — replaced with `status_command`
  returning one command, removing a dead branch.
- No test invokes `systemctl`, `launchctl` or `schtasks`: the only `Command::new` in the crate
  is inside `SystemManager::run` (line 77), and no test constructs `SystemManager` (the guard
  test enforces this over the file's text). Every test drives a `RecordingManager`.

## Verification (before commit)

- `cargo test -p sapphire-framework-service --all-features` → 39 + 14 + 0 passed, 0 failed.
- `cargo test --all-features --locked --no-fail-fast` → **64 test targets, all ok, zero
  failures** on the run immediately before commit.
- `cargo fmt --all -- --check` → clean.
- `cargo clippy --all-targets --all-features -- -D warnings` → clean.

## Known flakes (pre-existing, unrelated to this task)

- `sapphire-framework-server --test converge ::
  a_host_that_was_offline_catches_up_when_it_returns` — the documented redb-lock flake. It
  failed once during this task's full-suite runs and passed on the next full run; two isolated
  runs gave FAIL then PASS, reproducing the documented intermittency at `converge.rs:155`. Two
  subsequent full-suite runs were fully green. No action taken; flagged for the server's
  convergence-test owner.

## Files changed (in commit `937d646`)

- `crates/sapphire-framework-service/src/manager.rs` (new, 654 lines)
- `crates/sapphire-framework-service/src/lib.rs` (+9: module, re-exports, crate doc)
