//! The one-time import of the workgroup's external devices (#199) into the workspace's API
//! keys (#364).
//!
//! The ledger is read straight from the bridge's directory, read-only, so the bridge need
//! not run. Every host imports the same records under the same ids, so hosts that import at
//! once agree once the workspace syncs.

#![allow(deprecated)]

use std::path::Path;

use corundum_framework_keys::{Error as KeyError, Key, KeyHash, KeyId};
use sapphire_framework_keys::open_store;
use sapphire_registry::ExternalDevices;

/// The marker in the application's data directory that says this host has imported.
pub const IMPORTED_MARKER: &str = "api_keys_imported";

/// What an import did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Records copied into the workspace.
    pub imported: usize,
    /// Records the workspace already had (another host imported them).
    pub skipped: usize,
}

/// Copy the unretired external devices that may use `app_name`, from every workgroup in
/// `bridge_dir`, into the API keys of the workspace at `root`, keeping their ids and hashes;
/// then write [`IMPORTED_MARKER`] into `data_dir`. Does nothing when the marker is there.
///
/// A missing bridge directory is nothing to import yet, and writes no marker: a bridge not
/// run since its directory moved has it later. An error leaves no marker either, so the next
/// start tries again.
pub fn import_external_devices(
    app_name: &str,
    data_dir: &Path,
    bridge_dir: &Path,
    root: &Path,
) -> Result<ImportReport, String> {
    let marker = data_dir.join(IMPORTED_MARKER);
    if marker.exists() {
        return Ok(ImportReport::default());
    }
    let mut report = ImportReport::default();
    let workgroups = bridge_dir.join("workgroups");
    if !bridge_dir.exists() {
        // No bridge here, or one not run since its directory moved: look again next time.
        return Ok(report);
    }
    let entries = match std::fs::read_dir(&workgroups) {
        Ok(entries) => entries.collect::<Result<Vec<_>, _>>(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(err),
    }
    .map_err(|e| format!("{}: {e}", workgroups.display()))?;
    let mut store = None;
    for workgroup in entries {
        let ledger_dir = workgroup.path().join("root").join("external_devices");
        let ledger = ExternalDevices::open(&ledger_dir)
            .map_err(|e| format!("{}: {e}", ledger_dir.display()))?;
        for device in ledger
            .entries()
            .iter()
            .filter(|d| !d.is_retired() && d.allows(app_name))
        {
            let store = match &mut store {
                Some(store) => store,
                None => store.insert(open_store(app_name, root).map_err(|e| e.to_string())?),
            };
            let key = Key {
                id: KeyId::new(&device.id.to_string()).map_err(|e| e.to_string())?,
                name: device.name.clone(),
                description: device.description.clone(),
                subject: None,
                hash: format!("sha256:{}", device.token_sha256)
                    .parse::<KeyHash>()
                    .map_err(|e| e.to_string())?,
                created_at: device.created_at,
                rotated_at: device.rotated_at,
                retired_at: None,
            };
            match store.import(key) {
                Ok(_) => report.imported += 1,
                Err(KeyError::IdTaken(_)) => report.skipped += 1,
                Err(err) => return Err(err.to_string()),
            }
        }
    }
    std::fs::create_dir_all(data_dir)
        .and_then(|()| std::fs::write(&marker, ""))
        .map_err(|e| format!("{}: {e}", marker.display()))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sapphire_registry::GrainId;

    const APP: &str = "sapphire-importtest";

    struct Fixture {
        _tmp: tempfile::TempDir,
        data: std::path::PathBuf,
        bridge: std::path::PathBuf,
        root: std::path::PathBuf,
        ledger: ExternalDevices,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let bridge = tmp.path().join("bridge");
        let root = tmp.path().join("ws");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let wg = GrainId::random();
        let ledger = ExternalDevices::open(
            &bridge
                .join("workgroups")
                .join(wg.to_string())
                .join("root")
                .join("external_devices"),
        )
        .unwrap();
        Fixture {
            _tmp: tmp,
            data,
            bridge,
            root,
            ledger,
        }
    }

    fn run(f: &Fixture) -> Result<ImportReport, String> {
        import_external_devices(APP, &f.data, &f.bridge, &f.root)
    }

    #[test]
    fn imports_unretired_records_for_this_app() {
        let mut f = fixture();
        let (mine, token) = f.ledger.add("pendant", None, vec![APP.into()]).unwrap();
        f.ledger
            .add("other", None, vec!["sapphire-other".into()])
            .unwrap();
        f.ledger.add("gone", None, vec![APP.into()]).unwrap();
        f.ledger.retire("gone").unwrap();

        let report = run(&f).unwrap();
        assert_eq!(report.imported, 1);
        let store = open_store(APP, &f.root).unwrap();
        let key = store
            .authenticate(token.expose())
            .expect("the old token works");
        assert_eq!(key.id.as_str(), mine.id.to_string());
        assert_eq!(key.name, "pendant");
        assert_eq!(store.list().len(), 1);
        assert!(f.data.join(IMPORTED_MARKER).is_file());
    }

    #[test]
    fn second_run_does_nothing() {
        let mut f = fixture();
        f.ledger.add("pendant", None, vec![APP.into()]).unwrap();
        run(&f).unwrap();
        std::fs::remove_dir_all(sapphire_framework_keys::store_dir(APP, &f.root)).unwrap();
        assert_eq!(run(&f).unwrap(), ImportReport::default());
        assert!(!sapphire_framework_keys::store_dir(APP, &f.root).exists());
    }

    #[test]
    fn existing_id_is_skipped() {
        let mut f = fixture();
        let (d, _) = f.ledger.add("pendant", None, vec![APP.into()]).unwrap();
        // Another host imported it, and the workspace synced.
        let mut store = open_store(APP, &f.root).unwrap();
        let (mut key, _) = store.add("placeholder", None, None).unwrap();
        key.id = KeyId::new(&d.id.to_string()).unwrap();
        store.import(key).unwrap();

        let report = run(&f).unwrap();
        assert_eq!(
            report,
            ImportReport {
                imported: 0,
                skipped: 1
            }
        );
        assert!(f.data.join(IMPORTED_MARKER).is_file());
    }

    #[test]
    fn missing_bridge_dir_imports_nothing_and_tries_again_later() {
        let f = fixture();
        // Nothing was added, so no ledger, and no bridge directory, was ever written.
        assert!(!f.bridge.exists());
        assert_eq!(run(&f).unwrap(), ImportReport::default());
        // A bridge not run since its directory moved (Windows' Roaming to Local) has it
        // later: no marker yet.
        assert!(!f.data.join(IMPORTED_MARKER).exists());
    }

    #[cfg(unix)]
    #[test]
    fn failure_leaves_no_marker() {
        use std::os::unix::fs::PermissionsExt as _;
        let mut f = fixture();
        f.ledger.add("pendant", None, vec![APP.into()]).unwrap();
        let app_dir = f.root.join(format!(".{APP}"));
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::set_permissions(&app_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = run(&f);
        std::fs::set_permissions(&app_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert!(!f.data.join(IMPORTED_MARKER).exists());
    }
}
