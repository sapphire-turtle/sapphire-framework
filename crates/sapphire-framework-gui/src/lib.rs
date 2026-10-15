//! Shared egui components for sapphire-framework apps.
//!
//! [`views::WorkspacePicker`] shows the workspace the app server serves and switches it
//! through the server. [`WorkspaceManager`], a list + management screen over the shared
//! [`WorkspaceRegistry`](sapphire_backend::WorkspaceRegistry), predates the server serving
//! one workspace; it is deprecated (#217) and goes once the apps have moved off it.
//!
//! With the `eframe` feature, [`app::run`] starts an app's desktop window the same way for
//! every app.
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
pub mod fonts;
mod panel;
pub mod views;

pub use panel::SyncPanel;

mod manager;
#[allow(deprecated)]
pub use manager::{WorkspaceAction, WorkspaceHost, WorkspaceManager};
