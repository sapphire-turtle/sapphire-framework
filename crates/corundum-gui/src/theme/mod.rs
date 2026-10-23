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
