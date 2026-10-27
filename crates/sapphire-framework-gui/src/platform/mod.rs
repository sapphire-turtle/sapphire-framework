//! What the views need from the platform and cannot assume: a folder dialog, a file
//! manager, a clipboard, and — on a phone — the one place the app may keep its workspaces.
//!
//! On a desktop the user picks folders anywhere; on Android and iOS there is no folder
//! dialog for an app's own files and no file manager that can see them, so the views offer
//! "create in this app's storage" instead, under [`app_storage`].
//!
//! The dialogs, the file manager and the clipboard are corundum-framework-gui's, shared with
//! asteria, re-exported here; the workspaces' storage is sapphire's.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use corundum_framework_gui::platform::{
    AUDIO_EXTENSIONS, FilePicker, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS, clipboard_text,
    file_manager_available, file_picker_available, folder_picker_available, needs_paste_button,
    open_in_file_manager, pick_folder, window_insets,
};

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

/// Is `name` usable as one folder name under [`app_storage`]? Not empty, not `.` or `..`,
/// and no separator — so a typed name can never reach outside the app's storage.
pub fn valid_workspace_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
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
