//! The colours: light and dark, three contrast levels, an accent of the user's.
//!
//! A theme is a grid. Its rows are [`Tone`]s — lightness levels from the sunken
//! background to strong text — and its columns are [`Swatch`]es, each a hue and a chroma.
//! Every colour is one cell, so every colour, the accent and the status colours alike,
//! takes its lightness from where it is used: switching light and dark, or the contrast,
//! carries all of them with it.
//!
//! The lightness is OKLCH's, which follows perceived lightness: one tone gives about the
//! same contrast whatever the hue.

mod color;
mod palette;
mod visuals;

pub use palette::{Swatch, Theme, Tone};

/// How far apart the text and the background are.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Contrast {
    /// Body text at WCAG AA (4.5:1), quieter edges.
    Low,
    /// Body text at WCAG AAA (7:1).
    #[default]
    Medium,
    /// Body text at 12:1 and up.
    High,
}

/// The user's accent: a chroma and a hue. Its lightness comes from where it is used.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Accent {
    /// OKLCH chroma, 0 (grey) to [`MAX_CHROMA`](Self::MAX_CHROMA).
    pub chroma: f32,
    /// OKLCH hue in degrees; taken modulo 360.
    pub hue: f32,
}

impl Accent {
    /// The most chroma an accent may have: as much as every hue can carry at every tone
    /// and still meet the contrast targets.
    pub const MAX_CHROMA: f32 = 0.25;
}

impl Default for Accent {
    /// A blue.
    fn default() -> Accent {
        Accent {
            chroma: 0.15,
            hue: 255.0,
        }
    }
}
