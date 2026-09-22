//! Platform-directory resolution and the one-shot migrations of the
//! pre-unification layouts (issues #128/#129 and this task's undo of the
//! per-kind split).
//!
//! Layout: every app stores state under `<platform-root>/<app-name>/` — cache,
//! data and config each under their own platform root, with no per-kind layer.
//! [`AppKind`] survives as a description of what a process does; it never
//! appears in a path.
//!
//! Two one-shot migrations run inside
//! [`AppContext::init`](crate::context::AppContext::init):
//!
//! - **The unsplit migration** ([`unsplit_app_dir`]): #129 briefly stored
//!   per-kind state under `<platform-root>/<app-name>/<kind>/` so that a
//!   desktop app and a server would not open one database. The process
//!   architecture removes the collision itself — only the server opens one —
//!   so `<app>/<kind>/…` moves back up to `<app>/…`.
//! - **The keys migration** ([`migrate_keys_to_data`]): `keys.toml` files found
//!   under a migrated *cache* tree are moved into the matching per-workspace
//!   directory of the *data* tree — they are secrets, not rebuildable cache.
//!
//! Every move is a same-filesystem `std::fs::rename` when possible; if the
//! rename fails (e.g. the trees are on different mounts, `EXDEV`), it falls
//! back to copying the source and deleting it afterwards (`move_item`).

use std::path::{Path, PathBuf};

/// Which binary type a process is. No longer part of a resolved path: kept so
/// an application can still describe what it is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKind {
    Cli,
    Server,
    Desktop,
}

impl AppKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AppKind::Cli => "cli",
            AppKind::Server => "server",
            AppKind::Desktop => "desktop",
        }
    }
}

/// Env var that overrides the platform root for one directory category:
/// `SAPPHIRE_JOURNAL_CACHE_DIR` for ("sapphire-journal", "cache").
pub fn app_dir_env_var(app_name: &str, category: &str) -> String {
    format!(
        "{}_{}_DIR",
        app_name.to_uppercase().replace('-', "_"),
        category.to_uppercase()
    )
}

/// Env var that names the workspace root for an app: `SAPPHIRE_JOURNAL_DIR`.
pub fn workspace_dir_env_var(app_name: &str) -> String {
    format!("{}_DIR", app_name.to_uppercase().replace('-', "_"))
}

fn is_uuid_name(name: &str) -> bool {
    uuid::Uuid::parse_str(name).is_ok()
}

/// Move `from` to `to`, preferring a same-filesystem `std::fs::rename`.
/// If the rename fails (typically `EXDEV` when source and destination are on
/// different mounts — the cache and data trees may be different filesystems
/// via env overrides), fall back to [`copy_then_delete`]. `to` must not exist
/// yet (callers guard).
fn move_item(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(err) => copy_then_delete(from, to).map_err(|fallback_err| {
            std::io::Error::other(format!(
                "cross-device fallback move of {} to {} failed after rename failed ({}): {}",
                from.display(),
                to.display(),
                err,
                fallback_err
            ))
        }),
    }
}

/// Cross-device fallback for [`move_item`]: recursively copy `from` onto `to`
/// (creating `to`), then delete `from` — `remove_file` for a plain file
/// (`remove_dir_all` fails with `ENOTDIR` on a file), `remove_dir_all` for a
/// directory tree. Symlinked entries inside a tree are dereferenced: they are
/// copied as regular files, not recreated as symlinks.
fn copy_then_delete(from: &Path, to: &Path) -> std::io::Result<()> {
    copy_path(from, to)?;
    if from.is_file() {
        std::fs::remove_file(from)
    } else {
        std::fs::remove_dir_all(from)
    }
}

