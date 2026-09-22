# Task 6 Report — The log, and `bridge log`

**Status: DONE** · Commit `fb340f8` on `feat/p2p-sync-iroh` — `feat(bridge): write a log any process can read, and add bridge log`

## What was implemented

**`crates/sapphire-framework-bridge/src/logging.rs` (new)** — the log and its reader:

- `LOG_FILE = "node.log"`, `LOG_MAX_BYTES = 10 MiB`, `LOG_KEEP = 3`, plus two exported
  constants the brief implied but did not name: `DEFAULT_LOG_FILTER` (the console default
  `sapphire_framework_bridge=info`, shared by the binary) and `BRIDGE_TARGET`
  (`sapphire_framework_bridge`, the target prefix the file layer keeps).
- `install(&BridgeDir) -> Result<LogGuard>` / `install_with_limit(dir, limit)` — install is
  `install_with_limit(dir, LOG_MAX_BYTES)`; the rotation test exercises the real path.
  Events of the bridge's own targets (`sapphire_framework_bridge…`, prefix match via
  `tracing_subscriber::filter::Targets`) go to `<bridge dir>/logs/node.log` **in addition**
  to console output: one subscriber holds both layers, the console layer keeping the
  env-filter (`RUST_LOG`, else `DEFAULT_LOG_FILTER`) the binary always had, the file layer
  ANSI-free at TRACE.
- **Size rotation, hand-rolled.** `tracing-appender` 0.2.5 rotates only by *time*
  (`Rotation::MINUTELY/HOURLY/DAILY/NEVER`) — there is no size-based rotation — so the
  `Rotating` writer tracks bytes written alongside the open file (no per-write `stat`) and
  on passing `limit` shifts `node.log.1→.2`, `node.log.2→.3`, drops `node.log.3`, renames
  the current file to `node.log.1`, and opens a fresh one: exactly `LOG_KEEP + 1` files.
- **Append, not truncate** — the file is opened with `OpenOptions::append(true)`, and the
  writer's byte count starts at the file's current size, so a restart continues the same
  record. This is the `a_restart_continues_the_same_file` contract.
- `tail(&BridgeDir, lines) -> Result<Vec<String>>` — last N lines in order; missing log is
  `Ok(vec![])`, not an error.
- `follow(dir, lines, out, stop)` (`pub(crate)`) — `tail -f`: prints the tail, then polls
  (250 ms) printing only new bytes; on shrink/rotation it restarts at offset 0; `stop` is
  checked only *after* each drain so a stop raised while lines landed still prints them.

**Design note (the reason for the wire):** `tracing`'s global subscriber is **once-only**
(`set_global_default` is a compare_exchange on an INIT flag; verified in
`tracing-core-0.1.36/src/dispatcher.rs:299`), and `WorkerGuard` is flush-on-drop with
private fields. The brief's restart test calls `install` twice in one process, so a naive
"set a fresh global each time" is impossible. Instead the subscriber (console + file layer)
is built **once** around a static wire (`WIRE: OnceLock<Arc<RwLock<Option<Arc<NonBlocking>>>>>`),
and each `install` swaps which writer the wire feeds; `LogGuard::drop` unhooks the wire
(only if it is still the routed writer) and then flushes the worker. Consequence:
`apps/sapphire-bridge/src/main.rs` calls the new `sapphire_bridge::install_console()`
instead of `tracing_subscriber::fmt().init()` — if the binary installed its own subscriber
first, the file layer's `try_init` would lose the global slot and the log would stay empty.
`command.rs`'s `run()` calls `install(&dir)` **after `InstanceLock::acquire`** (one writer
is the lock's guarantee) and holds the guard for the bridge's lifetime; it also emits the
record's first line — `sapphire-bridge <version> starting (pid N)` — on `BRIDGE_TARGET`,
so every run visibly opens the record and a fresh `logs/` directory is not empty until the
first event. (Verified by smoke test: before this line, a healthy quiet bridge wrote
nothing for 20 s and `node.log` did not exist, since the `Rotating` file opens lazily on
first write.)

**`command.rs`** — `BridgeCommand::Log { follow: bool, lines: usize }`
(`--follow` flag; `--lines` with `default_value = "20"`, which Task 7's parse test
requires) → `log_command`: pure file read via `BridgeDir::open()`, no live call; no log →
`the bridge has not written a log yet`, exit 1; `--follow` loops until interrupted (the
way `tail -f` ends).

