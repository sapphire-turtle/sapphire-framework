# Task 1 Report: Extract `sapphire-framework-keys`

## Status: DONE

Commit: `98b36a8` — `refactor(keys)!: lift KeyStore out of the remote server crate (#103)` on `feat/p2p-sync-iroh` (single commit, work tree clean after).

## What was implemented

- **New crate `crates/sapphire-framework-keys/`** (`Cargo.toml`, `src/lib.rs`, `src/error.rs`, plus `src/config.rs` — see Deviations):
  - Manifest per the brief's Step 1 snippet, with one required addition: `grain-id.workspace = true` (`KeyEntry.device_id` / `Authenticated.device_id` are `GrainId`s — the brief's snippet omitted it, but the moved code cannot compile without it). `axum` is an optional dependency behind the `axum` feature; dev-deps (`tempfile`, `tower/util`, `tokio`, `http-body-util`, axum with `http1, json, tokio`) exist only for the middleware tests.
  - `git mv crates/sapphire-framework-remote-server/src/{keys.rs,auth.rs}` into the new crate. `src/keys.rs` is content-identical to its pre-move state **plus only the 5 appended extraction tests** (verified: `git diff 93fd631:<old path> HEAD:<new path>` shows 85 insertions, 0 deletions). Renames detected: `keys.rs` at 94% similarity; `auth.rs` is recorded as rewrite (its `crate::ServerState` → `crate::AuthConfig` rewrite touched most lines) — documented below.
  - `auth.rs` rewritten to take `Arc<AuthConfig>` instead of `Arc<ServerState>` (see Deviations), with the crate-level middleware doc comment carried over and the linked-name references updated.
  - `src/config.rs` (new, ~59 lines): `AuthConfig { keys: Option<Arc<KeyStore>>, insecure: bool }` with `new(keys)`, `unconfigured()`, `insecure_for_tests()`, `keys()`, `is_insecure()`.
  - `src/error.rs`: `Error { Io(#[from] std::io::Error), KeyFile(String) }` + `Result` alias, with doc comments; the moved `keys.rs` code paths compile against it unchanged (`crate::error::{Error, Result}`).
- **Remote server `crates/sapphire-framework-remote-server/`**:
  - `lib.rs`: `mod keys`/`mod auth` removed; re-exports `pub use sapphire_keys::{Authenticated, KeyEntry, KeyStore, protect}` so all old import paths keep working; new `ServerState::auth_config() -> sapphire_keys::AuthConfig` bridges the state's key store + insecure flag to the moved layer; `router()` now calls `sapphire_keys::protect(Arc::new(state.auth_config()), routes)`.
  - `Cargo.toml`: + `sapphire-keys = { …, features = ["axum"] }`, − `toml`, − `getrandom` (no longer used by any remaining module — verified by grep).
  - `tests/rpc.rs`: the 4 direct `protect(Arc::clone(&st), …)` calls became `protect(Arc::new(st.auth_config()), …)`; imports unchanged (types still come from the remote-server re-exports). All other tests untouched — 20/20 pass, including the end-to-end bearer-token 401 path and `serve` refusing to start without a usable key.
- **Workspace `Cargo.toml`**: `crates/sapphire-framework-keys` added to `members`; `Cargo.lock` committed.
- **Facade `crates/sapphire-framework/`**:
  - New `keys` feature: `keys = ["dep:sapphire-framework-keys", "sapphire-framework-keys/axum"]` (the prelude exposes `protect`, which needs axum); `remote-server` now chains `keys`, so one optional dependency serves both; module table row + `pub use sapphire_framework_keys as keys` added.
  - Prelude: `Authenticated, AuthConfig, KeyEntry, KeyStore, protect` now come from `crate::keys` (gated on `keys`, which `remote-server` implies); `ServerState, Uuid, WsStore, WsStoreConfig, router, serve` still come from `remote_server`; the `GrainId` fallback re-export moved from `remote_server` to `keys` with the collision comment updated.
