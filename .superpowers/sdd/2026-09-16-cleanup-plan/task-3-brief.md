### Task 3: Undo the per-kind directory split

**Files:**
- Modify: `crates/sapphire-framework-workspace/src/{app_dirs.rs,context.rs}`
- Test: inline `#[cfg(test)] mod tests` in `app_dirs.rs`

**What changes (spec §7):**

- Cache returns to `<platform cache root>/<app>/<uuid>/`; data and config to
  `<platform data root>/<app>/` and `<platform config root>/<app>/`.
- `AppKind` survives as a description of what a process does. **It no longer appears in a path.**
- The environment overrides keep their #129 names and still replace only the platform root.
- A **second one-shot migration** moves `<app>/<kind>/…` back up to `<app>/…`.

**Why the split can go:** it existed so that a desktop app and a server would not open one
database. The server is now the only process that opens one, so the split has no remaining
purpose — and it never solved the case that mattered, since a CLI invocation and the stdio MCP
server were both `cli`.

**The migration deletes nothing.** Where two kinds both left a cache for one workspace, the
server's copy wins and the others are left where they are, with a warning naming them. A cache
is rebuildable; a directory silently removed is not.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod unsplit_tests {
    use super::*;

    /// Build `<root>/<app>/<kind>/<uuid>/` with a file in it.
    fn seed(root: &std::path::Path, app: &str, kind: &str, uuid: &str, file: &str) {
        let dir = root.join(app).join(kind).join(uuid);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(file), b"x").unwrap();
    }

    #[test]
    fn a_per_kind_directory_moves_up_one_level() {
        let tmp = tempfile::tempdir().unwrap();
        seed(tmp.path(), "sapphire-journal", "server", "ws-1", "docs.redb");

        let app_dir = unsplit_app_dir(&tmp.path().join("sapphire-journal")).unwrap();

        assert_eq!(app_dir, tmp.path().join("sapphire-journal"));
        assert!(app_dir.join("ws-1").join("docs.redb").exists());
        assert!(
            !app_dir.join("server").join("ws-1").exists(),
            "the moved directory must not be left behind as well"
        );
    }

    #[test]
    fn migrating_twice_changes_nothing_the_second_time() {
        let tmp = tempfile::tempdir().unwrap();
        seed(tmp.path(), "sapphire-journal", "server", "ws-1", "docs.redb");
        let app_dir = tmp.path().join("sapphire-journal");

        unsplit_app_dir(&app_dir).unwrap();
        unsplit_app_dir(&app_dir).unwrap();

        assert!(app_dir.join("ws-1").join("docs.redb").exists());
    }

    #[test]
    fn the_server_copy_wins_when_two_kinds_left_one() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("sapphire-journal");
        std::fs::create_dir_all(app_dir.join("cli").join("ws-1")).unwrap();
        std::fs::write(app_dir.join("cli").join("ws-1").join("mark"), b"cli").unwrap();
        std::fs::create_dir_all(app_dir.join("server").join("ws-1")).unwrap();
        std::fs::write(app_dir.join("server").join("ws-1").join("mark"), b"server").unwrap();

        unsplit_app_dir(&app_dir).unwrap();

        assert_eq!(
            std::fs::read_to_string(app_dir.join("ws-1").join("mark")).unwrap(),
            "server"
        );
    }

    #[test]
    fn a_losing_copy_is_left_in_place_rather_than_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("sapphire-journal");
        std::fs::create_dir_all(app_dir.join("cli").join("ws-1")).unwrap();
        std::fs::write(app_dir.join("cli").join("ws-1").join("mark"), b"cli").unwrap();
        std::fs::create_dir_all(app_dir.join("server").join("ws-1")).unwrap();
        std::fs::write(app_dir.join("server").join("ws-1").join("mark"), b"server").unwrap();

        unsplit_app_dir(&app_dir).unwrap();

        assert!(
            app_dir.join("cli").join("ws-1").join("mark").exists(),
            "a directory this migration could not move must be left for the user to look at"
        );
    }

    #[test]
    fn keys_move_with_their_workspace() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("sapphire-agent");
        std::fs::create_dir_all(app_dir.join("server").join("ws-1")).unwrap();
        std::fs::write(app_dir.join("server").join("ws-1").join("keys.toml"), b"[[key]]\n")
            .unwrap();

        unsplit_app_dir(&app_dir).unwrap();

        assert!(
            app_dir.join("ws-1").join("keys.toml").exists(),
            "a server that lost its keys refuses to start"
        );
    }

    #[test]
    fn an_app_directory_that_was_never_split_is_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("sapphire-journal");
        std::fs::create_dir_all(app_dir.join("ws-1")).unwrap();
        std::fs::write(app_dir.join("ws-1").join("docs.redb"), b"x").unwrap();

        unsplit_app_dir(&app_dir).unwrap();

        assert!(app_dir.join("ws-1").join("docs.redb").exists());
    }

    #[test]
    fn a_missing_app_directory_is_created_not_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("brand-new");
        assert_eq!(unsplit_app_dir(&app_dir).unwrap(), app_dir);
        assert!(app_dir.is_dir());
    }

    #[test]
    fn a_directory_that_is_not_a_kind_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let app_dir = tmp.path().join("sapphire-journal");
        std::fs::create_dir_all(app_dir.join("ws-1")).unwrap();
        // A workspace uuid could look like anything; only the three known kinds move.
        std::fs::create_dir_all(app_dir.join("desktop-notes")).unwrap();

        unsplit_app_dir(&app_dir).unwrap();

        assert!(app_dir.join("desktop-notes").is_dir());
    }

    #[test]
    fn the_resolved_cache_path_no_longer_contains_a_kind() {
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: the test binary sets this before any other thread reads it.
        unsafe { std::env::set_var("SAPPHIRE_UNSPLIT_CACHE_DIR", tmp.path()) };
        static CTX: crate::AppContext = crate::AppContext::new("sapphire-unsplit");
        CTX.init(AppKind::Server);
        unsafe { std::env::remove_var("SAPPHIRE_UNSPLIT_CACHE_DIR") };

        let cache = CTX.cache_dir();
        for kind in ["/cli", "/server", "/desktop"] {
            assert!(
                !cache.to_string_lossy().contains(kind),
                "the kind is still in the path: {}",
                cache.display()
            );
        }
    }
}
```

`a_directory_that_is_not_a_kind_is_left_alone` matters because the migration walks a directory
whose other entries are workspace uuids. Moving anything that is not one of the three known
kind names would scatter a user's caches.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sapphire-framework-workspace --all-features unsplit`
Expected: FAIL — `unsplit_app_dir` does not exist.

