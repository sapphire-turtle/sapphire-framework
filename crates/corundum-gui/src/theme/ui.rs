//! [`settings_ui`]: the theme's settings, for an app's settings screen.

use super::picker::report;
use super::{Contrast, Swatch, ThemeSettings, Tint};

/// Light / dark / system, the contrast, and the accent's hue and strength, with a preview.
/// A change is [`install`](super::install)ed at once, so a slider being dragged previews
/// live; `true` comes once the change is made — a click, or the end of a drag — so save
/// `settings` then.
pub fn settings_ui(ui: &mut egui::Ui, settings: &mut ThemeSettings) -> bool {
    let before = settings.clone();
    // The sliders edit a copy, written back only when one moves: they would clamp an
    // accent outside their range, and that is not the user's change.
    let (mut chroma, mut hue) = match settings.accent() {
        Tint::Custom { chroma, hue } => (chroma, hue),
        Tint::Swatch(s) => s.chroma_hue(),
    };
    let mut moved = false;
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
            let r = ui.add(egui::Slider::new(&mut hue, 0.0..=360.0).suffix("°"));
            dragging |= r.dragged();
            moved |= r.changed();
            ui.end_row();

            ui.label("Accent strength");
            let r = ui.add(egui::Slider::new(&mut chroma, 0.0..=Tint::MAX_CHROMA));
            dragging |= r.dragged();
            moved |= r.changed();
            ui.end_row();
            if moved {
                settings.accent = Some(Tint::Custom { chroma, hue });
            }

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

/// What a change does: install the new settings.
fn apply(ctx: &egui::Context, settings: &ThemeSettings) {
    super::install(ctx, settings);
}

/// The accent as a fill and as text, in the theme being edited.
fn preview(ui: &mut egui::Ui, settings: &ThemeSettings) {
    use super::Theme;
    let dark = ui.ctx().theme() == egui::Theme::Dark;
    let theme = Theme::new(dark, settings.contrast, settings.accent());
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
    use crate::theme::{Swatch, Theme, Tint, current};

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
            Theme::new(dark, Contrast::High, Tint::Swatch(Swatch::Blue))
        );
    }

    #[test]
    fn out_of_range_accents_are_not_reported_as_changes() {
        for (hue, chroma) in [(-30.0, 0.15), (720.0, 0.15), (400.0, 0.3)] {
            let accent = Tint::Custom { chroma, hue };
            let mut settings = ThemeSettings {
                accent: Some(accent),
                ..Default::default()
            };
            let mut changed = false;
            render([600.0, 400.0], |ui| {
                changed |= settings_ui(ui, &mut settings)
            });
            assert!(!changed, "{accent:?} reported as a change");
            for dark in [true, false] {
                assert_eq!(
                    Theme::new(dark, Contrast::Medium, settings.accent()),
                    Theme::new(dark, Contrast::Medium, accent),
                    "{accent:?} drawn differently"
                );
            }
        }
    }

    #[test]
    fn a_hue_at_the_slider_end_stays_there() {
        let mut settings = ThemeSettings {
            accent: Some(Tint::Custom {
                chroma: 0.15,
                hue: 360.0,
            }),
            ..Default::default()
        };
        render([600.0, 400.0], |ui| {
            settings_ui(ui, &mut settings);
        });
        assert_eq!(
            settings.accent,
            Some(Tint::Custom {
                chroma: 0.15,
                hue: 360.0
            })
        );
    }
}
