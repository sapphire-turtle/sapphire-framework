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

use std::sync::Arc;

mod color;
mod palette;
mod picker;
mod tint;
mod ui;
mod visuals;

pub use palette::{Swatch, Theme, Tone};
pub use picker::TintPicker;
pub use tint::Tint;
pub use ui::settings_ui;

/// How far apart the text and the background are.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Contrast {
    /// Body text at WCAG AA (4.5:1), quieter edges.
    Low,
    /// Body text at WCAG AAA (7:1).
    #[default]
    Medium,
    /// Body text at 12:1 and up.
    High,
}

/// What the user chose: what the app saves, and hands to [`install`] at start.
#[derive(Clone, PartialEq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ThemeSettings {
    /// Light, dark, or whichever the system uses.
    pub preference: egui::ThemePreference,
    pub contrast: Contrast,
    /// The accent; `None` is the default, a blue.
    pub accent: Option<Tint>,
}

impl ThemeSettings {
    /// The accent, the default in place of `None`.
    pub fn accent(&self) -> Tint {
        self.accent.unwrap_or(DEFAULT_ACCENT)
    }
}

/// The accent when the user has not picked one.
pub(crate) const DEFAULT_ACCENT: Tint = Tint::Swatch(Swatch::Blue);

/// The light and the dark theme, as [`install`] left them.
#[derive(Clone)]
struct Installed {
    light: Arc<Theme>,
    dark: Arc<Theme>,
}

fn key() -> egui::Id {
    egui::Id::new("corundum_theme")
}

/// Use `settings` in `ctx`: build the light and the dark theme, give egui both looks, and
/// follow `settings.preference`. Call it at start and whenever the settings change.
pub fn install(ctx: &egui::Context, settings: &ThemeSettings) {
    let build = |dark| Arc::new(Theme::new(dark, settings.contrast, settings.accent()));
    let installed = Installed {
        light: build(false),
        dark: build(true),
    };
    ctx.set_visuals_of(egui::Theme::Light, installed.light.visuals());
    ctx.set_visuals_of(egui::Theme::Dark, installed.dark.visuals());
    ctx.set_theme(settings.preference);
    ctx.data_mut(|d| d.insert_temp(key(), installed));
}

/// Whether [`install`] has been called on `ctx`.
pub fn is_installed(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Installed>(key())).is_some()
}

/// The theme `ctx` draws with now: the dark or the light one, as egui's theme is. Before
/// [`install`], the default settings' (egui's own look is left as it is).
pub fn current(ctx: &egui::Context) -> Arc<Theme> {
    let dark = ctx.theme() == egui::Theme::Dark;
    match ctx.data(|d| d.get_temp::<Installed>(key())) {
        Some(installed) if dark => installed.dark,
        Some(installed) => installed.light,
        None => Arc::new(Theme::new(dark, Contrast::default(), DEFAULT_ACCENT)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(a: &Theme, b: &Theme) -> bool {
        a == b
    }

    #[test]
    fn install_follows_the_egui_theme() {
        let ctx = egui::Context::default();
        let settings = ThemeSettings {
            preference: egui::ThemePreference::Dark,
            ..Default::default()
        };
        install(&ctx, &settings);
        assert!(current(&ctx).is_dark());
        assert_eq!(
            ctx.global_style().visuals.panel_fill,
            current(&ctx).color(Tone::Base, Swatch::Neutral)
        );
        ctx.set_theme(egui::Theme::Light);
        assert!(!current(&ctx).is_dark());
        assert_eq!(
            ctx.global_style().visuals.panel_fill,
            current(&ctx).color(Tone::Base, Swatch::Neutral)
        );
    }

    #[test]
    fn system_preference_still_gives_a_theme() {
        let ctx = egui::Context::default();
        install(&ctx, &ThemeSettings::default());
        assert_eq!(current(&ctx).is_dark(), ctx.theme() == egui::Theme::Dark);
    }

    #[test]
    fn reinstall_replaces_both() {
        let ctx = egui::Context::default();
        install(&ctx, &ThemeSettings::default());
        let accent = Tint::Custom {
            chroma: 0.1,
            hue: 30.0,
        };
        install(
            &ctx,
            &ThemeSettings {
                contrast: Contrast::High,
                accent: Some(accent),
                ..Default::default()
            },
        );
        for (theme, dark) in [(egui::Theme::Dark, true), (egui::Theme::Light, false)] {
            ctx.set_theme(theme);
            assert!(same(
                &current(&ctx),
                &Theme::new(dark, Contrast::High, accent)
            ));
        }
    }

    #[test]
    fn before_install_current_is_the_default() {
        let ctx = egui::Context::default();
        let visuals = ctx.global_style().visuals.clone();
        assert!(!is_installed(&ctx));
        let dark = ctx.theme() == egui::Theme::Dark;
        assert!(same(
            &current(&ctx),
            &Theme::new(dark, Contrast::Medium, Tint::Swatch(Swatch::Blue))
        ));
        assert_eq!(ctx.global_style().visuals, visuals);
    }

    #[test]
    fn none_accent_is_blue() {
        let settings = ThemeSettings::default();
        assert_eq!(settings.accent, None);
        assert_eq!(settings.accent(), Tint::Swatch(Swatch::Blue));
        let ctx = egui::Context::default();
        install(&ctx, &settings);
        let t = current(&ctx);
        for tone in Tone::ALL {
            assert_eq!(t.color(tone, Swatch::Accent), t.color(tone, Swatch::Blue));
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn settings_with_no_accent_round_trip() {
        let settings = ThemeSettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let back: ThemeSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back, settings);
        assert_eq!(back.accent(), Tint::Swatch(Swatch::Blue));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn settings_round_trip() {
        let settings = ThemeSettings {
            preference: egui::ThemePreference::Light,
            contrast: Contrast::Low,
            accent: Some(Tint::Custom {
                chroma: 0.2,
                hue: 140.0,
            }),
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<ThemeSettings>(&json).unwrap(),
            settings
        );
    }
}