- [ ] **Step 3: Implement**

Replace `migrate_app_dir` with `unsplit_app_dir`:

```rust
/// Move `<app>/<kind>/…` back up to `<app>/…` (spec §7), and return the app directory.
///
/// #129 split these per binary kind so a desktop app and a server would not open one
/// database. The server is now the only process that opens one, so the split has no
/// remaining purpose — and it never covered the case that mattered, since a CLI invocation
/// and the stdio MCP server were both `cli`.
///
/// Idempotent, and it deletes nothing: where two kinds left a directory for one workspace,
/// the server's wins and the others stay where they are, named in a warning.
pub fn unsplit_app_dir(app_dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(app_dir)?;

    // Most specific first: the server's copy is the one the new architecture keeps writing.
    for kind in [AppKind::Server, AppKind::Desktop, AppKind::Cli] {
        let from = app_dir.join(kind.as_str());
        if !from.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&from)? {
            let entry = entry?;
            let target = app_dir.join(entry.file_name());
            if target.exists() {
                tracing::warn!(
                    kept = %target.display(),
                    left = %entry.path().display(),
                    "two kinds left a directory for one workspace; the first kept wins and \
                     the other is left in place — delete it once you are satisfied"
                );
                continue;
            }
            if let Err(err) = std::fs::rename(entry.path(), &target) {
                // A cross-device rename fails; fall back to a copy, and still delete nothing
                // on failure.
                tracing::warn!(
                    from = %entry.path().display(),
                    to = %target.display(),
                    "could not move: {err}"
                );
            }
        }
        // Only if it emptied out.
        let _ = std::fs::remove_dir(&from);
    }
    Ok(app_dir.to_owned())
}
```

`context.rs`'s `init_category` calls `unsplit_app_dir` instead of `migrate_app_dir`, and
`init` no longer passes the kind down to path resolution. `migrate_keys_to_data` stays as it
is — it moves keys between the cache and data trees, which is orthogonal.

- [ ] **Step 4: Verify and commit**

```bash
cargo test --all-features --locked
git add crates/sapphire-framework-workspace
git commit -m "refactor(workspace)!: undo the per-kind directory split

#129 split cache, data and config per binary kind so two kinds would not open
one database. The process architecture removes the collision itself, and the
split never covered the case that caused it: a CLI invocation and the stdio MCP
server are both \`cli\`. The migration moves directories back up and deletes
nothing."
```

---