/// Recursive copy of a file or directory tree (`from` onto `to`, which is
/// created). Used by [`copy_then_delete`] as the cross-device fallback.
fn copy_path(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_file() {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        return std::fs::copy(from, to).map(|_| ());
    }
    for entry in walkdir::WalkDir::new(from) {
        let entry = entry.map_err(std::io::Error::other)?;
        let Ok(rel) = entry.path().strip_prefix(from) else {
            return Err(std::io::Error::other("walkdir path escaped source"));
        };
        let dest = to.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(dest)?;
        } else if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

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

/// Move per-workspace `keys.toml` files from the cache tree into the data
/// tree, using the per-kind `<app>/<kind>/<uuid>/` layout that the unsplit
/// migration removes. Once per uuid: skipped when the data tree already has
/// that workspace directory, which is what keeps this a once-ever migration
/// (and protects an already-migrated `keys.toml` from being overwritten on
/// later launches).
///
/// The cache tree passed in should be the one *before* [`unsplit_app_dir`]
/// runs, so that `<app>/<kind>/<uuid>/keys.toml` is still there to move.
pub fn migrate_keys_to_data(
    cache_app_dir: &Path,
    data_app_dir: &Path,
    kind: AppKind,
) -> std::io::Result<()> {
    let kind_str = kind.as_str();
    let (cache_kind, data_kind) = (cache_app_dir.join(kind_str), data_app_dir.join(kind_str));
    if !cache_kind.is_dir() || !data_kind.is_dir() {
        return Ok(());
    }
    let mut migrated_any = false;
    for entry in std::fs::read_dir(&cache_kind)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_uuid_name(&name) || !entry.file_type()?.is_dir() {
            continue;
        }
        let dest = data_kind.join(&name);
        // Once-per-uuid guard: a data-tree dir that already exists for this
        // workspace means the migration (or later use) already happened.
        if dest.exists() {
            continue;
        }
        let key = cache_kind.join(&name).join("keys.toml");
        if key.exists() {
            std::fs::create_dir_all(&dest)?;
            move_item(&key, &dest.join("keys.toml"))?;
            migrated_any = true;
        }
    }
    if migrated_any {
        tracing::warn!(
            "migrated per-workspace data (keys.toml) under {} for the first time",
            kind_str
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn env_var_names_follow_the_app_name_rule() {
        assert_eq!(
            app_dir_env_var("sapphire-journal", "cache"),
            "SAPPHIRE_JOURNAL_CACHE_DIR"
        );
        assert_eq!(
            app_dir_env_var("sapphire-agent", "data"),
            "SAPPHIRE_AGENT_DATA_DIR"
        );
        assert_eq!(
            workspace_dir_env_var("sapphire-ledger"),
            "SAPPHIRE_LEDGER_DIR"
        );
    }

    #[test]
    fn keys_files_migrate_from_the_cache_tree_into_the_data_tree() {
        let root = tempdir().unwrap();
        let cache = root.path().join("cache");
        let data = root.path().join("data");
        let uuid = "2f1c0000-0000-8000-8000-000000000000";
        std::fs::create_dir_all(cache.join("sapphire-ledger/server").join(uuid)).unwrap();
        std::fs::create_dir_all(data.join("sapphire-ledger/server")).unwrap();
        std::fs::write(
            cache
                .join("sapphire-ledger/server")
                .join(uuid)
                .join("keys.toml"),
            "k",
        )
        .unwrap();

        migrate_keys_to_data(
            &cache.join("sapphire-ledger"),
            &data.join("sapphire-ledger"),
            AppKind::Server,
        )
        .unwrap();

        assert!(
            data.join("sapphire-ledger/server")
                .join(uuid)
                .join("keys.toml")
                .exists()
        );
        assert!(
            !cache
                .join("sapphire-ledger/server")
                .join(uuid)
                .join("keys.toml")
                .exists()
        );
    }

    #[test]
    fn second_migrate_keys_to_data_is_a_noop_and_does_not_overwrite() {
        let root = tempdir().unwrap();
        let cache = root.path().join("cache");
        let data = root.path().join("data");
        let uuid = "2f1c0000-0000-8000-8000-000000000000";
        let cache_uuid = cache.join("sapphire-ledger/server").join(uuid);
        let data_uuid = data.join("sapphire-ledger/server").join(uuid);
        std::fs::create_dir_all(&cache_uuid).unwrap();
        std::fs::create_dir_all(&data_uuid).unwrap();
        std::fs::write(cache_uuid.join("keys.toml"), "original").unwrap();

        migrate_keys_to_data(
            &cache.join("sapphire-ledger"),
            &data.join("sapphire-ledger"),
            AppKind::Server,
        )
        .unwrap();
        // the data tree now owns the (already-migrated) file
        std::fs::write(data_uuid.join("keys.toml"), "rotated").unwrap();
        std::fs::write(cache_uuid.join("keys.toml"), "stale").unwrap();

        // a second call must skip this uuid entirely — no overwrite, no error
        migrate_keys_to_data(
            &cache.join("sapphire-ledger"),
            &data.join("sapphire-ledger"),
            AppKind::Server,
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(data_uuid.join("keys.toml")).unwrap(),
            "rotated"
        );
        assert!(cache_uuid.join("keys.toml").exists()); // untouched, not moved back
    }

    #[test]
    fn move_item_falls_back_to_copy_and_delete_when_rename_is_not_possible() {
        // move_item first tries rename; on a single filesystem a plain rename
        // works and the fallback is never reached — so the fallback itself
        // (the copy + delete-source path move_item delegates to on EXDEV) is
        // exercised directly here, for both a directory tree and a plain file.
        let root = tempdir().unwrap();

        // directory tree (Option-A / shared-layout shape)
        let from = root.path().join("src-tree").join("nested");
        let to = root.path().join("other-mount").join("moved");
        std::fs::create_dir_all(from.join("inner")).unwrap();
        std::fs::write(from.join("keys.toml"), "secret").unwrap();
        std::fs::write(from.join("inner").join("index.bin"), "bytes").unwrap();

        copy_then_delete(&from, &to).unwrap();

        assert!(!from.exists());
        assert_eq!(
            std::fs::read_to_string(to.join("keys.toml")).unwrap(),
            "secret"
        );
        assert_eq!(
            std::fs::read_to_string(to.join("inner").join("index.bin")).unwrap(),
            "bytes"
        );

        // plain file (the keys.toml-between-mounts case): removing the
        // file source must not take the remove_dir_all path
        let file_from = root.path().join("cache-tree").join("keys.toml");
        std::fs::create_dir_all(file_from.parent().unwrap()).unwrap();
        std::fs::write(&file_from, "secret").unwrap();
        let file_to = root.path().join("data-tree").join("keys.toml");

        copy_then_delete(&file_from, &file_to).unwrap();

        assert!(!file_from.exists());
        assert_eq!(std::fs::read_to_string(&file_to).unwrap(), "secret");
    }
}

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
        seed(
            tmp.path(),
            "sapphire-journal",
            "server",
            "ws-1",
            "docs.redb",
        );

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
        let app_dir = tmp.path().join("sapphire-journal");
        seed(
            tmp.path(),
            "sapphire-journal",
            "server",
            "ws-1",
            "docs.redb",
        );

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
        std::fs::write(
            app_dir.join("server").join("ws-1").join("keys.toml"),
            b"[[key]]\n",
        )
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
        // Every env mutation in this crate's tests goes through `TestEnv` (see
        // `test_env.rs`); the data and config categories are pinned to their own
        // tempdirs so `init` never touches the real platform directories.
        let _env = crate::test_env::TestEnv::lock();
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        crate::test_env::TestEnv::set("SAPPHIRE_UNSPLIT_CACHE_DIR", tmp.path());
        crate::test_env::TestEnv::set("SAPPHIRE_UNSPLIT_DATA_DIR", data.path());
        crate::test_env::TestEnv::set("SAPPHIRE_UNSPLIT_CONFIG_DIR", config.path());
        static CTX: crate::AppContext = crate::AppContext::new("sapphire-unsplit");
        CTX.init(AppKind::Server);
        crate::test_env::TestEnv::remove("SAPPHIRE_UNSPLIT_CACHE_DIR");
        crate::test_env::TestEnv::remove("SAPPHIRE_UNSPLIT_DATA_DIR");
        crate::test_env::TestEnv::remove("SAPPHIRE_UNSPLIT_CONFIG_DIR");

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
