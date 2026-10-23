//! The theme's settings (`gui.theme.*`, stored by the bridge) as corundum's
//! [`ThemeSettings`], and back.

use corundum_gui::theme::{Accent, Contrast, ThemeSettings};
use egui::ThemePreference;
use sapphire_bridge_api::settings::gui;
use sapphire_bridge_api::{ConfigEntry, Value};

use crate::client::GuiValues;

/// The largest hue the settings hold: the theme widget keeps 360 itself.
const MAX_HUE: i64 = 360;
/// The largest chroma the settings hold, in thousandths.
const MAX_CHROMA: i64 = 250;

/// The theme `values` describe. A key that is missing, of the wrong kind or out of range
/// leaves that field at its default: a synced file may have been edited by hand.
pub(crate) fn theme_settings(values: &GuiValues) -> ThemeSettings {
    let mut theme = ThemeSettings::default();
    let text = |key| match values.get(key) {
        Some(Value::String(s)) => Some(s.as_str()),
        _ => None,
    };
    let int = |key, max| match values.get(key) {
        Some(Value::Int(i)) if (0..=max).contains(i) => Some(*i),
        _ => None,
    };
    if let Some(p) = text(gui::THEME_PREFERENCE).and_then(preference) {
        theme.preference = p;
    }
    if let Some(c) = text(gui::THEME_CONTRAST).and_then(contrast) {
        theme.contrast = c;
    }
    if let Some(hue) = int(gui::THEME_ACCENT_HUE, MAX_HUE) {
        theme.accent.hue = hue as f32;
    }
    if let Some(chroma) = int(gui::THEME_ACCENT_CHROMA, MAX_CHROMA) {
        theme.accent.chroma = chroma as f32 / 1000.0;
    }
    theme
}

/// `theme` as the four settings, in [`gui::THEME_KEYS`] order: the accent in whole degrees
/// and thousandths of chroma.
pub(crate) fn gui_entries(theme: &ThemeSettings) -> Vec<ConfigEntry> {
    let Accent { hue, chroma } = theme.accent;
    let entry = |key: &str, value| ConfigEntry {
        key: key.to_owned(),
        value,
    };
    vec![
        entry(
            gui::THEME_PREFERENCE,
            Value::String(preference_name(theme.preference).into()),
        ),
        entry(
            gui::THEME_CONTRAST,
            Value::String(contrast_name(theme.contrast).into()),
        ),
        entry(
            gui::THEME_ACCENT_HUE,
            Value::Int((hue.round() as i64).clamp(0, MAX_HUE)),
        ),
        entry(
            gui::THEME_ACCENT_CHROMA,
            Value::Int(((chroma * 1000.0).round() as i64).clamp(0, MAX_CHROMA)),
        ),
    ]
}

fn preference(name: &str) -> Option<ThemePreference> {
    match name {
        "system" => Some(ThemePreference::System),
        "light" => Some(ThemePreference::Light),
        "dark" => Some(ThemePreference::Dark),
        _ => None,
    }
}

fn preference_name(p: ThemePreference) -> &'static str {
    match p {
        ThemePreference::System => "system",
        ThemePreference::Light => "light",
        ThemePreference::Dark => "dark",
    }
}

fn contrast(name: &str) -> Option<Contrast> {
    match name {
        "low" => Some(Contrast::Low),
        "medium" => Some(Contrast::Medium),
        "high" => Some(Contrast::High),
        _ => None,
    }
}

fn contrast_name(c: Contrast) -> &'static str {
    match c {
        Contrast::Low => "low",
        Contrast::Medium => "medium",
        Contrast::High => "high",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(entries: Vec<ConfigEntry>) -> GuiValues {
        entries.into_iter().map(|e| (e.key, e.value)).collect()
    }

    fn one(key: &str, value: Value) -> GuiValues {
        GuiValues::from([(key.to_owned(), value)])
    }

    fn s(text: &str) -> Value {
        Value::String(text.into())
    }

    #[test]
    fn defaults_round_trip() {
        let theme = ThemeSettings::default();
        assert_eq!(theme_settings(&values(gui_entries(&theme))), theme);
        assert_eq!(theme_settings(&GuiValues::new()), theme);
    }

    #[test]
    fn each_key_maps_to_its_field() {
        let t = theme_settings(&one(gui::THEME_PREFERENCE, s("dark")));
        assert_eq!(t.preference, ThemePreference::Dark);
        let t = theme_settings(&one(gui::THEME_CONTRAST, s("low")));
        assert_eq!(t.contrast, Contrast::Low);
        let t = theme_settings(&one(gui::THEME_ACCENT_HUE, Value::Int(30)));
        assert_eq!(t.accent.hue, 30.0);
        let t = theme_settings(&one(gui::THEME_ACCENT_CHROMA, Value::Int(200)));
        assert!((t.accent.chroma - 0.2).abs() < 1e-6, "{}", t.accent.chroma);
    }

    #[test]
    fn a_bad_value_takes_the_defaults_field() {
        let defaults = ThemeSettings::default();
        let mut bad = GuiValues::from([
            (gui::THEME_ACCENT_HUE.to_owned(), Value::Int(999)),
            (gui::THEME_CONTRAST.to_owned(), s("loud")),
            (gui::THEME_ACCENT_CHROMA.to_owned(), s("x")),
        ]);
        bad.insert(gui::THEME_PREFERENCE.to_owned(), s("light"));
        let t = theme_settings(&bad);
        assert_eq!(t.accent, defaults.accent);
        assert_eq!(t.contrast, defaults.contrast);
        assert_eq!(t.preference, ThemePreference::Light);
    }

    #[test]
    fn the_accent_rounds() {
        let theme = ThemeSettings {
            accent: Accent {
                hue: 30.6,
                chroma: 0.1234,
            },
            ..Default::default()
        };
        let v = values(gui_entries(&theme));
        assert_eq!(v[gui::THEME_ACCENT_HUE], Value::Int(31));
        assert_eq!(v[gui::THEME_ACCENT_CHROMA], Value::Int(123));
    }

    #[test]
    fn entries_follow_the_keys_order() {
        let keys: Vec<String> = gui_entries(&ThemeSettings::default())
            .into_iter()
            .map(|e| e.key)
            .collect();
        assert_eq!(keys, gui::THEME_KEYS);
    }
}
