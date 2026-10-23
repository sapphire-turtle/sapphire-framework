//! [`settings_ui`]: the theme's settings, for an app's settings screen.

use super::{Accent, Contrast, ThemeSettings};

/// Light / dark / system, the contrast, and the accent's hue and strength, with a preview.
/// A change is [`install`](super::install)ed at once, so a slider being dragged previews
/// live; `true` comes once the change is made — a click, or the end of a drag — so save
/// `settings` then.
pub fn settings_ui(ui: &mut egui::Ui, settings: &mut ThemeSettings) -> bool {
    // The sliders would clamp an accent outside their range and look changed: bring it in
    // the way the theme reads it (hue modulo 360, keeping 360 itself; chroma at most the maximum) first.
    if !(0.0..=360.0).contains(&settings.accent.hue) {
        settings.accent.hue = settings.accent.hue.rem_euclid(360.0);
    }
    settings.accent.chroma = settings.accent.chroma.clamp(0.0, Accent::MAX_CHROMA);
    let before = settings.clone();
    let mut dragging = false;
    egui::Grid::new("corundum_theme_settings")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Appearance");
            ui.horizontal(|ui| {
                use egui::ThemePreference::*;
                ui.radio_value(&mut settings.preference, Light, "Light");
                ui.radio_value(&mut settings.preference, Dark, "Dark");
                ui.radio_value(&mut settings.preference, System, "System");
            });
            ui.end_row();

            ui.label("Contrast");
            ui.horizontal(|ui| {
                ui.radio_value(&mut settings.contrast, Contrast::Low, "Low");
                ui.radio_value(&mut settings.contrast, Contrast::Medium, "Medium");
                ui.radio_value(&mut settings.contrast, Contrast::High, "High");
            });
            ui.end_row();

            ui.label("Accent hue");
            dragging |= ui
                .add(egui::Slider::new(&mut settings.accent.hue, 0.0..=360.0).suffix("°"))
                .dragged();
            ui.end_row();

            ui.label("Accent strength");
            dragging |= ui
                .add(egui::Slider::new(
                    &mut settings.accent.chroma,
                    0.0..=Accent::MAX_CHROMA,
                ))
                .dragged();
            ui.end_row();

            ui.label("Preview");
            preview(ui, settings);
            ui.end_row();
        });
    let changed = *settings != before;
    if changed {
        apply(ui.ctx(), settings);
    }
    let id = ui.id().with("corundum_theme_held");
    let mut held = ui.data(|d| d.get_temp(id)).unwrap_or(false);
    let made = report(changed, dragging, &mut held);
    ui.data_mut(|d| d.insert_temp(id, held));
    made
}

/// Whether a change is made this frame. A change during a drag is `held` until the drag
/// ends, so an app saving on each change saves once per drag, not once per frame.
fn report(changed: bool, dragging: bool, held: &mut bool) -> bool {
    if dragging {
        *held |= changed;
        false
    } else {
        changed | std::mem::take(held)
    }
}

/// What a change does: install the new settings.
fn apply(ctx: &egui::Context, settings: &ThemeSettings) {
    super::install(ctx, settings);
}

/// The accent as a fill and as text, in the theme being edited.
fn preview(ui: &mut egui::Ui, settings: &ThemeSettings) {
    use super::{Swatch, Theme};
    let dark = ui.ctx().theme() == egui::Theme::Dark;
    let theme = Theme::new(dark, settings.contrast, settings.accent);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 4.0, theme.fill(Swatch::Accent));
        ui.colored_label(theme.text(Swatch::Accent), "Accent");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{render, render_in};
    use crate::theme::{Theme, current};

    #[test]
    fn settings_ui_renders() {
        let mut settings = ThemeSettings::default();
        let mut changed = false;
        render([600.0, 400.0], |ui| {
            changed |= settings_ui(ui, &mut settings)
        });
        assert!(!changed);
        assert_eq!(settings, ThemeSettings::default());
    }

    #[test]
    fn changes_reinstall() {
        let ctx = egui::Context::default();
        let settings = ThemeSettings {
            contrast: Contrast::High,
            ..Default::default()
        };
        render_in(&ctx, [600.0, 400.0], |ui| apply(ui.ctx(), &settings));
        let dark = ctx.theme() == egui::Theme::Dark;
        assert_eq!(
            *current(&ctx),
            Theme::new(dark, Contrast::High, Accent::default())
        );
    }

    #[test]
    fn out_of_range_accents_are_not_reported_as_changes() {
        for (hue, chroma) in [(-30.0, 0.15), (720.0, 0.15), (400.0, 0.3)] {
            let accent = Accent { chroma, hue };
            let mut settings = ThemeSettings {
                accent,
                ..Default::default()
            };
            let mut changed = false;
            render([600.0, 400.0], |ui| {
                changed |= settings_ui(ui, &mut settings)
            });
            assert!(!changed, "{accent:?} reported as a change");
            for dark in [true, false] {
                assert_eq!(
                    Theme::new(dark, Contrast::Medium, settings.accent),
                    Theme::new(dark, Contrast::Medium, accent),
                    "{accent:?} drawn differently"
                );
            }
        }
    }

    #[test]
    fn a_hue_at_the_slider_end_stays_there() {
        let mut settings = ThemeSettings {
            accent: Accent {
                chroma: 0.15,
                hue: 360.0,
            },
            ..Default::default()
        };
        render([600.0, 400.0], |ui| {
            settings_ui(ui, &mut settings);
        });
        assert_eq!(settings.accent.hue, 360.0);
    }

    #[test]
    fn a_drag_is_reported_once_it_ends() {
        let mut held = false;
        assert!(!report(true, true, &mut held), "mid-drag");
        assert!(!report(true, true, &mut held), "mid-drag");
        assert!(report(false, false, &mut held), "the drag ended");
        assert!(!report(false, false, &mut held), "only once");
        assert!(report(true, false, &mut held), "a click");
        assert!(
            !report(false, true, &mut held),
            "a drag that changed nothing"
        );
        assert!(!report(false, false, &mut held));
    }
}
