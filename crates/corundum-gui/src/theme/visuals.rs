//! [`Theme::visuals`]: the theme as egui's [`Visuals`].

use egui::Visuals;

use super::{Swatch, Theme, Tone};

impl Theme {
    /// egui's look in this theme's colours. Everything else — corner radii, shadows,
    /// spacing — is egui's own dark or light.
    pub fn visuals(&self) -> Visuals {
        let mut v = if self.is_dark() {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        let grey = |tone| self.color(tone, Swatch::Neutral);
        let base = grey(Tone::Base);
        let border = grey(Tone::Border);
        let text = grey(Tone::Text);
        let strong = grey(Tone::Strong);

        v.dark_mode = self.is_dark();
        v.panel_fill = base;
        v.window_fill = base;
        v.window_stroke.color = border;
        v.extreme_bg_color = grey(Tone::Sunken);
        v.faint_bg_color = grey(Tone::Raised);
        v.code_bg_color = grey(Tone::Raised);
        v.weak_text_color = Some(grey(Tone::Muted));
        v.hyperlink_color = self.text(Swatch::Accent);
        v.warn_fg_color = self.text(Swatch::WARNING);
        v.error_fg_color = self.text(Swatch::DANGER);
        v.selection.bg_fill = self.color(Tone::Pressed, Swatch::Accent);
        v.selection.stroke.color = self.text(Swatch::Accent);

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = base;
        w.noninteractive.weak_bg_fill = base;
        w.noninteractive.bg_stroke.color = border;
        w.noninteractive.fg_stroke.color = text;
        for (widget, fill, fg) in [
            (&mut w.inactive, Tone::Raised, text),
            (&mut w.hovered, Tone::Hover, strong),
            (&mut w.active, Tone::Pressed, strong),
            (&mut w.open, Tone::Hover, strong),
        ] {
            widget.bg_fill = grey(fill);
            widget.weak_bg_fill = grey(fill);
            widget.fg_stroke.color = fg;
        }
        w.hovered.bg_stroke.color = border;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::color::contrast_ratio;
    use crate::theme::{Accent, Contrast};

    fn both() -> [Theme; 2] {
        [true, false].map(|dark| Theme::new(dark, Contrast::Medium, Accent::default()))
    }

    #[test]
    fn visuals_take_the_theme_colours() {
        for t in both() {
            let v = t.visuals();
            assert_eq!(v.dark_mode, t.is_dark());
            assert_eq!(v.panel_fill, t.color(Tone::Base, Swatch::Neutral));
            assert_eq!(v.extreme_bg_color, t.color(Tone::Sunken, Swatch::Neutral));
            assert_eq!(v.widgets.inactive.weak_bg_fill, t.color(Tone::Raised, Swatch::Neutral));
            assert_eq!(v.selection.bg_fill, t.color(Tone::Pressed, Swatch::Accent));
            assert_eq!(v.hyperlink_color, t.text(Swatch::Accent));
            assert_eq!(v.error_fg_color, t.text(Swatch::DANGER));
            assert_eq!(v.warn_fg_color, t.text(Swatch::WARNING));
            assert_eq!(v.weak_text_color, Some(t.color(Tone::Muted, Swatch::Neutral)));
        }
    }

    #[test]
    fn non_colour_fields_are_eguis() {
        for t in both() {
            let v = t.visuals();
            let egui = if t.is_dark() { Visuals::dark() } else { Visuals::light() };
            assert_eq!(v.window_corner_radius, egui.window_corner_radius);
            assert_eq!(v.window_shadow.offset, egui.window_shadow.offset);
        }
    }

    #[test]
    fn selected_text_is_readable() {
        for contrast in [Contrast::Low, Contrast::Medium, Contrast::High] {
            let target = match contrast {
                Contrast::Low => 3.0,
                Contrast::Medium => 4.5,
                Contrast::High => 7.0,
            };
            let accents = std::iter::once(Accent::default()).chain(
                (0..360)
                    .step_by(30)
                    .map(|h| Accent { chroma: Accent::MAX_CHROMA, hue: h as f32 }),
            );
            for accent in accents {
                for dark in [true, false] {
                    let v = Theme::new(dark, contrast, accent).visuals();
                    let r = contrast_ratio(v.selection.stroke.color, v.selection.bg_fill);
                    assert!(r >= target, "dark {dark} {contrast:?} {accent:?}: {r}");
                }
            }
        }
    }
}
