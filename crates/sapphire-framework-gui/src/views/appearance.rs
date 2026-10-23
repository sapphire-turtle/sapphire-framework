//! The Appearance page: the theme's settings (`gui.theme.*`, stored by the bridge) edited
//! with corundum's widget, per scope, and converted to and from [`ThemeSettings`].

use corundum_gui::theme::{self, Contrast, Swatch, ThemeSettings, Tint};
use egui::ThemePreference;
use sapphire_bridge_api::settings::{Entry, EntryValue, Origin, Scope, gui};
use sapphire_bridge_api::{ConfigEntry, Value};

use super::{error_text_line, register};
use crate::client::{Command, CommandError, CommandOutput, ConfigState, GuiValues};

/// What the bridge says when it does not know the theme's settings.
pub(crate) const OLD_BRIDGE: &str =
    "this sapphire-bridge does not know the appearance settings; update it";

/// What the Appearance page reads.
pub(crate) struct AppearanceCtx<'a> {
    /// The settings as last loaded.
    pub state: Option<&'a ConfigState>,
    /// Why the settings cannot be edited now; the widget is shown disabled.
    pub unavailable: Option<&'a str>,
    /// A write this page sent is still running.
    pub busy: bool,
    /// The bridge is in a workgroup: without one there is nothing to override, and no tabs.
    pub workgroup: bool,
}

/// The Appearance page: a tab per scope, and the theme widget over the tab's values. A
/// change is written as it is made — a click, or the end of a drag.
pub(crate) struct AppearancePage {
    /// The open tab.
    pub tab: Scope,
    /// What the widget edits.
    working: ThemeSettings,
    /// A drag is under way, or a write is out: keep `working` rather than the loaded values.
    editing: bool,
    /// A write was sent and has not answered.
    held: bool,
    /// Why the last write failed.
    pub error: Option<String>,
}

impl Default for AppearancePage {
    fn default() -> AppearancePage {
        AppearancePage {
            tab: Scope::Workgroup,
            working: ThemeSettings::default(),
            editing: false,
            held: false,
            error: None,
        }
    }
}

impl AppearancePage {
    /// Render; the write the user asked for, if any.
    pub(crate) fn ui(&mut self, ui: &mut egui::Ui, cx: &AppearanceCtx) -> Option<Command> {
        register(ui);
        ui.horizontal(|ui| {
            ui.heading("Appearance");
            if cx.busy {
                ui.spinner();
            }
        });
        ui.separator();
        let state = match (cx.state, cx.unavailable) {
            (Some(state), None) => state,
            (_, why) => {
                if let Some(why) = why {
                    ui.weak(why);
                }
                // Shown as it is, not editable.
                let mut shown = self.working.clone();
                ui.add_enabled_ui(false, |ui| theme::settings_ui(ui, &mut shown));
                return None;
            }
        };
        if !cx.workgroup {
            self.tab = Scope::Workgroup;
        } else {
            ui.horizontal(|ui| {
                for (scope, label) in [(Scope::Workgroup, "Workgroup"), (Scope::Device, "Device")] {
                    if ui
                        .add(egui::Button::selectable(self.tab == scope, label))
                        .clicked()
                        && self.tab != scope
                    {
                        self.tab = scope;
                        self.editing = false;
                    }
                }
            });
        }
        let mut out = None;
        if cx.workgroup {
            match self.tab {
                Scope::Device => {
                    for (label, names) in inherited(state) {
                        ui.weak(format!("{}: {label}", names.join(", ")));
                    }
                    if !overridden(state).is_empty()
                        && ui
                            .add_enabled(!cx.busy, egui::Button::new("Use the workgroup's"))
                            .clicked()
                    {
                        out = Some(use_the_workgroups());
                    }
                }
                _ => {
                    let own = overridden(state);
                    if !own.is_empty() {
                        ui.weak(format!("This device uses its own {}", own.join(", ")));
                    }
                }
            }
        }
        error_text_line(ui, &mut self.error);

        if !self.editing && !self.held {
            self.working = tab_theme(state, self.tab);
        }
        let before = self.working.clone();
        let done = ui
            .add_enabled_ui(!cx.busy, |ui| theme::settings_ui(ui, &mut self.working))
            .inner;
        if done {
            self.editing = false;
            if let Some(write) = write_for(state, self.tab, &self.working) {
                let look = expected_theme(state, self.tab, &self.working, cx.workgroup);
                theme::install(ui.ctx(), &look);
                self.held = true;
                out = Some(write);
            }
        } else if self.working != before {
            self.editing = true;
        } else if !ui.input(|i| i.pointer.any_down()) {
            self.editing = false;
        }
        out
    }

