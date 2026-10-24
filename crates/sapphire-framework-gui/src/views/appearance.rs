//! The Appearance page: the theme's settings (`gui.theme.*`) and the language
//! (`gui.locale`), stored by the bridge, edited with corundum's widgets, per scope, and
//! converted to and from [`ThemeSettings`] and [`LanguageSettings`].

use corundum_gui::i18n::{self, Language, LanguageIdentifier, LanguageSettings};
use corundum_gui::theme::{self, Contrast, Swatch, ThemeSettings, Tint};
use egui::ThemePreference;
use sapphire_bridge_api::settings::{Entry, EntryValue, Origin, Scope, gui};
use sapphire_bridge_api::{ConfigEntry, Value};

use super::{error_text_line, register};
use crate::client::{Command, CommandError, CommandOutput, ConfigState, GuiValues};
use crate::i18n::Message;
use corundum_gui::tr;

/// What the page says when the bridge does not know the theme's settings.
pub(crate) fn old_bridge() -> Message {
    Message::new("sapphire-appearance-old-bridge")
}

/// What the page says when the bridge knows the theme's settings but not the language.
pub(crate) fn old_bridge_language() -> Message {
    Message::new("sapphire-appearance-old-bridge-language")
}

/// Every key the page edits: the theme's, then the language.
const KEYS: [&str; 6] = [
    gui::THEME_PREFERENCE,
    gui::THEME_CONTRAST,
    gui::THEME_ACCENT,
    gui::THEME_ACCENT_HUE,
    gui::THEME_ACCENT_CHROMA,
    gui::LOCALE,
];

/// What the page says when there is no bridge to ask.
pub(crate) fn bridge_down() -> Message {
    Message::new("sapphire-bridge-not-running")
}

/// What the Appearance page reads.
pub(crate) struct AppearanceCtx<'a> {
    /// The settings as last loaded.
    pub state: Option<&'a ConfigState>,
    /// Why the settings cannot be edited now; the widget is shown disabled.
    pub unavailable: Option<Message>,
    /// A write this page sent is still running.
    pub busy: bool,
    /// The bridge is in a workgroup: without one there is nothing to override, and no tabs.
    pub workgroup: bool,
    /// The bridge knows the language (`gui.locale`); an older one does not, and the
    /// picker is shown disabled.
    pub language: bool,
}

