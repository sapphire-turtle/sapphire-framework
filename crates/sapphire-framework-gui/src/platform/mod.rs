//! What the views need from the platform and cannot assume: a folder dialog, a file
//! manager, a clipboard, and — on a phone — the one place the app may keep its workspaces.
//!
//! On a desktop the user picks folders anywhere; on Android and iOS there is no folder
//! dialog for an app's own files and no file manager that can see them, so the views offer
//! "create in this app's storage" instead, under [`app_storage`].

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[cfg(target_os = "android")]
mod android;

/// Where this app keeps its workspaces, when the platform gives it no folder dialog. Set
/// once by the mobile host; `None` on a desktop.
static APP_STORAGE: OnceLock<PathBuf> = OnceLock::new();

/// Keep the app's workspaces under `dir` (`<dir>/<workspace name>`). The mobile host calls
/// this once at start-up; a second call is ignored.
pub fn set_app_storage(dir: PathBuf) {
    let _ = APP_STORAGE.set(dir);
}

/// The directory new workspaces go in on a platform without a folder dialog.
pub fn app_storage() -> Option<&'static Path> {
    APP_STORAGE.get().map(PathBuf::as_path)
}

/// Can the user pick a folder anywhere (a desktop), or only name one in [`app_storage`]?
pub fn folder_picker_available() -> bool {
    cfg!(not(any(target_os = "android", target_os = "ios")))
}

/// Ask the user for a folder, titled `title`. `None` when they cancel — or always, where
/// there is no folder dialog ([`folder_picker_available`]).
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        rfd::FileDialog::new().set_title(title).pick_folder()
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = title;
        None
    }
}

/// Can [`open_in_file_manager`] show a folder here?
pub fn file_manager_available() -> bool {
    folder_picker_available()
}

/// Show `path` in the platform's file manager. Failures are ignored: it is a convenience.
pub fn open_in_file_manager(path: &Path) {
    #[cfg(windows)]
    let program = Some("explorer");
    #[cfg(target_os = "macos")]
    let program = Some("open");
    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "android", target_os = "ios"))
    ))]
    let program = Some("xdg-open");
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let program: Option<&str> = None;
    if let Some(program) = program {
        let _ = std::process::Command::new(program).arg(path).spawn();
    }
}

/// Is `name` usable as one folder name under [`app_storage`]? Not empty, not `.` or `..`,
/// and no separator — so a typed name can never reach outside the app's storage.
pub fn valid_workspace_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// Text on the system clipboard, where egui cannot read it itself.
///
/// egui-winit's clipboard covers the desktops; on Android it has none, so a field that
/// takes a long pasted value (an invite ticket) offers a Paste button reading this.
/// `None` where nothing is there, or where egui's own clipboard is the one to use.
pub fn clipboard_text() -> Option<String> {
    #[cfg(target_os = "android")]
    {
        android::clipboard_text()
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

/// Does this platform need [`clipboard_text`] (a Paste button) rather than egui's paste?
pub fn needs_paste_button() -> bool {
    cfg!(target_os = "android")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_names_are_single_components() {
        assert!(valid_workspace_name("notes"));
        assert!(valid_workspace_name("日記 2026"));
        for bad in ["", "  ", ".", "..", "a/b", "a\\b", "../x", "/abs"] {
            assert!(!valid_workspace_name(bad), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_desktop_has_a_folder_picker_and_no_paste_button() {
        assert!(folder_picker_available());
        assert!(!needs_paste_button());
    }
}