    /// Take a write's outcome; `true` when it failed, so the stored theme is shown again.
    /// A written value stays on the widget until [`reloaded`](Self::reloaded): until then
    /// the loaded settings are the ones from before the write.
    pub(crate) fn on_outcome(&mut self, result: &Result<CommandOutput, CommandError>) -> bool {
        self.editing = false;
        match result {
            Ok(_) => {
                self.error = None;
                false
            }
            Err(e) => {
                self.held = false;
                self.error = Some(e.message.clone());
                true
            }
        }
    }

    /// The settings were loaded again (or the load failed): show what they hold.
    pub(crate) fn reloaded(&mut self) {
        self.held = false;
    }
}

/// The theme keys `entries` give a value.
fn values_of(entries: &[Entry]) -> GuiValues {
    entries
        .iter()
        .filter(|e| gui::THEME_KEYS.contains(&e.key.as_str()))
        .filter_map(|e| match &e.value {
            EntryValue::Value(v) => Some((e.key.clone(), v.clone())),
            _ => None,
        })
        .collect()
}

/// The theme keys `scope`'s file sets.
fn scope_values(state: &ConfigState, scope: Scope) -> GuiValues {
    state
        .by_scope
        .get(&scope)
        .map(|entries| values_of(entries))
        .unwrap_or_default()
}

/// What the `tab` shows: the workgroup's values over the defaults, or on the device the
/// effective ones.
pub(crate) fn tab_theme(state: &ConfigState, tab: Scope) -> ThemeSettings {
    match tab {
        Scope::Device => theme_settings(&values_of(&state.effective)),
        _ => theme_settings(&scope_values(state, tab)),
    }
}

/// The write that stores `working` in `tab`: only the keys that differ from what the tab
/// shows. `None` when nothing does.
pub(crate) fn write_for(
    state: &ConfigState,
    tab: Scope,
    working: &ThemeSettings,
) -> Option<Command> {
    let shown = gui_entries(&tab_theme(state, tab));
    let set: Vec<ConfigEntry> = gui_entries(working)
        .into_iter()
        .filter(|new| {
            !shown
                .iter()
                .any(|old| old.key == new.key && old.value == new.value)
        })
        .collect();
    (!set.is_empty()).then_some(Command::ConfigWrite {
        scope: tab,
        set,
        unset: Vec::new(),
    })
}

/// The look once `working` is written to `tab`: on the workgroup's tab, the device's own
/// values still win — if there is a workgroup; without one, both scopes are one file.
pub(crate) fn expected_theme(
    state: &ConfigState,
    tab: Scope,
    working: &ThemeSettings,
    workgroup: bool,
) -> ThemeSettings {
    let mut values: GuiValues = gui_entries(working)
        .into_iter()
        .map(|e| (e.key, e.value))
        .collect();
    if tab == Scope::Workgroup && workgroup {
        values.extend(scope_values(state, Scope::Device));
    }
    theme_settings(&values)
}

/// The settings this device sets itself, by their names on the page.
pub(crate) fn overridden(state: &ConfigState) -> Vec<&'static str> {
    names(
        scope_values(state, Scope::Device)
            .keys()
            .map(String::as_str),
    )
}

/// The settings this device takes from elsewhere, by where: `from workgroup`, `default`.
fn inherited(state: &ConfigState) -> Vec<(&'static str, Vec<&'static str>)> {
    [
        (Origin::Scope(Scope::Workgroup), "from workgroup"),
        (Origin::Default, "default"),
    ]
    .into_iter()
    .filter_map(|(origin, label)| {
        let keys = state
            .effective
            .iter()
            .filter(|e| e.origin == origin && gui::THEME_KEYS.contains(&e.key.as_str()))
            .map(|e| e.key.as_str());
        let names = names(keys);
        (!names.is_empty()).then_some((label, names))
    })
    .collect()
}