/// The Appearance page: a tab per scope, and the theme widget and the language picker over
/// the tab's values. A change is written as it is made — a click, a choice, or the end of a
/// drag.
pub(crate) struct AppearancePage {
    /// The open tab.
    pub tab: Scope,
    /// What the widget edits.
    working: ThemeSettings,
    /// What the language picker edits.
    language: LanguageSettings,
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
            language: LanguageSettings::default(),
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
            ui.heading(tr!(ui, "sapphire-settings-appearance"));
            if cx.busy {
                ui.spinner();
            }
        });
        ui.separator();
        let state = match (cx.state, cx.unavailable.as_ref()) {
            (Some(state), None) => state,
            (_, why) => {
                if let Some(why) = why {
                    ui.weak(why.text(ui.ctx()));
                }
                // Shown as it is, not editable.
                let mut shown = self.working.clone();
                let mut language = self.language.clone();
                ui.add_enabled_ui(false, |ui| {
                    theme::settings_ui(ui, &mut shown);
                    language_row(ui, &mut language);
                });
                return None;
            }
        };
        if !cx.workgroup {
            self.tab = Scope::Workgroup;
        } else {
            ui.horizontal(|ui| {
                for scope in [Scope::Workgroup, Scope::Device] {
                    let label = tr!(ui, scope_id(scope));
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
                    for (origin, names) in inherited(state) {
                        let names = joined(ui, &names);
                        ui.weak(tr!(
                            ui,
                            "sapphire-appearance-inherited",
                            names = names,
                            origin = tr!(ui, origin)
                        ));
                    }
                    if !overridden(state).is_empty()
                        && ui
                            .add_enabled(
                                !cx.busy,
                                egui::Button::new(tr!(ui, "sapphire-appearance-use-workgroup")),
                            )
                            .clicked()
                    {
                        out = Some(use_the_workgroups(state));
                    }
                }
                _ => {
                    let own = overridden(state);
                    if !own.is_empty() {
                        let names = joined(ui, &own);
                        ui.weak(tr!(ui, "sapphire-appearance-own", names = names));
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
                out = merged(out, Some(write));
            }
        } else if self.working != before {
            self.editing = true;
        } else if !ui.input(|i| i.pointer.any_down()) {
            self.editing = false;
        }
        ui.add_space(8.0);
        let language = self.language_ui(ui, state, cx);
        merged(out, language)
    }

    /// The language picker over the tab's value; the write a choice asks for.
    fn language_ui(
        &mut self,
        ui: &mut egui::Ui,
        state: &ConfigState,
        cx: &AppearanceCtx,
    ) -> Option<Command> {
        if !self.held {
            self.language = tab_language(state, self.tab);
        }
        if !cx.language {
            ui.weak(old_bridge_language().text(ui.ctx()));
        }
        let changed = ui
            .add_enabled_ui(!cx.busy && cx.language, |ui| {
                language_row(ui, &mut self.language)
            })
            .inner;
        if !changed {
            return None;
        }
        self.language_chosen(ui.ctx(), state, cx)
    }

    /// The picker chose `self.language` (and installed it): install the language the page
    /// will show once it is written — the device's own still wins on the workgroup's tab —
    /// and give the write.
    fn language_chosen(
        &mut self,
        ctx: &egui::Context,
        state: &ConfigState,
        cx: &AppearanceCtx,
    ) -> Option<Command> {
        let write = locale_write(state, self.tab, &self.language);
        i18n::install(
            ctx,
            &expected_language(state, self.tab, &self.language, cx.workgroup),
        );
        if write.is_some() {
            self.held = true;
        }
        write
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

/// A row of the language picker, aligned as the theme widget's rows; `true` on a choice.
fn language_row(ui: &mut egui::Ui, language: &mut LanguageSettings) -> bool {
    egui::Grid::new("sapphire_language_row")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label(tr!(ui, "corundum-language"));
            let changed = i18n::settings_ui(ui, language);
            ui.end_row();
            changed
        })
        .inner
}

/// The page's keys `entries` give a value.
fn values_of(entries: &[Entry]) -> GuiValues {
    entries
        .iter()
        .filter(|e| KEYS.contains(&e.key.as_str()))
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

/// The message naming `scope`.
pub(crate) fn scope_id(scope: Scope) -> &'static str {
    match scope {
        Scope::Device => "sapphire-scope-device",
        Scope::Workgroup => "sapphire-scope-workgroup",
        Scope::Workspace => "sapphire-scope-workspace",
    }
}

/// The language the `tab` shows, as [`tab_theme`] does the theme.
pub(crate) fn tab_language(state: &ConfigState, tab: Scope) -> LanguageSettings {
    match tab {
        Scope::Device => language_settings(&values_of(&state.effective)),
        _ => language_settings(&scope_values(state, tab)),
    }
}

/// The write that stores `language` in `tab`: `gui.locale` alone, and only when it differs
/// from what the tab shows.
pub(crate) fn locale_write(
    state: &ConfigState,
    tab: Scope,
    language: &LanguageSettings,
) -> Option<Command> {
    let entry = locale_entry(language);
    (entry != locale_entry(&tab_language(state, tab))).then(|| Command::ConfigWrite {
        scope: tab,
        set: vec![entry],
        unset: Vec::new(),
    })
}

/// The language once `language` is written to `tab`: on the workgroup's tab, the device's
/// own still wins — if there is a workgroup.
pub(crate) fn expected_language(
    state: &ConfigState,
    tab: Scope,
    language: &LanguageSettings,
    workgroup: bool,
) -> LanguageSettings {
    let device = scope_values(state, Scope::Device);
    if tab == Scope::Workgroup && workgroup && device.contains_key(gui::LOCALE) {
        language_settings(&device)
    } else {
        language.clone()
    }
}

/// The page's names (message ids) resolved and joined as a list.
fn joined(ui: &egui::Ui, ids: &[&str]) -> String {
    let separator = tr!(ui, "sapphire-list-separator");
    ids.iter()
        .map(|id| tr!(ui, id))
        .collect::<Vec<_>>()
        .join(&separator)
}

/// The settings this device sets itself, by their names on the page (message ids).
pub(crate) fn overridden(state: &ConfigState) -> Vec<&'static str> {
    names(
        scope_values(state, Scope::Device)
            .keys()
            .map(String::as_str),
    )
}

