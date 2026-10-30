//! Moving a directory from Roaming to Local on Windows (#365).
//!
//! corundum-framework-platform 0.4 keeps settings and data in `%LOCALAPPDATA%` on Windows,
//! where the cache already was, instead of `%APPDATA%` (Roaming), and moves an app's
//! directory over with [`AppDirs::migrate_from_roaming`](corundum_framework_platform::dirs::AppDirs::migrate_from_roaming)
//! (corundum-framework#57). [`migrate`] is that move as the sapphire processes need it:
//!
//! - **The old cache steps aside first.** The cache used to be `%LOCALAPPDATA%\<app>`
//!   itself, and its per-workspace directories have the same names as the data's in
//!   Roaming. corundum merges entry by entry and leaves in Roaming whatever Local has
//!   already, so the data's workspace directories would stay behind. With `old_cache`
//!   set, a Local directory found beside a Roaming one is the old cache: it is moved into
//!   `<app>\<old_cache>`, where the cache now lives, before the merge.
//! - **What the processes recreate is dropped from Roaming.** A `disposable` entry Local
//!   has already (the bridge's `run`, which any client makes) is removed from Roaming
//!   rather than left there.
//! - **One process moves; the others wait.** The app server, the bridge, the window and
//!   the CLI all start it, often together. They hold `%LOCALAPPDATA%\<name>.sapphire-migrate.lock`
//!   while moving, so a second start waits for the first and then finds nothing to move,
//!   instead of corundum's refusal.
//!
//! Outside Windows there is no Roaming, and nothing happens.

use std::io;
use std::path::Path;

/// Move `name`'s directory from Roaming to Local, once per process: see the
/// [module](self). On an error the caller must not read or write the directory in this
/// run, as the move may have stopped halfway; the next start goes on with it.
pub fn migrate(name: &str, old_cache: Option<&str>, disposable: &[&str]) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::collections::BTreeSet;
        use std::sync::Mutex;

        // The names moved in this process: the runtime directory is asked for often.
        static DONE: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
        let mut done = DONE.lock().unwrap_or_else(|e| e.into_inner());
        if done.contains(name) {
            return Ok(());
        }
        let roaming_root = std::env::var_os("APPDATA").filter(|v| !v.is_empty());
        let local_root = corundum_framework_platform::dirs::data_dir();
        if let (Some(roaming_root), Some(local_root)) = (roaming_root, local_root) {
            migrate_with(
                Path::new(&roaming_root),
                &local_root,
                name,
                old_cache,
                disposable,
                || corundum_framework_platform::dirs::AppDirs::migrate_from_roaming(name),
            )?;
        }
        done.insert(name.to_owned());
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (name, old_cache, disposable);
        Ok(())
    }
}

/// [`migrate`] for the bridge's directory, which every process reaches: the bridge keeps
/// its state there, and the clients find the runtime directory in it. Its `run` is made
/// again by whichever process starts first, so it is not kept from Roaming.
pub fn migrate_bridge() -> io::Result<()> {
    migrate(crate::BRIDGE_DIR_NAME, None, &["run"])
}

/// [`migrate`] between the roots given, with `merge` as corundum's move, so it can be
/// tried on any OS.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn migrate_with<T: std::fmt::Debug>(
    roaming_root: &Path,
    local_root: &Path,
    name: &str,
    old_cache: Option<&str>,
    disposable: &[&str],
    merge: impl FnOnce() -> io::Result<T>,
) -> io::Result<()> {
    let roaming = roaming_root.join(name);
    let local = local_root.join(name);
    // The old cache, between the two renames that put it into the new place.
    let parked = local_root.join(format!("{name}.sapphire-old-cache"));
    // Most starts: nothing to move, and no lock file made for it.
    if !is_there(&roaming)? && !is_there(&parked)? {
        return Ok(());
    }

    std::fs::create_dir_all(local_root)?;
    let lock = std::fs::File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(local_root.join(format!("{name}.sapphire-migrate.lock")))?;
    // Blocks while another start moves; it is released when `lock` is dropped.
    lock.lock()?;

    if let Some(cache) = old_cache {
        // A Local directory without the new cache folder, beside a Roaming one, is the
        // old cache: no merge has begun, as one only begins after this.
        if !is_there(&parked)?
            && is_there(&roaming)?
            && local.is_dir()
            && !is_there(&local.join(cache))?
        {
            std::fs::rename(&local, &parked)?;
            tracing::info!(
                "moving the old cache {} into {}",
                local.display(),
                local.join(cache).display()
            );
        }
        if is_there(&parked)? {
            std::fs::create_dir_all(&local)?;
            let dest = local.join(cache);
            if is_there(&dest)? {
                tracing::warn!(
                    "{} is left where it is: {} exists already",
                    parked.display(),
                    dest.display()
                );
            } else {
                std::fs::rename(&parked, &dest)?;
            }
        }
    }

    if is_there(&roaming)? && local.is_dir() {
        for entry in disposable {
            let old = roaming.join(entry);
            if !is_there(&old)? || !is_there(&local.join(entry))? {
                continue;
            }
            let removed = if old.is_dir() {
                std::fs::remove_dir_all(&old)
            } else {
                std::fs::remove_file(&old)
            };
            // Best effort: what stays is left in Roaming by the merge, and reported.
            if let Err(err) = removed {
                tracing::warn!("could not remove {}: {err}", old.display());
            }
        }
    }

    if is_there(&roaming)? {
        let moved = merge()?;
        tracing::info!(
            "moved {} to {}: {moved:?}",
            roaming.display(),
            local.display()
        );
    }
    Ok(())
}