/// The page's names for `keys`, each once, in the page's order.
fn names<'a>(keys: impl Iterator<Item = &'a str>) -> Vec<&'static str> {
    let keys: Vec<&str> = keys.collect();
    let mut out = Vec::new();
    for key in gui::THEME_KEYS {
        let name = match key {
            gui::THEME_PREFERENCE => "light/dark",
            gui::THEME_CONTRAST => "contrast",
            _ => "accent",
        };
        if keys.contains(&key) && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Drop every theme setting this device sets, so the workgroup's apply.
pub(crate) fn use_the_workgroups() -> Command {
    Command::ConfigWrite {
        scope: Scope::Device,
        set: Vec::new(),
        unset: gui::THEME_KEYS.iter().map(|k| (*k).to_owned()).collect(),
    }
}

/// The largest hue the settings hold: the theme widget keeps 360 itself.
const MAX_HUE: i64 = 360;
/// The largest chroma the settings hold, in thousandths.
const MAX_CHROMA: i64 = 250;
/// A custom accent's hue and chroma when the settings hold none: the bridge's defaults.
const DEFAULT_HUE: i64 = 255;
const DEFAULT_CHROMA: i64 = 150;

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
    theme.accent = match text(gui::THEME_ACCENT) {
        Some("custom") => Some(Tint::Custom {
            chroma: int(gui::THEME_ACCENT_CHROMA, MAX_CHROMA).unwrap_or(DEFAULT_CHROMA) as f32
                / 1000.0,
            hue: int(gui::THEME_ACCENT_HUE, MAX_HUE).unwrap_or(DEFAULT_HUE) as f32,
        }),
        Some(name) => swatch(name).map(Tint::Swatch),
        None => None,
    };
    theme
}

/// `theme` as settings, in [`gui::THEME_KEYS`] order. A custom accent adds its hue in whole
/// degrees and its chroma in thousandths; any other leaves them out, so they keep what they
/// hold.
pub(crate) fn gui_entries(theme: &ThemeSettings) -> Vec<ConfigEntry> {
    let entry = |key: &str, value| ConfigEntry {
        key: key.to_owned(),
        value,
    };
    let accent = match theme.accent {
        Some(Tint::Custom { .. }) => "custom",
        Some(Tint::Swatch(s)) => swatch_name(s),
        None => "default",
    };
    let mut entries = vec![
        entry(
            gui::THEME_PREFERENCE,
            Value::String(preference_name(theme.preference).into()),
        ),
        entry(
            gui::THEME_CONTRAST,
            Value::String(contrast_name(theme.contrast).into()),
        ),
        entry(gui::THEME_ACCENT, Value::String(accent.into())),
    ];
    if let Some(Tint::Custom { chroma, hue }) = theme.accent {
        entries.push(entry(
            gui::THEME_ACCENT_HUE,
            Value::Int((hue.round() as i64).clamp(0, MAX_HUE)),
        ));
        entries.push(entry(
            gui::THEME_ACCENT_CHROMA,
            Value::Int(((chroma * 1000.0).round() as i64).clamp(0, MAX_CHROMA)),
        ));
    }
    entries
}

/// The swatch an accent's value names; `None` for `default` and anything unknown.
fn swatch(name: &str) -> Option<Swatch> {
    Swatch::ALL
        .into_iter()
        .filter(|s| *s != Swatch::Accent)
        .find(|s| swatch_name(*s) == name)
}

