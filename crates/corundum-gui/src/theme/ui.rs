//! [`settings_ui`]: the theme's settings, for an app's settings screen.

use super::{Accent, Contrast, ThemeSettings};

/// Light / dark / system, the contrast, and the accent's hue and strength, with a preview.
/// A change is [`install`](super::install)ed at once and returns `true`: save `settings`
/// then.
pub fn settings_ui(ui: &mut egui::Ui, settings: &mut ThemeSettings) -> bool {
    let before = settings.clone();
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
            ui.add(egui::Slider::new(&mut settings.accent.hue, 0.0..=360.0).suffix("°"));
            ui.end_row();

            ui.label("Accent strength");
            ui.add(egui::Slider::new(
                &mut settings.accent.chroma,
                0.0..=Accent::MAX_CHROMA,
            ));
            ui.end_row();

            ui.label("Preview");
            preview(ui, settings);
            ui.end_row();
        });
    let changed = *settings != before;
    if changed {
        apply(ui.ctx(), settings);
    }
    changed
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
}
