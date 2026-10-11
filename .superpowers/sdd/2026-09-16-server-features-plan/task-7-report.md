# Task 7 report — Update the CLI table and the architecture note

## Status

DONE_WITH_CONCERNS (concern is a pre-existing, unrelated flaky test — see below; the task's
own work is complete and verified).

## What I implemented

### Step 1 — CLI parse test (`crates/sapphire-framework-bridge/src/command.rs`)

Added the brief's test verbatim into the existing `mod tests` block, immediately after
`the_subcommands_parse` and before `there_is_no_way_to_map_a_workspace_from_here`:

```rust
#[test]
fn the_log_subcommand_parses() {
    assert!(Probe::try_parse_from(["b", "log"]).is_ok());
    assert!(Probe::try_parse_from(["b", "log", "--follow"]).is_ok());
    assert!(Probe::try_parse_from(["b", "log", "--lines", "50"]).is_ok());
}
```

It reuses the module's existing `Probe { #[command(subcommand)] command: BridgeCommand }`
helper and the module-local `use clap::Parser;`. The existing parse tests in this module carry
no doc comment, so the new test has none either (match-existing-style convention).

### Step 2 — Operational-surface note (`docs/ARCHITECTURE.md`)

Inserted the brief's blockquote verbatim (text unchanged, including the four hard-wrapped
lines) after the crate table, i.e. after the `sapphire-framework-bridge` /
`apps/sapphire-bridge` rows and the rest of the table's rows, immediately before
`## GUI 向け 非同期 Backend trait`, separated by one blank line.

**Placement note:** the brief said "after the bridge rows". Literally inserting the blockquote
right after the `sapphire-framework-bridge` row would split the markdown crate table across a
blockquote, breaking the table for the `session` / `apps/sapphire-bridge` / `mcp` /
`cache-wasm` rows below it (Markdown tables cannot have a blockquote in the middle). I first
inserted it directly after the bridge row, observed the split, and moved it to the end of the
table. It is still "after the bridge rows", it sits with the bridge material it annotates, and
the table renders intact. This is a rendering-correctness judgement call, not a text change.

## Testing

TDD was specified. The RED step is degenerate, as the brief anticipated: Task 6 (`fb340f8`)
already added the `Log` variant, so the test passes immediately. To make RED meaningful I
temporarily removed the `Log { follow, lines }` variant and its dispatch arm, ran the test,
observed it fail for the expected reason, then restored the file from a backup.

### RED

Removed the `Log` variant + `BridgeCommand::Log { follow, lines } => log_command(...)` arm.

```
cargo test --all-features --locked -p sapphire-framework-bridge the_log_subcommand_parses
```

```
running 1 test
test command::tests::the_log_subcommand_parses ... FAILED

failures:

---- command::tests::the_log_subcommand_parses stdout ----

thread 'command::tests::the_log_subcommand_parses' (530931) panicked at crates/sapphire-framework-bridge/src/command.rs:602:9:
assertion failed: Probe::try_parse_from(["b", "log"]).is_ok()
```

Expected: with no `log` subcommand, `clap` rejects `["b", "log"]`, so the first assertion must
fail. It did. (Removing the variant also produced 4 dead-code/unused warnings in the lib build
of that throwaway tree — expected, and gone after the restore.)

### GREEN

File restored (verified identical byte-for-byte to the intended content; no trace of the
temporary removal in the final diff or commit).

```
cargo test --all-features --locked -p sapphire-framework-bridge the_log_subcommand_parses
```

```
running 1 test
test command::tests::the_log_subcommand_parses ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 131 filtered out; finished in 0.00s
```

Bridge package, all suites:

```
cargo test --all-features --locked -p sapphire-framework-bridge
test result: ok. 132 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.04s
test result: ok. 5 passed; 0 failed; ... (switchboard)
test result: ok. 5 passed; 0 failed; ... (wake / node suites)
test result: ok. 6 passed; 0 failed; ...
test result: ok. 4 passed; 0 failed; ...
test result: ok. 7 passed; 0 failed; ...
test result: ok. 4 passed; 0 failed; ...
test result: ok. 0 passed; 0 failed; ...
```

Full workspace:

```
cargo fmt --all -- --check        # FMT_OK (clean)
cargo clippy --all-targets --all-features -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 7.75s   # no warnings
cargo test --all-features --locked
    EXIT=101 — one failure in an unrelated crate (see concern)
cargo test --all-features --locked --workspace -- --skip a_host_that_was_offline_catches_up_when_it_returns
    EXIT=0   passed=677 failed=0 ignored=0   # zero compiler/test warnings
```

### Concern: pre-existing flake, unrelated to this task

`-p sapphire-framework-server --test converge` / `a_host_that_was_offline_catches_up_when_it_returns`
fails intermittently with:

```
Rpc(RpcError { code: -32603, message: "replication failed: replica store error:
Database already open. Cannot acquire lock.", data: None })
```

at `crates/sapphire-framework-server/tests/converge.rs:155:10`. This is a redb lock race in the
server's converge test, not caused by my change:

- It reproduces **on clean HEAD (`fb340f8`) with my diff stashed** — 1 failure in 3 runs
  (`ok / FAILED / ok`).
- With my diff applied the runs were `FAILED / ok / FAILED` — the same intermittent pattern.
- My diff touches only `command.rs` tests (a `#[cfg(test)]` parse assertion) and
  `docs/ARCHITECTURE.md`; it cannot affect the server crate's replication locking.

I left it alone: it is outside this task's brief, which is a one-commit docs/test touch-up, and
fixing a server replication race would be an unplanned change on the final task of the plan.

## Files changed

- `crates/sapphire-framework-bridge/src/command.rs` — +7 lines (one test).
- `docs/ARCHITECTURE.md` — +5 lines (blockquote note + preceding blank line).
- No other file touched. `.superpowers/sdd/...` is gitignored, as expected.

## Commit

- `504d7f2 docs(bridge): record the operational surface` — exact message from the brief,
  on `feat/p2p-sync-iroh`, parent `fb340f8`. Work tree clean afterwards.

## Self-review findings

- **Completeness:** both steps done; test assertions match the brief character-for-character;
  note text matches the brief character-for-character (verified by grep against the plan's
  copy at `docs/superpowers/plans/2026-09-16-server-features-plan.md:1115`).
- **YAGNI:** no extra assertions, no table-row edits, no unrelated doc cleanup.
- **Testing:** the assertion checks real `clap` parsing through the real `BridgeCommand` tree,
  not a mock; RED was demonstrated by actually removing the variant rather than assumed.
- **Style:** no doc comment added because the neighbouring parse tests have none; the note
  matches the file's existing `> **...**:` blockquote style.

## Concerns

1. (Reported above) The pre-existing `converge` flake makes a bare `cargo test --all-features
   --locked` non-deterministically non-zero. Worth a separate task if the plan wants a
   deterministic final full-suite run.
2. The note's literal placement ("after the bridge rows") was adjusted by one table length to
   keep the crate table rendering. If the plan author truly wanted the blockquote to interrupt
   the table, this needs a re-dispatch.
