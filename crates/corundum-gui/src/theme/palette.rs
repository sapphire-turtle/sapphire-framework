//! The grid: [`Tone`]s by [`Swatch`]es, and [`Theme`], its colours worked out.

use egui::Color32;

use super::color::{Oklch, to_color32};
use super::{Accent, Contrast};

/// A lightness level, from the background side to the text side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// Text edit backgrounds.
    Sunken,
    /// Panels and windows.
    Base,
    /// Inactive buttons, code, tinted bands.
    Raised,
    /// Hovered widgets.
    Hover,
    /// Pressed and selected widgets.
    Pressed,
    /// Separators and outlines.
    Border,
    /// Weak text.
    Muted,
    /// Body text.
    Text,
    /// The text of hovered and active widgets.
    Strong,
}

impl Tone {
    /// Every tone, background side first.
    pub const ALL: [Tone; 9] = [
        Tone::Sunken,
        Tone::Base,
        Tone::Raised,
        Tone::Hover,
        Tone::Pressed,
        Tone::Border,
        Tone::Muted,
        Tone::Text,
        Tone::Strong,
    ];

    /// How much of a swatch's chroma the tone keeps: fills stay quiet, text takes it all.
    fn chroma_scale(self) -> f32 {
        match self {
            Tone::Sunken | Tone::Base => 0.2,
            Tone::Raised => 0.35,
            Tone::Hover => 0.45,
            Tone::Pressed => 0.6,
            Tone::Border => 0.4,
            Tone::Muted | Tone::Text | Tone::Strong => 1.0,
        }
    }
}

/// A hue and a chroma. Red to Purple line up with Base16's base08 to base0E.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Swatch {
    /// Greys.
    Neutral,
    /// The user's [`Accent`].
    Accent,
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
}

impl Swatch {
    /// Every swatch.
    pub const ALL: [Swatch; 9] = [
        Swatch::Neutral,
        Swatch::Accent,
        Swatch::Red,
        Swatch::Orange,
        Swatch::Yellow,
        Swatch::Green,
        Swatch::Cyan,
        Swatch::Blue,
        Swatch::Purple,
    ];
    /// Something went well: a service running, a device online.
    pub const SUCCESS: Swatch = Swatch::Green;
    /// Something needs a look.
    pub const WARNING: Swatch = Swatch::Orange;
    /// Something failed.
    pub const DANGER: Swatch = Swatch::Red;
    /// Something to know.
    pub const INFO: Swatch = Swatch::Blue;

    /// OKLCH (chroma, hue).
    fn chroma_hue(self, accent: Accent) -> (f32, f32) {
        match self {
            Swatch::Neutral => (0.0, 0.0),
            Swatch::Accent => (accent.chroma.clamp(0.0, Accent::MAX_CHROMA), accent.hue),
            Swatch::Red => (0.17, 25.0),
            Swatch::Orange => (0.15, 55.0),
            Swatch::Yellow => (0.14, 95.0),
            Swatch::Green => (0.14, 145.0),
            Swatch::Cyan => (0.12, 200.0),
            Swatch::Blue => (0.15, 255.0),
            Swatch::Purple => (0.15, 305.0),
        }
    }
}

/// Each tone's OKLCH lightness, in [`Tone::ALL`]'s order.
fn ladder(dark: bool, contrast: Contrast) -> [f32; 9] {
    match (dark, contrast) {
        (true, Contrast::Low) => [0.18, 0.23, 0.29, 0.34, 0.39, 0.42, 0.66, 0.80, 0.88],
        (true, Contrast::Medium) => [0.16, 0.21, 0.27, 0.32, 0.38, 0.44, 0.72, 0.88, 0.96],
        (true, Contrast::High) => [0.10, 0.14, 0.22, 0.28, 0.34, 0.50, 0.82, 0.96, 0.995],
        (false, Contrast::Low) => [0.985, 0.96, 0.91, 0.87, 0.83, 0.78, 0.58, 0.42, 0.32],
        (false, Contrast::Medium) => [0.995, 0.97, 0.92, 0.88, 0.84, 0.78, 0.50, 0.30, 0.20],
        (false, Contrast::High) => [1.0, 0.99, 0.93, 0.89, 0.85, 0.70, 0.40, 0.16, 0.06],
    }
}

/// The colours of one theme: every [`Tone`] of every [`Swatch`], worked out when built.
#[derive(Clone, PartialEq, Debug)]
pub struct Theme {
    dark: bool,
    cells: [[Color32; 9]; 9],
}

impl Theme {
    /// The dark or the light theme at `contrast`, with `accent`.
    pub fn new(dark: bool, contrast: Contrast, accent: Accent) -> Theme {
        let ladder = ladder(dark, contrast);
        let cells = std::array::from_fn(|t| {
            let tone = Tone::ALL[t];
            std::array::from_fn(|s| {
                let (c, h) = Swatch::ALL[s].chroma_hue(accent);
                to_color32(Oklch {
                    l: ladder[t],
                    c: c * tone.chroma_scale(),
                    h,
                })
            })
        });
        Theme { dark, cells }
    }

