//! [`settings_ui`]: the theme's settings, for an app's settings screen.

use super::{Contrast, DEFAULT_ACCENT, ThemeSettings, TintPicker};
use crate::tr;

/// Light / dark / system, the contrast, and the accent (a [`TintPicker`] with its
/// preview). A change is [`install`](super::install)ed at once, so dragging on the accent's
/// disk previews live; `true` comes once the change is made — a click, or the end of a
/// drag — so save `settings` then.
pub fn settings_ui(ui: &mut egui::Ui, settings: &mut ThemeSettings) -> bool {
    let before = settings.clone();
    let mut picked = false;
    egui::Grid::new("corundum_theme_settings")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label(tr!(ui, "corundum-theme-appearance"));
            ui.horizontal(|ui| {
                use egui::ThemePreference::*;
                ui.radio_value(
                    &mut settings.preference,
                    Light,
                    tr!(ui, "corundum-theme-light"),
                );
                ui.radio_value(
                    &mut settings.preference,
                    Dark,
                    tr!(ui, "corundum-theme-dark"),
                );
                ui.radio_value(
                    &mut settings.preference,
                    System,
                    tr!(ui, "corundum-theme-system"),
                );
            });
            ui.end_row();

            ui.label(tr!(ui, "corundum-theme-contrast"));
            ui.horizontal(|ui| {
                ui.radio_value(
                    &mut settings.contrast,
                    Contrast::Low,
                    tr!(ui, "corundum-theme-low"),
                );
                ui.radio_value(
                    &mut settings.contrast,
                    Contrast::Medium,
                    tr!(ui, "corundum-theme-medium"),
                );
                ui.radio_value(
                    &mut settings.contrast,
                    Contrast::High,
                    tr!(ui, "corundum-theme-high"),
                );
            });
            ui.end_row();

            ui.label(tr!(ui, "corundum-theme-accent"));
            picked = TintPicker::new(&mut settings.accent, DEFAULT_ACCENT)
                .preview(true)
                .show(ui);
            ui.end_row();
        });
    let changed = *settings != before;
    if changed {
        apply(ui.ctx(), settings);
    }
    // The picker says when its change is made; the other rows' changes are made at once.
    let others_changed = ThemeSettings {
        accent: before.accent,
        ..settings.clone()
    } != before;
    picked || others_changed
}

/// What a change does: install the new settings.
fn apply(ctx: &egui::Context, settings: &ThemeSettings) {
    super::install(ctx, settings);
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
    fn theme_labels_follow_the_language() {
        let ctx = egui::Context::default();
        crate::i18n::install(
            &ctx,
            &crate::i18n::LanguageSettings {
                language: crate::i18n::Language::Explicit("ja".into()),
            },
        );
        let mut settings = ThemeSettings::default();
        render_in(&ctx, [600.0, 400.0], |ui| {
            settings_ui(ui, &mut settings);
        });
        assert_eq!(crate::i18n::tr(&ctx, "corundum-theme-appearance"), "外観");
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
    fn the_accent_row_is_a_tint_picker() {
        let ctx = egui::Context::default();
        let mut settings = ThemeSettings::default();
        render_in(&ctx, [600.0, 600.0], |ui| {
            settings_ui(ui, &mut settings);
        });
        let rects = ctx.data(|d| {
            d.get_temp::<crate::theme::picker::PickerRects>(crate::theme::picker::PickerRects::id())
        });
        assert!(rects.is_some_and(|r| r.disk.is_positive()));
    }
}
