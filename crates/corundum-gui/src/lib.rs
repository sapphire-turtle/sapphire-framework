//! The look shared by the sapphire apps and asteria: what an egui app's window is made of
//! before any of its own screens.
//!
//! [`fonts`] finds a CJK fallback font on the system; [`icons`] is a set of Lucide icons and
//! the SVG loader that draws them.
//!
//! The crate depends on egui alone (no eframe), so it renders inside any host's egui
//! context, on the desktops and on phones.

/// The egui this crate renders with. Apps should use this re-export so their egui is
/// always the one the components were built against.
pub use egui;

pub mod fonts;
pub mod icons;
