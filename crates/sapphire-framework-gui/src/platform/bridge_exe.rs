//! Where Sapphire Bridge's window is: the executable an app's "Open Sapphire Bridge" runs.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The window's executable name on this OS.
pub fn bridge_desktop_name() -> String {
    format!("sapphire-bridge-desktop{}", std::env::consts::EXE_SUFFIX)
}

/// Where Sapphire Bridge's window is on this host (see [`bridge_desktop_exe_with`]).
pub fn bridge_desktop_exe() -> Option<PathBuf> {
    let own = std::env::current_exe().ok();
    let path = std::env::var_os("PATH");
    bridge_desktop_exe_with(None, own.as_deref(), path.as_deref())
}

/// The first that exists of: beside the program the bridge's service starts (that program
/// itself if it is the window's), beside `own_exe`, and on `path_var`.
pub fn bridge_desktop_exe_with(
    service_exe: Option<&Path>,
    own_exe: Option<&Path>,
    path_var: Option<&OsStr>,
) -> Option<PathBuf> {
    let name = bridge_desktop_name();
    let beside = |exe: &Path| exe.parent().map(|dir| dir.join(&name));
    let service = service_exe.and_then(|exe| {
        if exe.file_name() == Some(OsStr::new(&name)) {
            Some(exe.to_owned())
        } else {
            beside(exe)
        }
    });
    let path = path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(&name));
    service
        .into_iter()
        .chain(own_exe.and_then(beside))
        .chain(path)
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) -> PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
        path.to_owned()
    }

    #[test]
    fn bridge_desktop_exe_order() {
        let tmp = tempfile::tempdir().unwrap();
        let name = bridge_desktop_name();
        let service_dir = tmp.path().join("service");
        let own_dir = tmp.path().join("own");
        let path_dir = tmp.path().join("path");
        let headless = touch(&service_dir.join("sapphire-bridge"));
        let own = touch(&own_dir.join("app"));
        let path_var = std::env::join_paths([&path_dir]).unwrap();

        // (e) nowhere
        assert_eq!(
            bridge_desktop_exe_with(Some(&headless), Some(&own), Some(&path_var)),
            None
        );
        // (d) only on PATH
        let on_path = touch(&path_dir.join(&name));
        assert_eq!(
            bridge_desktop_exe_with(Some(&headless), Some(&own), Some(&path_var)),
            Some(on_path)
        );
        // (c) beside the app, before PATH
        let beside_own = touch(&own_dir.join(&name));
        assert_eq!(
            bridge_desktop_exe_with(Some(&headless), Some(&own), Some(&path_var)),
            Some(beside_own)
        );
        // (a) beside the service's program, before the others
        let beside_service = touch(&service_dir.join(&name));
        assert_eq!(
            bridge_desktop_exe_with(Some(&headless), Some(&own), Some(&path_var)),
            Some(beside_service.clone())
        );
        // (b) the service's program is the window itself
        assert_eq!(
            bridge_desktop_exe_with(Some(&beside_service), None, None),
            Some(beside_service)
        );
    }
}
