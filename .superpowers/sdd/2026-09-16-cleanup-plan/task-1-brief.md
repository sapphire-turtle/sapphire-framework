### Task 1: Extract `sapphire-framework-keys`

**Files:**
- Create: `crates/sapphire-framework-keys/{Cargo.toml,src/lib.rs,src/error.rs}`
- Move: `crates/sapphire-framework-remote-server/src/{keys.rs,auth.rs}` → the new crate
- Modify: `Cargo.toml` (workspace `members`), facade
- Test: the tests that move with the code, plus the new ones below

**Interfaces:**
- Produces:
  - `KeyEntry { token, id, label, created_at, expires_at }`
  - `KeyStore`: `load(path) -> Result<KeyStore>`, `generate(prefix, label, expires_at)`,
    `revoke(selector)`, `entries()`, `authenticate(token) -> Option<&KeyEntry>`
  - `protect(store: Arc<KeyStore>, router: axum::Router) -> axum::Router` (feature `axum`)
  - `Authenticated { key_id: Uuid, label: Option<String> }`

**Why this survives at all:** sync no longer uses HTTP, but the apps' other HTTP endpoints do
— `sapphire-agent`'s `/mcp`, `/acp` and `/a2a`. Those still need a bearer token checked before
the router sees the request. Framework issue **#103** tracks this extraction.

- [ ] **Step 1: Move the code**

```bash
git mv crates/sapphire-framework-remote-server/src/keys.rs crates/sapphire-framework-keys/src/keys.rs
git mv crates/sapphire-framework-remote-server/src/auth.rs crates/sapphire-framework-keys/src/auth.rs
```

Use `git mv` so the history follows the file. Write the manifest with `axum` behind a feature,
so a caller that only wants `KeyStore` does not link a web framework:

```toml
[features]
default = []
axum = ["dep:axum"]

[dependencies]
axum = { workspace = true, optional = true }
base64.workspace = true
chrono = { workspace = true }
getrandom.workspace = true
serde.workspace = true
thiserror.workspace = true
toml.workspace = true
tracing.workspace = true
uuid = { workspace = true }
```

- [ ] **Step 2: Write the tests that pin what must not change in the move**

```rust
#[cfg(test)]
mod extraction_tests {
    use super::*;

    #[test]
    fn a_key_file_written_by_the_old_crate_still_loads() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("keys.toml");
        std::fs::write(
            &path,
            r#"
[[key]]
token = "sjt_abc123"
id = "3f2a4b5c-6d7e-4f80-9112-233445566778"
label = "laptop"
created_at = "2026-08-25T00:00:00Z"
"#,
        )
        .unwrap();

        let store = KeyStore::load(&path).unwrap();
        assert!(store.authenticate("sjt_abc123").is_some());
        assert_eq!(store.entries()[0].label.as_deref(), Some("laptop"));
    }

    #[test]
    fn a_key_with_only_a_token_is_completed_and_written_back() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("keys.toml");
        std::fs::write(&path, "[[key]]\ntoken = \"sjt_xyz\"\n").unwrap();

        let store = KeyStore::load(&path).unwrap();
        assert!(store.entries()[0].created_at.timestamp() > 0);

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("created_at"), "the completion must be written back:\n{text}");
    }

    #[test]
    fn an_expired_key_does_not_authenticate_but_stays_in_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("keys.toml");
        std::fs::write(
            &path,
            "[[key]]\ntoken = \"sjt_old\"\nexpires_at = \"2020-01-01T00:00:00Z\"\n",
        )
        .unwrap();

        let store = KeyStore::load(&path).unwrap();
        assert!(store.authenticate("sjt_old").is_none());
        assert_eq!(
            store.entries().len(),
            1,
            "a key that vanished would leave nobody able to see why they cannot connect"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_generated_key_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("keys.toml");
        let mut store = KeyStore::load(&path).unwrap();
        store.generate("sjt", Some("laptop".into()), None).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
    }

    #[test]
    fn the_crate_does_not_pull_in_axum_by_default() {
        let manifest = include_str!("../Cargo.toml");
        assert!(
            manifest.contains("axum = { workspace = true, optional = true }"),
            "a caller that only wants KeyStore should not link a web framework"
        );
    }
}
```

`a_key_file_written_by_the_old_crate_still_loads` is the point of writing tests for a move at
all: every existing deployment has one of these files, and a silent change to the format would
lock people out of their own servers.

- [ ] **Step 3: Point the remaining users at the new crate, verify, commit**

```bash
cargo test --all-features --locked
git add crates Cargo.toml Cargo.lock
git commit -m "refactor(keys)!: lift KeyStore out of the remote server crate (#103)"
```

---

