//! What a window needs from the platform and egui cannot give it: the clipboard where egui
//! has none, and the margins the system's bars and keyboard take (Android).
//!
//! The Android half reads them through JNI, with the `android` feature; everywhere else
//! these say there is nothing to read.

#[cfg(all(target_os = "android", feature = "android"))]
mod android;

/// Text on the system clipboard, where egui cannot read it itself.
///
/// egui-winit's clipboard covers the desktops; on Android it has none, so a field that
/// takes a long pasted value (an invite ticket) offers a Paste button reading this.
/// `None` where nothing is there, or where egui's own clipboard is the one to use.
pub fn clipboard_text() -> Option<String> {
    #[cfg(all(target_os = "android", feature = "android"))]
    {
        android::clipboard_text()
    }
    #[cfg(not(all(target_os = "android", feature = "android")))]
    {
        None
    }
}

/// The margins the window must leave for the system's bars and the keyboard, in physical
/// pixels (left, top, right, bottom), where the window is drawn edge to edge under them.
pub fn window_insets() -> Option<[i32; 4]> {
    #[cfg(all(target_os = "android", feature = "android"))]
    {
        android::window_insets()
    }
    #[cfg(not(all(target_os = "android", feature = "android")))]
    {
        None
    }
}

/// Tell the platform layer which activity the window belongs to (Android).
#[cfg(all(target_os = "android", feature = "android"))]
pub(crate) fn set_android_activity(activity: *mut std::ffi::c_void) {
    android::set_activity(activity);
}

/// Does this platform need [`clipboard_text`] (a Paste button) rather than egui's paste?
pub fn needs_paste_button() -> bool {
    cfg!(target_os = "android")
}

// Off Android only: on a phone these read the real clipboard and insets.
#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;

    #[test]
    fn off_android_there_is_no_clipboard_insets_or_paste_button() {
        assert_eq!(clipboard_text(), None);
        assert_eq!(window_insets(), None);
        assert!(!needs_paste_button());
    }
}
