//! Helpers for tests (feature `test-util`).
//!
//! Not a public interface: everything here stands in for a part of the pairing flow that a
//! later task of the pairing plan implements.

use crate::dir::BridgeDir;
use crate::error::{Error, Result};
use crate::workgroup::Workgroup;

/// Make this host a member of `workgroup`'s workgroup without a pairing.
///
/// Writes `workgroup`'s id and name into this bridge directory as [`Workgroup::create`]
/// would, and copies its device ledger wholesale — the joiner of a real join inherits every
/// record in the founder's ledger, and this stands in for exactly that, including the
/// founder's own record.
///
/// A workgroup this host already belongs to is removed first: a test fixture that creates a
/// workgroup to have something to adopt out of must end up with exactly one. (A real join
/// refuses instead, and says so itself.)
///
/// The two hosts must not share a directory; each keeps its own copy of the root. Returns
/// the adopted `Workgroup`.
pub fn adopt_workgroup(dir: &BridgeDir, workgroup: &Workgroup) -> Result<Workgroup> {
    // One workgroup per host (plan constraint): whatever was here is replaced, stale
    // directories included — a leftover root would otherwise be found by `open` first.
    for entry in std::fs::read_dir(dir.workgroups_dir())? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        }
    }

    // The joined root is the founder's root: the ledger lives inside it (`root/devices/`),
    // and device records are one file each precisely so that a set of them can be copied
    // host to host without inventing ids. The replica store and staging directory are
    // outside the root, so a copy carries no replication state.
    copy_tree(
        &workgroup.dir.join("root"),
        &dir.workgroup_dir(workgroup.id).join("root"),
    )?;
    // The marker directory the sync core pauses without, in case the founder's root has
    // none yet.
    std::fs::create_dir_all(
        dir.workgroup_dir(workgroup.id)
            .join("root")
            .join(format!(".{}", crate::wgsync::WORKSPACE_APP_NAME)),
    )?;

    // Re-open through the front door: `open` finds the directory by id and reads the
    // ledger, so the returned `Workgroup` is exactly what a real join would produce.
    Workgroup::open(dir)?
        .filter(|adopted| adopted.id == workgroup.id)
        .ok_or_else(|| Error::Config("the adopted workgroup did not open".to_owned()))
}

/// Copy `from` and everything under it to `to`, creating `to`.
fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}
