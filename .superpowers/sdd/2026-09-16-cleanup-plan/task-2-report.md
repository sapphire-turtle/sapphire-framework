# Task 2 report: Delete the HTTP sync stack

Plan: `docs/superpowers/plans/2026-09-16-cleanup-plan.md` §Task 2
Brief: `.superpowers/sdd/2026-09-16-cleanup-plan/task-2-brief.md`
Branch: `feat/p2p-sync-iroh` · Base: `98b36a8` (`refactor(keys)!`)

## What was implemented

Deleted the four HTTP-sync crates (`git rm -r`):

- `crates/sapphire-framework-rpc` (serde-only shared types + JSON-RPC envelope)
- `crates/sapphire-framework-remote-client` (reqwest JSON-RPC client)
- `crates/sapphire-framework-remote-server` (axum single `POST /rpc` server, WsStore, change log)
- `crates/sapphire-framework-blob` (BlobStore / FsBlobStore)

Manifest changes:

- Root `Cargo.toml`: the four `members` entries removed.
- `crates/sapphire-framework-backend/Cargo.toml`: `sapphire-remote-client`,
  `sapphire-rpc` dependencies removed; `sapphire-remote-server` dev-dependency removed.
- `crates/sapphire-framework-backend/src/remote.rs` deleted (the `RemoteBackend`
  implementation with its in-process-server tests).
- `crates/sapphire-framework-backend/src/error.rs`: `Error::Remote`
  (`From<sapphire_remote_client::Error>`) and `Error::Conflict { paths }` removed;
  `InvalidWorkspace` doc updated ("both `path` and `url` set" → "unknown workspace id").
- `crates/sapphire-framework-backend/src/source.rs`: `WorkspaceLocator` is now
  `Path`-only (`Local(PathBuf)` single variant): the `Remote { url, ws, token }`
  variant, `WorkspaceLocator::remote()`, the `http(s)://` special-casing in
  `parse()`, `is_remote()`, `DEFAULT_WS`, `WorkspaceEntry::{url, token, remote()}`,
  `WorkspaceSelection::{ad_hoc_url, token}` and `WorkspaceSource::Remote` are all
  gone. `WorkspaceSource` is `Local { state }` only and `into_backend()` builds a
  `LocalBackend`. Tests rewritten for the path-only world.
- `crates/sapphire-framework-backend/src/lib.rs`: `remote` module, `RemoteBackend`
  and `RemoteClient` re-exports removed; crate doc rewritten for the two remaining
  implementations (`LocalBackend`, `IpcBackend`); `DEFAULT_WS` dropped from the
  re-export list.
- Facade `crates/sapphire-framework/{Cargo.toml,src/lib.rs}`: features `rpc`,
  `blob`, `remote-client`, `remote-server` and their optional dependencies and
  re-exports removed; `prelude` entries `RemoteBackend`/`RemoteClient`/`ServerState`
  /`Uuid`/`WsStore`/`WsStoreConfig`/`router`/`serve` dropped; crate doc table and
  examples updated. Task 4 owns the full feature-list rework, so `keys` keeps its
  current shape and `native` keeps a minimal coherent form
  (`workspace, backend, server, bridge, registry, keys, service`) — no dangling
  features, and `cargo check -p sapphire-framework --no-default-features` passes.
- `release-plz.toml`: the four `[[package]]` blocks naming the deleted crates removed.

## Findings from the full grep sweep (before deleting)

- `apps/` (sapphire-bridge) has no dependency on any deleted crate.
- `-session` / `-ipc` / `-server` / `-bridge` / `-keys` reference none of the
  deleted crates or types (grep over `crates/ apps/` for
  `sapphire-framework-(rpc|remote-client|remote-server|blob)`,
  `sapphire_(rpc|remote_client|remote_server|blob)`, `RemoteBackend`, `RemoteClient`,
  `WorkspaceSource::Remote`, `search.fts`, `search.semantic` came back empty except
  for the deletion-test strings themselves and doc prose in `docs/`).
- `search.fts` / `search.semantic` existed only inside `-rpc/src/jsonrpc.rs`
  (deleted) and in docs (`docs/ARCHITECTURE.md` — doc prose is Task 5's scope;
  specs and historical plans are immutable history).

## One scope extension (needs review attention)