- **Remaining users**: verified by grep across `crates/`, `apps/`, and the sibling `sapphire-agent` workspace (the extraction's stated motivation) — every consumer reaches the moved API through `sapphire_framework_remote_server::{KeyStore,…}` or the facade prelude; both keep compiling unchanged thanks to the re-exports. No other crate imports keys/auth internals directly.

## Deviations from the brief (and why)

1. **`AuthConfig` parameter instead of `protect(store: Arc<KeyStore>, router)`**: the brief's sketch would break three behaviours pinned by the moved tests — fail-closed 503 when no key store is configured (axum `Router::layer` wraps even unmatched routes, verified in axum 0.8.9 source), the `insecure_for_tests` escape hatch, and the layer finding the store at all through layer state. A bare `Arc<KeyStore>` carries none of that. `AuthConfig` (59 lines, plain accessors) holds exactly the configuration the layer reads; the remote server adapts its own state through `ServerState::auth_config()`. The 4 protect tests moved out of remote-server's `tests/rpc.rs` into `auth.rs`'s `#[cfg(all(test, feature = "axum"))] mod tests`, adapted to build an `AuthConfig` instead of a `ServerState`, keeping their names and assertions (incl. the fail-closed 503 and "bypass is the only way through" pins).
2. **Brief's interface sketch lists `generate(prefix, label, expires_at)` / `Authenticated { key_id, label }`**: the actual moved code uses `generate(prefix, id, device_id, label, expires_at)` and `Authenticated.device_id` — its 28 tests, the device-binding docs, and the out-of-repo consumer (`sapphire-agent`, whose `device_auth.rs` comment states "the link runs key → device (`KeyEntry.device_id`)") all depend on them. Taken as a summary of the crate, not an API change: the move preserves the real surface, and the brief's "the tests that move with the code" constraint pins exactly that. The 5 new extraction tests were adapted accordingly (their `generate("sjt", …)` calls pass the two `None`s for `id`/`device_id`; the `device_id` round-trip is covered by the 28 moved tests).
3. **`auth.rs` recorded as A/D rather than R** in the final commit: the `ServerState` → `AuthConfig` rewrite touches most of its lines, so git's rename detector records a rewrite (git only shows one rename per pair). `git mv` was still used at move time; `keys.rs` — the payload of the extraction — is detected as `R094`.

## TDD Evidence

- **RED (before the move)**: the 5 `extraction_tests` were appended to remote-server's `keys.rs` first. `cargo test -p sapphire-framework-remote-server --lib extraction_tests`:
  - 4 behavioural tests **passed** (pinned behaviours intact pre-move — the baseline that makes the move meaningful), and
  - `the_crate_does_not_pull_in_axum_by_default` **failed** with `a caller that only wants KeyStore should not link a web framework` — expected, because the manifest the test pins (`axum = { workspace = true, optional = true }`) only exists once the crate exists.
- **GREEN (after the move)**: `cargo test -p sapphire-framework-keys --all-features` → `41 passed` (33 keys tests: 28 moved + 5 extraction; 4 auth/middleware tests moved; plus doctest) + `1 passed` (doctest compile).
- **Key-by-key**: all 28 original `keys::tests` pass in the new crate (constant-time auth, 0600 temp-file writes, failed-save atomicity, rotate/revoke selectors, device_id round-trips, header regeneration).
- **Consumers**: `-p sapphire-framework-remote-server` 39 lib + 20 rpc integration + 2 doctests ok; `-p sapphire-framework-backend` 20 + e2e ok; `-p sapphire-framework-remote-client` roundtrip incl. `bad_token_surfaces_rpc_error` ok; facade builds with `--no-default-features --features keys` and `--features remote-server` separately.

## Full-suite evidence

- `cargo test --workspace --all-features --locked --no-fail-fast`: **66 test targets `test result: ok`, 0 failures** (final run `/tmp/full4.log`). Two transient parallel-load flakes appeared during development and were investigated per the plan header's known-flake note:
  - `sapphire-framework-ipc --test race::eight_simultaneous_clients_produce_one_server` ("the spawn lock" timeout) — failed once under full-suite parallelism; passes standalone on both the pre-change tree (verified via `git stash`) and the final tree.
  - `sapphire-framework-server --test converge::a_host_that_was_offline_catches_up_when_it_returns` (redb "Database already open") — failed once under full-suite parallelism; passes standalone, and both flakes were absent from the final full-suite run.
  - Neither is touched by this task (ipc spawn lock / redb locking paths).
- `cargo fmt --all -- --check` → clean (final tree).
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` → 0 errors, 0 warnings (final tree).
- `cargo tree -p sapphire-framework-keys --no-default-features -e normal | grep -c "axum v"` → **0**: the default build links no axum, the extraction's core goal. With `--features axum`: 1.

## Files changed (commit `98b36a8`)

- `crates/sapphire-framework-keys/{Cargo.toml,src/lib.rs,src/error.rs,src/config.rs}` (new)
- `crates/sapphire-framework-keys/src/keys.rs` ← `crates/sapphire-framework-remote-server/src/keys.rs` (`git mv` + extraction tests; R094)
- `crates/sapphire-framework-keys/src/auth.rs` ← `crates/sapphire-framework-remote-server/src/auth.rs` (`git mv` + `AuthConfig` rewrite)
- `crates/sapphire-framework-remote-server/{Cargo.toml,src/lib.rs,tests/rpc.rs}` (repoint)
- `Cargo.toml` (members), `Cargo.lock`
- `crates/sapphire-framework/{Cargo.toml,src/lib.rs}` (facade)

## Self-review findings (fixed during the work)

- A leftover `debug_tests` module (from live-debugging the 401 mystery) was found in self-review and removed before the final commit.
- `auth.rs` was initially not feature-gated, breaking the default (no-axum) build — fixed with `#![cfg(feature = "axum")]` and a gated `mod auth` + gated re-exports.
- `AuthConfig::unconfigured()` was added after the moved fail-closed test caught a real semantics gap in the first draft (an "empty store" is a *configured* store with zero keys; "no key store" is a distinct state that must refuse with 503).

## Concerns

- None blocking. `sapphire-agent` (sibling repo, pinned to framework 0.13) will switch to the facade `keys` feature in its own follow-up, as its `Cargo.toml` comment already anticipates (`fluo10/sapphire-framework#103 … after which this becomes `keys``).
