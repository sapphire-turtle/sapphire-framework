//! The look shared by the sapphire apps and asteria: what an egui app's window is made of
//! before any of its own screens.
//!
//! [`Shell`] is the window frame: the app's screens on a rail (a bottom bar when narrow, as
//! on a phone), with a footer for Settings and a slot of the app's own. [`fonts`] finds a
//! CJK fallback font on the system; [`icons`] is a set of Lucide icons and the SVG loader
//! that draws them; [`theme`] is the colours — light and dark, the contrast, the accent.
//!
//! The components depend on egui alone, so they render inside any host's egui context, on
//! the desktops and on phones. With the `eframe` feature (on by default), [`window::run`]
//! opens a desktop window on wgpu the same way for every app, set up by [`gpu`]; an app that
//! renders egui without eframe turns default features off.

/// The egui this crate renders with. Apps should use this re-export so their egui is
/// always the one the components were built against.
pub use egui;

/// The eframe [`window::run`] opens windows with, for the same reason as [`egui`].
#[cfg(feature = "eframe")]
pub use eframe;

pub mod fonts;
#[cfg(feature = "eframe")]
pub mod gpu;
pub mod icons;
mod shell;
pub mod theme;
#[cfg(feature = "eframe")]
pub mod window;

#[cfg(test)]
mod test_support;

pub use shell::{Action, COMPACT_WIDTH, Destination, Shell};