`WorkspaceEntry::remote()` / `url` / `token` and `WorkspaceSelection::ad_hoc_url`
were used by `crates/sapphire-framework-gui/src/lib.rs` ("Add Remote" dialog:
name + server URL + optional token). Deleting them is implied by "WorkspaceLocator
becomes Path-only, its url form goes" but the gui file was not in the brief's
modify list. I removed the Add-Remote dialog, the "remote" badge, and the
"Files on disk / the remote server are not deleted" wording (now local-only), and
added a regression test in `source.rs` proving an old config carrying `url =
"..."` still deserializes (unknown field ignored) with `locator()` then erroring.
Flagging this as DONE_WITH_CONCERNS input: the gui change is required for the
build, but is a user-visible feature removal the brief did not name explicitly.

## TDD evidence

RED — test written verbatim from the brief, run before any deletion:

```
$ cargo test -p sapphire-framework --test surface
running 2 tests
test the_crates_that_replaced_it_are_present ... ok
test the_http_sync_stack_is_gone ... FAILED

failures:

---- the_http_sync_stack_is_gone stdout ----
thread 'the_http_sync_stack_is_gone' panicked at crates/sapphire-framework/tests/surface.rs:12:9:
sapphire-framework-rpc was replaced by the process architecture; see docs/superpowers/specs/2026-09-16-process-architecture-design.md §6
```

Expected failure: the crates were still listed in the workspace manifest.

GREEN — after deletion:

```
$ cargo test -p sapphire-framework --test surface
running 2 tests
test the_crates_that_replaced_it_are_present ... ok
test the_http_sync_stack_is_gone ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Verification

- `cargo fmt --all -- --check` — clean.
- `cargo clippy --all-targets --all-features -- -D warnings` — clean (fixed one
  `needless_update` in a rewritten test).
- `cargo test --all-features --locked --no-fail-fast` — full-suite run in
  progress at report-writing time; result recorded in the final reply.
- `cargo check -p sapphire-framework --no-default-features` — clean (facade
  still builds with no features).
- `Cargo.lock` committed with the four workspace packages and the now-unreachable
  `quinn` / `quinn-proto` / `quinn-udp` (reqwest HTTP/3) entries removed.

## Self-review notes

- The brief's file list was fully covered; the gui edit is the only file touched
  outside it, forced by the `WorkspaceEntry::remote` deletion and documented above.
- Facade `native`: kept coherent rather than fully to Task 4's final list, since
  the old `native` named removed features. Task 4 will rewrite it and its test
  expects `workspace/backend/server/bridge` — all present.
- `docs/ARCHITECTURE.md` still names the deleted crates and methods; the plan
  assigns the doc rewrite to Task 5, so left untouched.
- `.superpowers/sdd/` is gitignored, per the brief.

## Files changed

Deleted: 4 crates (see top).
Modified: `Cargo.toml`, `Cargo.lock`, `release-plz.toml`,
`crates/sapphire-framework-backend/{Cargo.toml,src/{lib,error,source}.rs}`,
`crates/sapphire-framework/{Cargo.toml,src/lib.rs}`,
`crates/sapphire-framework-gui/src/lib.rs`.
Added: `crates/sapphire-framework/tests/surface.rs` (verbatim from the brief).


## Verification results (final)

- `cargo test --all-features --locked --no-fail-fast` (full workspace, 57 test
  binaries): 678 tests passed, one failure —
  `-p sapphire-framework-server --test converge ::
  a_host_that_was_offline_catches_up_when_it_returns`, the documented pre-existing
  flake ("replica store error: Database already open. Cannot acquire lock." —
  redb lock under parallel load). Re-run standalone: 5/5 passed.
  Also re-run standalone: `-p sapphire-framework-server --test concurrent` (the
  other known-flake target, `reopening_after_eviction_works` / tantivy): 3/3
  passed, output pristine.
- Modified-target confirmation, all 0 failures: `-p sapphire-framework-backend`
  (15 unit + 2 integration), `-p sapphire-framework-gui` (1 unit), 
  `-p sapphire-framework` (2 surface tests).
- `cargo fmt --all -- --check` clean; `cargo clippy --all-targets --all-features
  -- -D warnings` clean.
- Committed `ea5fb2c refactor!: remove the HTTP sync stack` on
  `feat/p2p-sync-iroh`: 32 files changed, +119/−4660; working tree clean.

## Status

DONE_WITH_CONCERNS (single scope extension, see above).