/// The settings this device takes from elsewhere, by where (the origin's message id) and
/// their names on the page (message ids).
fn inherited(state: &ConfigState) -> Vec<(&'static str, Vec<&'static str>)> {
    [
        (Origin::Scope(Scope::Workgroup), "sapphire-origin-workgroup"),
        (Origin::Default, "sapphire-origin-default"),
    ]
    .into_iter()
    .filter_map(|(origin, label)| {
        let keys = state
            .effective
            .iter()
            .filter(|e| e.origin == origin && KEYS.contains(&e.key.as_str()))
            .map(|e| e.key.as_str());
        let names = names(keys);
        (!names.is_empty()).then_some((label, names))
    })
    .collect()
}

/// The page's names (message ids) for `keys`, each once, in the page's order.
fn names<'a>(keys: impl Iterator<Item = &'a str>) -> Vec<&'static str> {
    let keys: Vec<&str> = keys.collect();
    let mut out = Vec::new();
    for key in KEYS {
        let name = match key {
            gui::THEME_PREFERENCE => "sapphire-appearance-name-theme",
            gui::THEME_CONTRAST => "sapphire-appearance-name-contrast",
            gui::LOCALE => "sapphire-appearance-name-language",
            _ => "sapphire-appearance-name-accent",
        };
        if keys.contains(&key) && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Drop every theme setting and the language this device sets, so the workgroup's apply.
/// Only the keys the device sets: a bridge that does not know `gui.locale` would refuse the
/// whole write over it.
pub(crate) fn use_the_workgroups(state: &ConfigState) -> Command {
    let own = scope_values(state, Scope::Device);
    Command::ConfigWrite {
        scope: Scope::Device,
        set: Vec::new(),
        unset: KEYS
            .iter()
            .filter(|k| own.contains_key(**k))
            .map(|k| (*k).to_owned())
            .collect(),
    }
}

/// Two writes the page asked for in one frame, as one: a choice of the language and the end
/// of a theme drag, say. Both are to the open tab's scope; a key set by either is not also
/// unset. Writes to different scopes cannot happen; the first would be kept.
fn merged(a: Option<Command>, b: Option<Command>) -> Option<Command> {
    match (a, b) {
        (
            Some(Command::ConfigWrite {
                scope,
                mut set,
                mut unset,
            }),
            Some(Command::ConfigWrite {
                scope: other,
                set: more_set,
                unset: more_unset,
            }),
        ) if scope == other => {
            set.extend(more_set);
            unset.extend(more_unset);
            unset.retain(|k| !set.iter().any(|e| e.key == *k));
            unset.dedup();
            Some(Command::ConfigWrite { scope, set, unset })
        }
        (a, b) => a.or(b),
    }
}

/// The value of `gui.locale` that follows the system's languages.
const SYSTEM_LOCALE: &str = "system";

/// The language `values` hold in `gui.locale`: `system`, or a language tag. A missing
/// value, or one that is not a language tag, follows the system: a synced file may have been
/// edited by hand.
pub(crate) fn language_settings(values: &GuiValues) -> LanguageSettings {
    let language = match values.get(gui::LOCALE) {
        Some(Value::String(tag))
            if !tag.trim().eq_ignore_ascii_case(SYSTEM_LOCALE) && !tag.trim().is_empty() =>
        {
            tag.trim()
                .parse::<LanguageIdentifier>()
                .map_or(Language::System, |id| Language::Explicit(id.to_string()))
        }
        _ => Language::System,
    };
    LanguageSettings { language }
}

/// `settings` as the `gui.locale` setting.
pub(crate) fn locale_entry(settings: &LanguageSettings) -> ConfigEntry {
    let value = match &settings.language {
        Language::System => SYSTEM_LOCALE.to_owned(),
        Language::Explicit(tag) => tag.clone(),
    };
    ConfigEntry {
        key: gui::LOCALE.to_owned(),
        value: Value::String(value),
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
        let defaults = gui_entries(&ThemeSettings::default())
            .into_iter()
            .chain([locale_entry(&LanguageSettings::default())]);
        for e in defaults {
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
        let all = [
            (gui::THEME_PREFERENCE, s("dark")),
            (gui::THEME_CONTRAST, s("low")),
            (gui::THEME_ACCENT, s("custom")),
            (gui::THEME_ACCENT_HUE, Value::Int(10)),
            (gui::THEME_ACCENT_CHROMA, Value::Int(10)),
            (gui::LOCALE, s("ja")),
        ];
        assert_eq!(
            write(Some(use_the_workgroups(&state(&[], &all)))),
            Some((
                Scope::Device,
                Vec::new(),
                [
                    gui::THEME_PREFERENCE,
                    gui::THEME_CONTRAST,
                    gui::THEME_ACCENT,
                    gui::THEME_ACCENT_HUE,
                    gui::THEME_ACCENT_CHROMA,
                    gui::LOCALE,
                ]
                .iter()
                .map(|k| (*k).to_owned())
                .collect()
            ))
        );
    }

    #[test]
    fn use_the_workgroups_unsets_only_what_the_device_sets() {
        // A bridge without `gui.locale` (or a device without its own) never sees it unset:
        // the bridge would refuse the whole write over an unknown key.
        let st = state(&[], &[(gui::THEME_CONTRAST, s("low"))]);
        assert_eq!(
            write(Some(use_the_workgroups(&st))),
            Some((
                Scope::Device,
                Vec::new(),
                vec![gui::THEME_CONTRAST.to_owned()]
            ))
        );
    }

    #[test]
    fn writes_in_one_frame_are_merged() {
        let theme = Some(Command::ConfigWrite {
            scope: Scope::Device,
            set: vec![ConfigEntry {
                key: gui::THEME_CONTRAST.into(),
                value: s("high"),
            }],
            unset: Vec::new(),
        });
        let language = locale_write(&state(&[], &[]), Scope::Device, &ja());
        assert_eq!(
            write(merged(theme, language)),
            Some((
                Scope::Device,
                vec![
                    (gui::THEME_CONTRAST.to_owned(), s("high")),
                    (gui::LOCALE.to_owned(), s("ja")),
                ],
                Vec::new()
            ))
        );
        let unset = Some(use_the_workgroups(&state(
            &[],
            &[(gui::THEME_CONTRAST, s("low")), (gui::LOCALE, s("en"))],
        )));
        let language = locale_write(&state(&[], &[]), Scope::Device, &ja());
        assert_eq!(
            write(merged(unset, language)),
            Some((
                Scope::Device,
                vec![(gui::LOCALE.to_owned(), s("ja"))],
                vec![gui::THEME_CONTRAST.to_owned()]
            ))
        );
        assert_eq!(write(merged(None, None)), None);
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
        assert_eq!(
            overridden(&st),
            [
                "sapphire-appearance-name-contrast",
                "sapphire-appearance-name-accent"
            ]
        );
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
            language: true,
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
            language: true,
        };
        let mut page = AppearancePage {
            tab: Scope::Device,
            ..Default::default()
        };
        let text = drawn(&mut page, &cx);
        assert!(text.contains("light/dark: from workgroup"), "{text}");
        assert!(text.contains("accent, language: default"), "{text}");
        assert!(text.contains("Use the workgroup's"), "{text}");
    }

    #[test]
    fn the_tabs_and_lines_follow_the_language() {
        use crate::views::config::widgets::tests::drawn_text_in;
        let st = state(
            &[(gui::THEME_PREFERENCE, s("dark"))],
            &[(gui::THEME_CONTRAST, s("low"))],
        );
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: true,
            language: true,
        };
        let ctx = crate::test_support::japanese_ctx();
        let mut page = AppearancePage {
            tab: Scope::Device,
            ..Default::default()
        };
        let text = drawn_text_in(&ctx, |ui| {
            page.ui(ui, &cx);
        });
        for ja in [
            "外観",
            "ワークグループ",
            "デバイス",
            "ライト/ダーク: ワークグループから",
            "アクセント、言語: デフォルト",
            "ワークグループの設定を使う",
        ] {
            assert!(text.contains(ja), "{ja} missing from {text}");
        }
        assert!(!text.contains("Use the workgroup's"), "{text}");
    }

    #[test]
    fn unavailable_draws_the_reason() {
        let cx = AppearanceCtx {
            state: None,
            unavailable: Some(bridge_down()),
            busy: false,
            workgroup: true,
            language: true,
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
            language: true,
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
            language: true,
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
            language: true,
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

    // ── the language ────────────────────────────────────────────────────────

    fn lang(language: Language) -> LanguageSettings {
        LanguageSettings { language }
    }

    fn ja() -> LanguageSettings {
        lang(Language::Explicit("ja".into()))
    }

    #[test]
    fn locale_values_map_to_the_language() {
        assert_eq!(
            language_settings(&GuiValues::new()),
            LanguageSettings::default()
        );
        assert_eq!(
            language_settings(&one(gui::LOCALE, s("system"))),
            lang(Language::System)
        );
        assert_eq!(language_settings(&one(gui::LOCALE, s("ja"))), ja());
        assert_eq!(language_settings(&one(gui::LOCALE, s(" ja "))), ja());
        for system in ["System", "SYSTEM", " system "] {
            assert_eq!(
                language_settings(&one(gui::LOCALE, s(system))),
                lang(Language::System),
                "{system}"
            );
        }
        assert_eq!(
            language_settings(&one(gui::LOCALE, s("en-US"))),
            lang(Language::Explicit("en-US".into()))
        );
    }

    #[test]
    fn a_bad_locale_follows_the_system() {
        for bad in [s("not a tag!"), s(""), s("en_US_x_"), Value::Int(3)] {
            assert_eq!(
                language_settings(&one(gui::LOCALE, bad.clone())),
                lang(Language::System),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_language_round_trips_through_its_entry() {
        for settings in [
            lang(Language::System),
            ja(),
            lang(Language::Explicit("en-US".into())),
        ] {
            let entry = locale_entry(&settings);
            assert_eq!(entry.key, gui::LOCALE);
            assert_eq!(language_settings(&values(vec![entry])), settings);
        }
        assert_eq!(
            locale_entry(&LanguageSettings::default()).value,
            s("system")
        );
        assert_eq!(locale_entry(&ja()).value, s("ja"));
    }

    #[test]
    fn a_language_change_writes_only_the_locale_to_the_tab() {
        let st = state(
            &[(gui::THEME_CONTRAST, s("high"))],
            &[(gui::THEME_ACCENT, s("red"))],
        );
        for tab in [Scope::Workgroup, Scope::Device] {
            assert_eq!(
                write(locale_write(&st, tab, &ja())),
                Some((tab, vec![(gui::LOCALE.to_owned(), s("ja"))], Vec::new())),
                "{tab:?}"
            );
        }
        // The tab's own value: nothing to write.
        assert_eq!(
            write(locale_write(&st, Scope::Device, &lang(Language::System))),
            None
        );
    }

    #[test]
    fn the_device_tab_starts_from_the_effective_language() {
        let st = state(&[(gui::LOCALE, s("ja"))], &[]);
        assert_eq!(tab_language(&st, Scope::Device), ja());
        assert_eq!(write(locale_write(&st, Scope::Device, &ja())), None);
    }

    #[test]
    fn a_choice_on_the_page_installs_and_writes_once() {
        let st = state(&[], &[]);
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: true,
            language: true,
        };
        let ctx = crate::test_support::english_ctx();
        let mut page = AppearancePage {
            tab: Scope::Device,
            language: ja(),
            ..Default::default()
        };
        let sent = write(page.language_chosen(&ctx, &st, &cx));
        assert_eq!(
            sent,
            Some((
                Scope::Device,
                vec![(gui::LOCALE.to_owned(), s("ja"))],
                Vec::new()
            ))
        );
        assert_eq!(i18n::current(&ctx).to_string(), "ja");
        assert!(page.held);
    }

    #[test]
    fn a_workgroup_choice_keeps_the_devices_language() {
        let st = state(&[], &[(gui::LOCALE, s("en"))]);
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: true,
            language: true,
        };
        let ctx = crate::test_support::english_ctx();
        let mut page = AppearancePage {
            language: ja(),
            ..Default::default()
        };
        let sent = write(page.language_chosen(&ctx, &st, &cx));
        assert_eq!(
            sent,
            Some((
                Scope::Workgroup,
                vec![(gui::LOCALE.to_owned(), s("ja"))],
                Vec::new()
            ))
        );
        assert_eq!(i18n::current(&ctx).to_string(), "en");
        // Without a workgroup both scopes are one file: the choice is what shows.
        assert_eq!(expected_language(&st, Scope::Workgroup, &ja(), false), ja());
    }

    #[test]
    fn the_picker_is_in_both_tabs() {
        let st = state(&[], &[]);
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: true,
            language: true,
        };
        for tab in [Scope::Workgroup, Scope::Device] {
            let mut page = AppearancePage {
                tab,
                ..Default::default()
            };
            let text = drawn(&mut page, &cx);
            assert!(text.contains("Language"), "{tab:?}: {text}");
            assert!(text.contains("System ("), "{tab:?}: {text}");
        }
    }

    #[test]
    fn the_override_line_names_the_language() {
        let st = state(&[], &[(gui::LOCALE, s("ja"))]);
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: true,
            language: true,
        };
        let text = drawn(&mut AppearancePage::default(), &cx);
        assert!(text.contains("This device uses its own language"), "{text}");
        let mut page = AppearancePage {
            tab: Scope::Device,
            ..Default::default()
        };
        let text = drawn(&mut page, &cx);
        assert!(text.contains("Use the workgroup's"), "{text}");
        assert!(!text.contains("language: "), "{text}");
    }

    #[test]
    fn the_device_tab_says_the_language_is_the_workgroups() {
        let st = state(&[(gui::LOCALE, s("ja"))], &[]);
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: true,
            language: true,
        };
        let mut page = AppearancePage {
            tab: Scope::Device,
            ..Default::default()
        };
        let text = drawn(&mut page, &cx);
        assert!(text.contains("language: from workgroup"), "{text}");
        assert_eq!(page.language, ja());
    }

    #[test]
    fn a_bridge_without_the_language_says_so() {
        let st = state(&[], &[]);
        let cx = AppearanceCtx {
            state: Some(&st),
            unavailable: None,
            busy: false,
            workgroup: true,
            language: false,
        };
        let text = drawn(&mut AppearancePage::default(), &cx);
        assert!(
            text.contains("this sapphire-bridge does not know the language setting"),
            "{text}"
        );
        // The theme is still there.
        assert!(text.contains("Workgroup"), "{text}");
    }
}
