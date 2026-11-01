//! Keys stored before corundum 0.7 named no profile, and corundum 0.7 skips such a file,
//! refusing its token. [`fill_missing_profiles`] gives each the [`MIGRATED`] profile, once.

use std::path::Path;

use crate::profiles::MIGRATED;

/// Give every key file in `dir` that names no profile (or an empty one) the [`MIGRATED`]
/// profile, dropping corundum 0.6's `subject`. The number of files rewritten.
///
/// Idempotent, so every host of a synced workspace may run it: all write the same
/// profile. A missing `dir` is no keys. A file that is not a key's TOML is left as it is,
/// for corundum to warn about.
pub fn fill_missing_profiles(dir: &Path) -> std::io::Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let mut rewritten = 0;
    for entry in entries {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Hidden files are a write in progress, or not a key's.
        if name.starts_with('.') || !name.ends_with(".toml") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let Ok(mut doc) = text.parse::<toml_edit::DocumentMut>() else {
            continue;
        };
        let named = doc
            .get("profile")
            .and_then(|p| p.as_str())
            .is_some_and(|p| !p.is_empty());
        if named || doc.get("id").is_none() {
            continue;
        }
        doc.remove("subject");
        doc["profile"] = toml_edit::value(MIGRATED);
        let tmp = path.with_file_name(format!(".{name}.profile.tmp"));
        std::fs::write(&tmp, doc.to_string())?;
        std::fs::rename(&tmp, &path)?;
        rewritten += 1;
    }
    Ok(rewritten)
}

#[cfg(test)]
mod tests {
    use super::*;
    use corundum_framework_keys::{KeyStore, ProfileId};

    // A key as corundum 0.6 wrote it: `subject`, no `profile`.
    const OLD: &str = r#"id = "k1"
name = "pendant"
subject = "x"
hash = "blake3:0000000000000000000000000000000000000000000000000000000000000000"
created_at = "2026-10-31T00:00:00Z"
"#;

    #[test]
    fn a_key_without_a_profile_is_read_only_after() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("k1.toml"), OLD).unwrap();
        assert!(KeyStore::open(dir.path()).unwrap().list().is_empty());
        assert_eq!(fill_missing_profiles(dir.path()).unwrap(), 1);
        let store = KeyStore::open(dir.path()).unwrap();
        let [key] = store.list() else {
            panic!("one key: {:?}", store.list());
        };
        assert_eq!(key.profile, ProfileId::new(MIGRATED).unwrap());
        assert_eq!(key.name, "pendant");
    }

    #[test]
    fn a_key_with_a_profile_and_other_files_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = KeyStore::open(dir.path()).unwrap();
        store
            .add("p", None, ProfileId::new("admin").unwrap())
            .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        std::fs::write(dir.path().join("broken.toml"), "not = [toml").unwrap();
        assert_eq!(fill_missing_profiles(dir.path()).unwrap(), 0);
        assert_eq!(fill_missing_profiles(dir.path()).unwrap(), 0);
        let store = KeyStore::open(dir.path()).unwrap();
        assert_eq!(store.list()[0].profile.as_str(), "admin");
    }

    #[test]
    fn it_runs_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("k1.toml"), OLD).unwrap();
        assert_eq!(fill_missing_profiles(dir.path()).unwrap(), 1);
        assert_eq!(fill_missing_profiles(dir.path()).unwrap(), 0);
    }

    #[test]
    fn no_directory_is_no_keys() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(fill_missing_profiles(&dir.path().join("none")).unwrap(), 0);
    }
}
