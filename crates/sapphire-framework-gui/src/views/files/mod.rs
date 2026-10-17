//! [`FileBrowser`]: the workspace's files, read-only, with a text preview.
//!
//! A phone gives its user no way into an app's private directory, so the app shows the
//! synced folder itself. The desktop shows the same screen.

pub mod model;
mod view;

pub use view::FileBrowser;
