### Task 2: Delete the HTTP sync stack

**Files:**
- Delete: `crates/sapphire-framework-rpc/`
- Delete: `crates/sapphire-framework-remote-client/`
- Delete: `crates/sapphire-framework-remote-server/`
- Delete: `crates/sapphire-framework-blob/`
- Modify: `Cargo.toml`, `crates/sapphire-framework-backend/{Cargo.toml,src/*}`, facade,
  `release-plz.toml`

**What goes, and why it is safe now:**

| Crate | Replaced by |
|---|---|
| `-rpc` | `-ipc` for local calls, `-session` for replication |
| `-remote-client` | `IpcBackend` for local, the session for peers |
| `-remote-server` | the app server and the bridge |
| `-blob` | content is served from the origin; the session carries it (sync spec §2.3) |

Also gone: `RemoteBackend`, `Error::Remote`, `Error::Conflict`, `WorkspaceLocator`'s URL form,
and the search RPC (`search.fts` / `search.semantic`) — every node has its own index now.

- [ ] **Step 1: Write the test that will fail if any of it comes back**

`crates/sapphire-framework/tests/surface.rs`:

```rust
//! What the framework is made of, stated where a reviewer will see it.

#[test]
fn the_http_sync_stack_is_gone() {
    let manifest = include_str!("../../../Cargo.toml");
    for gone in [
        "sapphire-framework-rpc",
        "sapphire-framework-remote-client",
        "sapphire-framework-remote-server",
        "sapphire-framework-blob",
    ] {
        assert!(
            !manifest.contains(gone),
            "{gone} was replaced by the process architecture; see \
             docs/superpowers/specs/2026-09-16-process-architecture-design.md §6"
        );
    }
}

#[test]
fn the_crates_that_replaced_it_are_present() {
    let manifest = include_str!("../../../Cargo.toml");
    for present in [
        "sapphire-framework-ipc",
        "sapphire-framework-server",
        "sapphire-framework-session",
        "sapphire-framework-bridge",
        "sapphire-framework-keys",
    ] {
        assert!(manifest.contains(present), "{present} is missing from the workspace");
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p sapphire-framework --test surface`
Expected: FAIL — the crates are still listed.

- [ ] **Step 3: Delete**

```bash
git rm -r crates/sapphire-framework-rpc \
          crates/sapphire-framework-remote-client \
          crates/sapphire-framework-remote-server \
          crates/sapphire-framework-blob
```

Then remove their `members` entries, their dependencies from `-backend` (and delete
`-backend/src/remote.rs`, the `Remote` and `Conflict` error variants, and the `RemoteClient`
re-export), and their `release-plz.toml` entries if any name them.

`WorkspaceLocator` becomes `Path`-only, as the sync spec's §1 already recorded. Its `url` form
and `WorkspaceSource::Remote` go with it.

- [ ] **Step 4: Verify and commit**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked
git add -A
git commit -m "refactor!: remove the HTTP sync stack

Replaced by the process architecture: -ipc and -server for local calls,
-session and -bridge for replication. Content is served from the origin, so
-blob has no user left."
```

---

