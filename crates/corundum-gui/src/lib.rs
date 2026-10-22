//! The look shared by the sapphire apps and asteria: what an egui app's window is made of
//! before any of its own screens.
//!
//! [`Shell`] is the window frame: the app's screens on a rail (a bottom bar when narrow, as
//! on a phone), with a footer for Settings and a slot of the app's own. [`fonts`] finds a
//! CJK fallback font on the system; [`icons`] is a set of Lucide icons and the SVG loader
//! that draws them.
//!
//! The crate depends on egui alone (no eframe), so it renders inside any host's egui
//! context, on the desktops and on phones.

/// The egui this crate renders with. Apps should use this re-export so their egui is
/// always the one the components were built against.
pub use egui;

pub mod fonts;
pub mod icons;
mod shell;

#[cfg(test)]
mod test_support;

pub use shell::{Action, COMPACT_WIDTH, Destination, Shell};