/// Whether anything (a file, a directory or a link) is at `path`.
fn is_there(path: &Path) -> io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    struct Roots {
        _tmp: tempfile::TempDir,
        roaming: PathBuf,
        local: PathBuf,
    }

    fn roots() -> Roots {
        let tmp = tempfile::tempdir().unwrap();
        let roaming = tmp.path().join("Roaming");
        let local = tmp.path().join("Local");
        std::fs::create_dir_all(&roaming).unwrap();
        std::fs::create_dir_all(&local).unwrap();
        Roots {
            _tmp: tmp,
            roaming,
            local,
        }
    }

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    /// The top-level entries of `dir`, sorted.
    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn without_a_roaming_directory_nothing_happens() {
        let r = roots();
        touch(&r.local.join("app/ws/index"));
        let merged = RefCell::new(false);
        migrate_with(&r.roaming, &r.local, "app", Some("cache"), &[], || {
            *merged.borrow_mut() = true;
            Ok(())
        })
        .unwrap();
        assert!(!*merged.borrow());
        // The old cache is not touched, and no lock file is made.
        assert_eq!(entries(&r.local), ["app"]);
        assert_eq!(entries(&r.local.join("app")), ["ws"]);
    }

    #[test]
    fn the_old_cache_steps_aside_before_the_merge() {
        let r = roots();
        touch(&r.local.join("app/ws/index"));
        touch(&r.roaming.join("app/ws/keys.toml"));
        let local_app = r.local.join("app");
        migrate_with(&r.roaming, &r.local, "app", Some("cache"), &[], || {
            // What corundum finds: the cache in its new place, nothing else to clash with.
            assert_eq!(entries(&local_app), ["cache"]);
            Ok(())
        })
        .unwrap();
        assert!(r.local.join("app/cache/ws/index").is_file());
        assert!(!r.local.join("app.sapphire-old-cache").exists());
    }

    #[test]
    fn a_parked_cache_is_put_in_place_on_the_next_start() {
        let r = roots();
        // A start stopped between the two renames.
        touch(&r.local.join("app.sapphire-old-cache/ws/index"));
        touch(&r.roaming.join("app/ws/keys.toml"));
        migrate_with(&r.roaming, &r.local, "app", Some("cache"), &[], || Ok(())).unwrap();
        assert!(r.local.join("app/cache/ws/index").is_file());
        assert!(!r.local.join("app.sapphire-old-cache").exists());
    }

    #[test]
    fn a_merge_under_way_is_not_taken_for_the_old_cache() {
        let r = roots();
        // An earlier start moved the cache and part of Roaming, then stopped.
        touch(&r.local.join("app/cache/ws/index"));
        touch(&r.local.join("app/settings.toml"));
        touch(&r.roaming.join("app/ws/keys.toml"));
        let merged = RefCell::new(false);
        migrate_with(&r.roaming, &r.local, "app", Some("cache"), &[], || {
            *merged.borrow_mut() = true;
            Ok(())
        })
        .unwrap();
        assert!(*merged.borrow());
        assert_eq!(entries(&r.local.join("app")), ["cache", "settings.toml"]);
        assert!(r.local.join("app/cache/ws/index").is_file());
    }

    #[test]
    fn without_old_cache_the_local_directory_stays() {
        let r = roots();
        touch(&r.local.join("sapphire-bridge/models/m.onnx"));
        touch(&r.roaming.join("sapphire-bridge/device.toml"));
        migrate_with(
            &r.roaming,
            &r.local,
            "sapphire-bridge",
            None,
            &[],
            || Ok(()),
        )
        .unwrap();
        assert_eq!(entries(&r.local.join("sapphire-bridge")), ["models"]);
    }

    #[test]
    fn a_disposable_entry_local_has_is_dropped_from_roaming() {
        let r = roots();
        touch(&r.local.join("sapphire-bridge/run/app.lock"));
        touch(&r.roaming.join("sapphire-bridge/run/app.lock"));
        touch(&r.roaming.join("sapphire-bridge/state/x"));
        // `state` is not disposable, and Local has no `state`: it is the merge's to move.
        touch(&r.local.join("sapphire-bridge/models/m.onnx"));
        let bridge = r.roaming.join("sapphire-bridge");
        migrate_with(
            &r.roaming,
            &r.local,
            "sapphire-bridge",
            None,
            &["run", "state"],
            || {
                assert_eq!(entries(&bridge), ["state"]);
                Ok(())
            },
        )
        .unwrap();
    }

    #[test]
    fn a_failed_merge_is_an_error() {
        let r = roots();
        touch(&r.roaming.join("app/settings.toml"));
        let err = migrate_with(&r.roaming, &r.local, "app", Some("cache"), &[], || {
            Err::<(), _>(io::Error::new(io::ErrorKind::WouldBlock, "in use"))
        })
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    }
}