**Cargo manifests** — `tracing-appender = "0.2"` in workspace deps;
`sapphire-framework-bridge` gains `tracing-appender.workspace = true` and
`tracing-subscriber = { workspace = true, features = ["env-filter", "registry"] }`
(`registry` is *not* a default feature — verified; `env-filter` for the console layer).
`Cargo.lock` updated accordingly.

## Tests

`#[cfg(test)] mod tests` in `logging.rs`: the brief's **5 tests verbatim**, plus one more
(`following_prints_the_lines_that_arrive`: a thread appends lines mid-follow; the stop flag
carries a 10 s deadline so a failed append fails assertions rather than hanging). The
install-driven tests serialize on a module `INSTALL_LOCK` — the one global subscriber is
process-wide and `cargo test` runs tests in parallel, so without it one test's events could
land in another test's file. The extra lines other tests' events might add are harmless:
every assertion is `contains` or counts *files*, not lines.

### TDD evidence

- **RED** — stub with `todo!()`; `cargo test -p sapphire-framework-bridge --all-features logging`
  → `5 failed; 0 passed`: `not yet implemented: the log writer` / `... the log tail` — the
  expected failure mode (unimplemented, not type errors).
- **GREEN** — same command → `6 passed; 0 failed` (5 brief + follow). The captured output
  also shows the console layer emitting `INFO sapphire_framework_bridge: first run` etc.
  while the file assertions pass — "in addition to", verified.

### Full verification (commands + results)

- `cargo test --workspace --all-features --locked` → exit 0, **61 suites, 677 passed,
  0 failed** (final run; the known flake
  `server::converge::a_host_that_was_offline_catches_up_when_it_returns` was green here —
  it had failed once in an earlier run, as documented, ~50%).
- One earlier workspace run also saw `pairing_e2e` fail all 6 tests at
  `tests/common/mod.rs:156` ("the bridge never started") under full compile+test load;
  the suite passes alone (6/6) and passed in the final full run. Same load-flake family as
  the documented one, not touched by this diff (`install` is only in the CLI's `run()`).
- `cargo fmt --all -- --check` → clean.
- `cargo clippy --all-targets --all-features --locked -- -D warnings` → exit 0.

### CLI smoke tests (real binary)

- fresh dir → `the bridge has not written a log yet`, exit 1
- hand-written file → tail prints all 5 lines, exit 0; `--lines 2` → `line 3/line 4`
- `sapphire-bridge run` → `<bridge dir>/logs/node.log` contains
  `INFO sapphire_framework_bridge: sapphire-bridge 0.14.0 starting (pid …)`; a second shell's
  `bridge log` reads it; `--follow` picked up appended lines and ended on SIGINT.

## Files changed

- `crates/sapphire-framework-bridge/src/logging.rs` (new, 632 insertions total across diff)
- `crates/sapphire-framework-bridge/src/command.rs` (Log variant, dispatch, `log_command`,
  install in `run()`, startup log line, `std::io::Write` import)
- `crates/sapphire-framework-bridge/src/lib.rs` (module + re-exports)
- `crates/sapphire-framework-bridge/Cargo.toml`, `Cargo.toml`, `Cargo.lock` (deps)
- `apps/sapphire-bridge/src/main.rs` (`install_console()` instead of own `init()`

## Self-review findings

- **Fixed during work:** first design set a fresh global per `install` — impossible
  (once-only global, verified in source). Rewritten to the one-subscriber/wire design.
- **Fixed during work:** `main.rs` kept its own `init()` at first, which would have raced
  `install` for the global slot; replaced with `install_console()`.
- **`bridge log` reads only the current file** — rotated `node.log.1…` are not tailed.
  The brief's tail contract (`tail(&dir, lines)` on `LOG_FILE`) is what `--lines` reads;
  following across rotation is handled by the follower's shrink-restart. Reading the full
  rotated history is left to the files themselves (`cat logs/node.log.*`).
- Rotation is approximate (one event of overshoot past `limit`) — documented in the API doc.
- `install_with_limit` with `limit = 0` would rotate every event into `node.log.1`; not
  guarded — it is a test-only knob (`install` always passes `LOG_MAX_BYTES`), noted here
  rather than guarded (YAGNI).

## Concerns

- Two pre-existing flake families observed (documented above), both load-related, neither
  touched by this diff; the final full run was entirely green.