/// A swatch's name in the settings. The accent swatch would name itself: it is the default.
fn swatch_name(swatch: Swatch) -> &'static str {
    match swatch {
        Swatch::Accent => "default",
        Swatch::Neutral => "neutral",
        Swatch::Red => "red",
        Swatch::Orange => "orange",
        Swatch::Yellow => "yellow",
        Swatch::Green => "green",
        Swatch::Cyan => "cyan",
        Swatch::Blue => "blue",
        Swatch::Purple => "purple",
    }
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
        let t = theme_settings(&one(gui::THEME_ACCENT, s("red")));
        assert_eq!(t.accent, Some(Tint::Swatch(Swatch::Red)));
        let t = theme_settings(&GuiValues::from([
            (gui::THEME_ACCENT.to_owned(), s("custom")),
            (gui::THEME_ACCENT_HUE.to_owned(), Value::Int(30)),
            (gui::THEME_ACCENT_CHROMA.to_owned(), Value::Int(200)),
        ]));
        assert_eq!(
            t.accent,
            Some(Tint::Custom {
                chroma: 0.2,
                hue: 30.0
            })
        );
    }

    #[test]
    fn a_hue_without_a_custom_accent_is_not_used() {
        let t = theme_settings(&one(gui::THEME_ACCENT_HUE, Value::Int(30)));
        assert_eq!(t.accent, None);
    }

    #[test]
    fn a_bad_value_takes_the_defaults_field() {
        let defaults = ThemeSettings::default();
        let mut bad = GuiValues::from([
            (gui::THEME_ACCENT.to_owned(), s("mauve")),
            (gui::THEME_CONTRAST.to_owned(), s("loud")),
        ]);
        bad.insert(gui::THEME_PREFERENCE.to_owned(), s("light"));
        let t = theme_settings(&bad);
        assert_eq!(t.accent, defaults.accent);
        assert_eq!(t.contrast, defaults.contrast);
        assert_eq!(t.preference, ThemePreference::Light);
    }

    #[test]
    fn a_custom_accent_with_bad_numbers_takes_the_default_numbers() {
        let t = theme_settings(&GuiValues::from([
            (gui::THEME_ACCENT.to_owned(), s("custom")),
            (gui::THEME_ACCENT_HUE.to_owned(), Value::Int(999)),
            (gui::THEME_ACCENT_CHROMA.to_owned(), s("x")),
        ]));
        assert_eq!(
            t.accent,
            Some(Tint::Custom {
                chroma: 0.15,
                hue: 255.0
            })
        );
    }

    #[test]
    fn the_accent_rounds() {
        let theme = ThemeSettings {
            accent: Some(Tint::Custom {
                hue: 30.6,
                chroma: 0.1234,
            }),
            ..Default::default()
        };
        let v = values(gui_entries(&theme));
        assert_eq!(v[gui::THEME_ACCENT], s("custom"));
        assert_eq!(v[gui::THEME_ACCENT_HUE], Value::Int(31));
        assert_eq!(v[gui::THEME_ACCENT_CHROMA], Value::Int(123));
    }

    #[test]
    fn a_swatch_accent_leaves_the_numbers_out() {
        let theme = ThemeSettings {
            accent: Some(Tint::Swatch(Swatch::Green)),
            ..Default::default()
        };
        let v = values(gui_entries(&theme));
        assert_eq!(v[gui::THEME_ACCENT], s("green"));
        assert!(!v.contains_key(gui::THEME_ACCENT_HUE), "{v:?}");
        assert!(!v.contains_key(gui::THEME_ACCENT_CHROMA), "{v:?}");
        assert_eq!(theme_settings(&v), theme);
    }

    #[test]
    fn entries_follow_the_keys_order() {
        let theme = ThemeSettings {
            accent: Some(Tint::Custom {
                hue: 10.0,
                chroma: 0.1,
            }),
            ..Default::default()
        };
        let keys: Vec<String> = gui_entries(&theme).into_iter().map(|e| e.key).collect();
        assert_eq!(keys, gui::THEME_KEYS);
    }

    // ── the page ────────────────────────────────────────────────────────────

    use crate::views::config::widgets::tests::drawn_text;

    fn entry(key: &str, value: Value, origin: Origin) -> Entry {
        Entry {
            key: key.into(),
            value: EntryValue::Value(value),
            origin,
            pending_restart: false,
            note: None,
        }
    }

    /// The workgroup sets `workgroup`, the device `device`; the effective values follow.
    fn state(workgroup: &[(&str, Value)], device: &[(&str, Value)]) -> ConfigState {
        let at = |scope, items: &[(&str, Value)]| -> Vec<Entry> {
            items
                .iter()
                .map(|(k, v)| entry(k, v.clone(), Origin::Scope(scope)))
                .collect()
        };
        let mut effective = Vec::new();
        for e in gui_entries(&ThemeSettings::default()) {
            let from = |items: &[(&str, Value)]| {
                items
                    .iter()
                    .find(|(k, _)| *k == e.key)
                    .map(|(_, v)| v.clone())
            };
            effective.push(match (from(device), from(workgroup)) {
                (Some(v), _) => entry(&e.key, v, Origin::Scope(Scope::Device)),
                (None, Some(v)) => entry(&e.key, v, Origin::Scope(Scope::Workgroup)),
                (None, None) => entry(&e.key, e.value, Origin::Default),
            });
        }
        ConfigState {
            schema: Vec::new(),
            effective,
            by_scope: [
                (Scope::Workgroup, at(Scope::Workgroup, workgroup)),
                (Scope::Device, at(Scope::Device, device)),
            ]
            .into(),
        }
    }

    fn high() -> ThemeSettings {
        ThemeSettings {
            contrast: Contrast::High,
            ..Default::default()
        }
    }

    /// A write's scope, the keys and values it sets, and the keys it unsets.
    type Write = (Scope, Vec<(String, Value)>, Vec<String>);

    fn write(command: Option<Command>) -> Option<Write> {
        match command? {
            Command::ConfigWrite { scope, set, unset } => Some((
                scope,
                set.into_iter().map(|e| (e.key, e.value)).collect(),
                unset,
            )),
            other => panic!("not a write: {other:?}"),
        }
    }

    #[test]
    fn a_change_writes_only_the_changed_keys() {
        let st = state(&[], &[]);
        assert_eq!(
            write(write_for(&st, Scope::Workgroup, &high())),
            Some((
                Scope::Workgroup,
                vec![(gui::THEME_CONTRAST.to_owned(), s("high"))],
                Vec::new()
            ))
        );
        assert_eq!(
            write(write_for(&st, Scope::Workgroup, &ThemeSettings::default())),
            None
        );
    }

    fn custom(hue: f32, chroma: f32) -> ThemeSettings {
        ThemeSettings {
            accent: Some(Tint::Custom { chroma, hue }),
            ..Default::default()
        }
    }

    #[test]
    fn a_swatch_after_a_custom_accent_writes_only_the_accent() {
        let st = state(
            &[
                (gui::THEME_ACCENT, s("custom")),
                (gui::THEME_ACCENT_HUE, Value::Int(30)),
                (gui::THEME_ACCENT_CHROMA, Value::Int(100)),
            ],
            &[],
        );
        assert_eq!(tab_theme(&st, Scope::Workgroup), custom(30.0, 0.1));
        let red = ThemeSettings {
            accent: Some(Tint::Swatch(Swatch::Red)),
            ..Default::default()
        };
        assert_eq!(
            write(write_for(&st, Scope::Workgroup, &red)),
            Some((
                Scope::Workgroup,
                vec![(gui::THEME_ACCENT.to_owned(), s("red"))],
                Vec::new()
            ))
        );
    }

    #[test]
    fn a_custom_accent_writes_its_numbers() {
        let st = state(&[], &[]);
        assert_eq!(
            write(write_for(&st, Scope::Workgroup, &custom(30.0, 0.1))),
            Some((
                Scope::Workgroup,
                vec![
                    (gui::THEME_ACCENT.to_owned(), s("custom")),
                    (gui::THEME_ACCENT_HUE.to_owned(), Value::Int(30)),
                    (gui::THEME_ACCENT_CHROMA.to_owned(), Value::Int(100)),
                ],
                Vec::new()
            ))
        );
    }

    #[test]
    fn the_device_tab_starts_from_the_effective_values() {
        let st = state(&[(gui::THEME_CONTRAST, s("high"))], &[]);
        assert_eq!(tab_theme(&st, Scope::Device), high());
        // Unchanged on the device tab: nothing to write, so no override appears.
        assert_eq!(write(write_for(&st, Scope::Device, &high())), None);
    }

    #[test]
    fn use_the_workgroups_unsets_every_key_on_the_device() {
        assert_eq!(
            write(Some(use_the_workgroups())),
            Some((
                Scope::Device,
                Vec::new(),
                gui::THEME_KEYS.iter().map(|k| (*k).to_owned()).collect()
            ))
        );
    }

    #[test]
    fn an_overridden_key_keeps_the_devices_look() {
        let st = state(&[], &[(gui::THEME_CONTRAST, s("low"))]);
        let shown = expected_theme(&st, Scope::Workgroup, &high(), true);
        assert_eq!(shown.contrast, Contrast::Low);
        // On the device tab the edit is what shows.
        let shown = expected_theme(&st, Scope::Device, &high(), true);
        assert_eq!(shown.contrast, Contrast::High);
    }

    #[test]
    fn overridden_names_each_setting_once() {
        let st = state(
            &[],
            &[
                (gui::THEME_ACCENT_HUE, Value::Int(10)),
                (gui::THEME_ACCENT_CHROMA, Value::Int(10)),
                (gui::THEME_CONTRAST, s("low")),
            ],
        );
        assert_eq!(overridden(&st), ["contrast", "accent"]);
    }

    fn drawn(page: &mut AppearancePage, cx: &AppearanceCtx) -> String {
        drawn_text(|ui| {
            page.ui(ui, cx);
        })
    }

    #[test]
    fn the_override_line_names_the_overridden() {
        let st = state(&[], &[(gui::THEME_CONTRAST, s("low"))]);
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: true,
        };
        let text = drawn(&mut AppearancePage::default(), &cx);
        assert!(text.contains("Workgroup"), "{text}");
        assert!(text.contains("Device"), "{text}");
        assert!(text.contains("This device uses its own contrast"), "{text}");
    }

    #[test]
    fn the_device_tab_says_what_it_takes_from_the_workgroup() {
        let st = state(
            &[(gui::THEME_PREFERENCE, s("dark"))],
            &[(gui::THEME_CONTRAST, s("low"))],
        );
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: true,
        };
        let mut page = AppearancePage {
            tab: Scope::Device,
            ..Default::default()
        };
        let text = drawn(&mut page, &cx);
        assert!(text.contains("light/dark: from workgroup"), "{text}");
        assert!(text.contains("accent: default"), "{text}");
        assert!(text.contains("Use the workgroup's"), "{text}");
    }

    #[test]
    fn unavailable_draws_the_reason() {
        let cx = AppearanceCtx {
            state: None,
            unavailable: Some("sapphire-bridge is not running"),
            busy: false,
            workgroup: true,
        };
        let text = drawn(&mut AppearancePage::default(), &cx);
        assert!(text.contains("sapphire-bridge is not running"), "{text}");
    }

    #[test]
    fn a_failed_write_says_so() {
        let mut page = AppearancePage::default();
        assert!(page.on_outcome(&Err("nope".into())));
        assert_eq!(page.error.as_deref(), Some("nope"));
        assert!(!page.on_outcome(&Ok(crate::client::CommandOutput::Done)));
    }

    #[test]
    fn without_a_workgroup_there_are_no_tabs() {
        // No workgroup: the bridge keeps the workgroup's keys in the device file, so the
        // device "sets" them all; the page must not call that an override.
        let st = state(&[], &[(gui::THEME_CONTRAST, s("low"))]);
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: false,
        };
        let mut page = AppearancePage {
            tab: Scope::Device,
            ..Default::default()
        };
        let text = drawn(&mut page, &cx);
        assert!(!text.contains("Workgroup"), "{text}");
        assert!(!text.contains("uses its own"), "{text}");
        assert_eq!(page.tab, Scope::Workgroup);
    }

    #[test]
    fn without_a_workgroup_the_device_file_is_not_an_overlay() {
        // The same file answers for both scopes: its old values must not mask the edit.
        let st = state(
            &[(gui::THEME_CONTRAST, s("low"))],
            &[(gui::THEME_CONTRAST, s("low"))],
        );
        let shown = expected_theme(&st, Scope::Workgroup, &high(), false);
        assert_eq!(shown.contrast, Contrast::High);
        let shown = expected_theme(&st, Scope::Workgroup, &high(), true);
        assert_eq!(shown.contrast, Contrast::Low);
    }

    #[test]
    fn a_written_value_is_kept_until_the_reload() {
        let st = state(&[], &[]);
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: true,
        };
        let mut page = AppearancePage {
            working: high(),
            held: true,
            ..Default::default()
        };
        page.on_outcome(&Ok(crate::client::CommandOutput::Done));
        // The answer is in, the reload is not: `st` is still the state before the write.
        drawn(&mut page, &cx);
        assert_eq!(page.working, high());
        page.reloaded();
        drawn(&mut page, &cx);
        assert_eq!(page.working, ThemeSettings::default());
    }

    #[test]
    fn a_failed_write_shows_the_stored_values_again() {
        let st = state(&[], &[]);
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: true,
        };
        let mut page = AppearancePage {
            working: high(),
            held: true,
            ..Default::default()
        };
        page.on_outcome(&Err("rejected".into()));
        drawn(&mut page, &cx);
        assert_eq!(page.working, ThemeSettings::default());
    }
}