    /// Whether this is the dark theme.
    pub fn is_dark(&self) -> bool {
        self.dark
    }

    /// `swatch` at `tone`.
    pub fn color(&self, tone: Tone, swatch: Swatch) -> Color32 {
        self.cells[tone as usize][swatch as usize]
    }

    /// `swatch` as text: at [`Tone::Text`].
    pub fn text(&self, swatch: Swatch) -> Color32 {
        self.color(Tone::Text, swatch)
    }

    /// `swatch` as a fill: at [`Tone::Raised`].
    pub fn fill(&self, swatch: Swatch) -> Color32 {
        self.color(Tone::Raised, swatch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::color::{contrast_ratio, from_color32};
    use crate::theme::{Accent, Contrast};

    const CONTRASTS: [Contrast; 3] = [Contrast::Low, Contrast::Medium, Contrast::High];

    /// (`Text` on `Base`, `Muted` and swatch text on `Base`) for `contrast`.
    fn targets(contrast: Contrast) -> (f32, f32) {
        match contrast {
            Contrast::Low => (4.5, 3.0),
            Contrast::Medium => (7.0, 4.5),
            Contrast::High => (12.0, 7.0),
        }
    }

    fn themes(accent: Accent) -> impl Iterator<Item = (Theme, Contrast)> {
        [false, true]
            .into_iter()
            .flat_map(move |dark| CONTRASTS.map(|c| (Theme::new(dark, c, accent), c)))
    }

    fn on_base(t: &Theme, c: egui::Color32) -> f32 {
        contrast_ratio(c, t.color(Tone::Base, Swatch::Neutral))
    }

    #[test]
    fn every_ladder_meets_its_targets() {
        for (t, contrast) in themes(Accent::default()) {
            let (text, muted) = targets(contrast);
            let name = format!("dark {} {contrast:?}", t.is_dark());
            let r = on_base(&t, t.color(Tone::Text, Swatch::Neutral));
            assert!(r >= text, "{name}: text {r}");
            let r = on_base(&t, t.color(Tone::Muted, Swatch::Neutral));
            assert!(r >= muted, "{name}: muted {r}");
            for s in Swatch::ALL {
                let r = on_base(&t, t.text(s));
                assert!(r >= muted, "{name}: {s:?} {r}");
            }
        }
    }

    #[test]
    fn ladders_are_monotonic() {
        for (t, contrast) in themes(Accent::default()) {
            let l: Vec<f32> = Tone::ALL
                .iter()
                .map(|&tone| from_color32(t.color(tone, Swatch::Neutral)).l)
                .collect();
            for w in l.windows(2) {
                let rising = w[1] > w[0];
                assert_eq!(rising, t.is_dark(), "dark {} {contrast:?}: {l:?}", t.is_dark());
            }
        }
    }

    #[test]
    fn accent_extremes_meet_the_swatch_target() {
        for hue in (0..360).step_by(10) {
            let accent = Accent { chroma: Accent::MAX_CHROMA, hue: hue as f32 };
            for (t, contrast) in themes(accent) {
                let r = on_base(&t, t.text(Swatch::Accent));
                assert!(
                    r >= targets(contrast).1,
                    "hue {hue}, dark {} {contrast:?}: {r}",
                    t.is_dark()
                );
            }
        }
    }

    #[test]
    fn grey_accent_is_neutral() {
        for hue in [0.0, 123.0, 300.0] {
            let t = Theme::new(true, Contrast::Medium, Accent { chroma: 0.0, hue });
            for tone in Tone::ALL {
                assert_eq!(t.color(tone, Swatch::Accent), t.color(tone, Swatch::Neutral));
            }
        }
    }

    #[test]
    fn hue_wraps() {
        for (hue, same) in [(360.0, 0.0), (-30.0, 330.0), (720.0, 0.0)] {
            let a = Theme::new(false, Contrast::Medium, Accent { chroma: 0.15, hue });
            let b = Theme::new(false, Contrast::Medium, Accent { chroma: 0.15, hue: same });
            for tone in Tone::ALL {
                assert_eq!(a.color(tone, Swatch::Accent), b.color(tone, Swatch::Accent), "{hue}");
            }
        }
    }

    #[test]
    fn aliases() {
        assert_eq!(Swatch::SUCCESS, Swatch::Green);
        assert_eq!(Swatch::WARNING, Swatch::Orange);
        assert_eq!(Swatch::DANGER, Swatch::Red);
        assert_eq!(Swatch::INFO, Swatch::Blue);
    }
}
