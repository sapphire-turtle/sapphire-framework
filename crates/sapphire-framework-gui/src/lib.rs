//! Shared egui components for sapphire-framework apps.
//!
//! [`Shell`] is every app's window frame: the app's screens on a rail (a bottom bar when
//! narrow, as on a phone), and [`Settings`] at its foot — the app's own sections, the
//! framework's sync screens and About — with the services' [`Health`] beside it.
//!
//! [`views::WorkspacePicker`] shows the workspace the app server serves and switches it
//! through the server. [`WorkspaceManager`], a list + management screen over the shared
//! [`WorkspaceRegistry`](sapphire_backend::WorkspaceRegistry), predates the server serving
//! one workspace; it is deprecated (#217) and goes once the apps have moved off it.
//!
//! With the `eframe` feature, [`app::run`] starts an app's desktop window the same way for
//! every app; with `mobile`, [`mobile`] starts it on a phone, with the bridge and the app
//! server in the same process. [`views::FileBrowser`] shows the workspace's files, which a
//! phone's user has no other way to see.
//!
//! The [`client`] module is the egui-free half: a background task that keeps a snapshot of
//! the bridge and the app server current and carries out the commands the views (workspace,
//! workgroup and device panels) send it.

/// The egui this crate renders with. Apps should use this re-export so their egui is
/// always the one the components were built against.
pub use egui;

/// The eframe [`app::run`] opens windows with, for the same reason as [`egui`].
#[cfg(feature = "eframe")]
pub use eframe;

#[cfg(feature = "eframe")]
pub mod app;

pub mod client;
mod health;
#[cfg(feature = "mobile")]
pub mod mobile;
pub mod platform;
mod settings;
mod shell;
pub mod views;

#[cfg(test)]
mod i18n_check;
#[cfg(test)]
mod test_support;

/// This crate's own strings: register them with [`i18n::register`]. [`Shell`] and
/// [`Settings`] do so themselves on their first frame.
pub const RESOURCES: i18n::Resources = i18n::Resources {
    name: "sapphire-framework-gui",
    files: &[
        ("en", include_str!("../locales/en/sapphire.ftl")),
        ("ja", include_str!("../locales/ja/sapphire.ftl")),
    ],
};

/// corundum-gui's translation support, for apps to register their own strings with.
pub use corundum_gui::i18n;
pub use corundum_gui::{Action, COMPACT_WIDTH, Destination};
/// The system CJK fallback font and the Lucide icons, from corundum-gui (shared with asteria).
pub use corundum_gui::{fonts, icons, theme};
pub use health::{Health, bridge_needed};
pub use settings::{Settings, SettingsSection};
pub use shell::Shell;

/// The sync screens on their own: now [`Settings`], which adds the app's sections and
/// About to them.
#[deprecated(note = "renamed to `Settings`; put it in a `Shell` for the rail and the status")]
pub type SyncPanel = Settings;

mod manager;
#[allow(deprecated)]
pub use manager::{WorkspaceAction, WorkspaceHost, WorkspaceManager};
