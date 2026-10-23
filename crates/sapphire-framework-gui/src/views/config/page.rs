//! One settings page: a schema section, a tab per scope, a row per key.

use std::collections::{BTreeMap, BTreeSet};

use egui::Color32;
use sapphire_bridge_api::settings::{Apply, Process};
use sapphire_bridge_api::{EntryValue, Origin, Scope, SettingDef};

use crate::client::{Command, CommandError, CommandOutput, ConfigState, Conn};

use super::super::{ViewCtx, error_line, unavailable};
use super::model::{Drafts, Edit, PageSpec, origin_label, route_errors, tab_enabled};
use super::widgets::value_widget;

/// What the page last sent, so its outcome lands in the right place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sent {
    Load,
    Write(Scope),
}

/// One settings page. It holds no values: those come from the loaded [`ConfigState`] and
/// the [`Drafts`] passed in each frame.
#[derive(Debug)]
pub(crate) struct ConfigPage {
    /// The schema section it shows.
    pub section: String,
    /// The open tab.
    pub tab: Scope,
    /// The last failed write's errors: by key for the page's fields, and the lines for the
    /// rest.
    pub errors: (BTreeMap<String, String>, Vec<String>),
    /// The open tab's drafted keys whose value changed under the draft.
    pub changed: BTreeSet<String>,
    /// The page's keys, for routing a write's field errors.
    keys: Vec<String>,
    sent: Option<Sent>,
}

impl ConfigPage {
    /// A page for `section`, on its first tab.
    pub(crate) fn new(section: impl Into<String>) -> ConfigPage {
        ConfigPage {
            section: section.into(),
            tab: Scope::Device,
            errors: Default::default(),
            changed: BTreeSet::new(),
            keys: Vec::new(),
            sent: None,
        }
    }

    /// Render. Returns a [`Command::ConfigWrite`] on Apply and a [`Command::ConfigLoad`] on
    /// "↻".
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        cx: &ViewCtx,
        spec: &PageSpec,
        state: &ConfigState,
        drafts: &mut Drafts,
    ) -> Option<Command> {
        let mut out = None;
        self.keys.clone_from(&spec.keys);
        ui.horizontal(|ui| {
            ui.heading(&spec.section);
            if cx.busy {
                ui.spinner();
            } else if ui.small_button("↻").on_hover_text("Reload").clicked() {
                out = Some(self.reload());
            }
        });
        ui.separator();
        let Some(server) = cx.snapshot.server.up() else {
            unavailable(ui, &format!("The {} server", cx.app_name));
            return out;
        };
        let workspace = server.current.is_some();

        // Tabs. An unusable open tab yields to the first usable one; its draft is kept.
        if (!spec.scopes.contains(&self.tab) || !tab_enabled(self.tab, workspace))
            && let Some(first) = spec
                .scopes
                .iter()
                .copied()
                .find(|s| tab_enabled(*s, workspace))
        {
            self.tab = first;
        }
        ui.horizontal_wrapped(|ui| {
            for scope in &spec.scopes {
                let dirty = drafts
                    .get(&spec.section, *scope)
                    .is_some_and(|d| !d.is_empty());
                let label = format!("{}{}", tab_label(*scope), if dirty { " •" } else { "" });
                if ui
                    .add_enabled(
                        tab_enabled(*scope, workspace),
                        egui::Button::selectable(self.tab == *scope, label),
                    )
                    .clicked()
                {
                    self.tab = *scope;
                }
            }
            if spec.scopes.contains(&Scope::Workspace) && !workspace {
                ui.small("No workspace is selected");
            }
        });
        if !tab_enabled(self.tab, workspace) {
            return out;
        }
        let tab = self.tab;

        let bridge_up = matches!(cx.snapshot.bridge, Conn::Up(_));
        let defs: Vec<&SettingDef> = spec
            .keys
            .iter()
            .filter_map(|key| state.schema.iter().find(|d| d.key == *key))
            .filter(|d| d.allows(tab))
            .collect();
        if !bridge_up && defs.iter().any(|d| d.process == Process::Bridge) {
            ui.colored_label(WARN, "sapphire-bridge is not running");
        }
        let draft = drafts.get_mut(&spec.section, tab);
        self.changed = draft.on_reload(state, tab);
        ui.add_space(4.0);

        for def in defs {
            let here = state
                .by_scope
                .get(&tab)
                .and_then(|entries| entries.iter().find(|e| e.key == def.key))
                .filter(|e| e.value != EntryValue::Unset);
            let (shown, origin) = match here {
                Some(e) => (Some(&e.value), None),
                None => match state.effective.iter().find(|e| e.key == def.key) {
                    Some(e) => (Some(&e.value), origin_label(e.origin, tab)),
                    None => (None, origin_label(Origin::Default, tab)),
                },
            };
            let edit = draft.edits.get(&def.key);
            let editable = editable(def, bridge_up) && !cx.busy;
            let mut new_edit = None;
            ui.push_id((&spec.section, tab, &def.key), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.monospace(&def.key);
                    if let Some(origin) = origin {
                        ui.small(origin);
                    }
                    if def.apply == Apply::Restart {
                        ui.small("restart")
                            .on_hover_text("Takes effect after a restart");
                    }
                    if self.changed.contains(&def.key) {
                        ui.colored_label(WARN, "changed elsewhere");
                    }
                    if matches!(edit, Some(Edit::Unset)) {
                        ui.small("removed here on Apply");
                    } else if here.is_some()
                        && ui
                            .add_enabled(editable, egui::Button::new("×").small())
                            .on_hover_text(format!("Remove from {}", tab_label(tab)))
                            .clicked()
                    {
                        new_edit = Some(Edit::Unset);
                    }
                });
                if let Some(e) = value_widget(ui, def, shown, edit, editable) {
                    new_edit = Some(e);
                }
                if !def.doc.is_empty() {
                    ui.weak(&def.doc);
                }
                let error = match edit {
                    Some(Edit::Set { parsed: Err(e), .. }) => Some(e),
                    _ => self.errors.0.get(&def.key),
                };
                if let Some(error) = error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
            });
            // Drafted and observed together, so a later reload can tell what changed.
            match new_edit {
                Some(Edit::Set { text, .. }) => draft.set(&def.key, &def.kind, &text),
                Some(Edit::Unset) => draft.unset(&def.key),
                None => continue,
            }
            draft.observe(state, tab);
            self.errors.0.remove(&def.key);
        }

        ui.add_space(8.0);
        let mut lines = (!self.errors.1.is_empty()).then(|| self.errors.1.join("\n"));
        error_line(ui, &mut lines);
        if lines.is_none() {
            self.errors.1.clear();
        }
        let draft = drafts.get(&spec.section, tab);
        let has_draft = draft.is_some_and(|d| !d.is_empty());
        let valid = draft.is_some_and(|d| d.valid());
        ui.horizontal(|ui| {
            if ui
                .add_enabled(has_draft && !cx.busy, egui::Button::new("Revert"))
                .clicked()
            {
                drafts.get_mut(&spec.section, tab).revert();
                self.errors.0.clear();
            }
            if ui
                .add_enabled(has_draft && valid && !cx.busy, egui::Button::new("Apply"))
                .clicked()
            {
                out = self.apply(drafts).or(out.take());
            }
        });
        out
    }

    /// The write the open tab's draft makes, when it has one and every field parses.
    pub(crate) fn apply(&mut self, drafts: &mut Drafts) -> Option<Command> {
        let draft = drafts.get(&self.section, self.tab)?;
        if draft.is_empty() || !draft.valid() {
            return None;
        }
        let (set, unset) = draft.batch(self.tab)?;
        self.sent = Some(Sent::Write(self.tab));
        self.errors = Default::default();
        Some(Command::ConfigWrite {
            scope: self.tab,
            set,
            unset,
        })
    }

    /// Reload the settings.
    fn reload(&mut self) -> Command {
        self.sent = Some(Sent::Load);
        Command::ConfigLoad
    }

    /// Take the outcome of the command this page sent: a written draft is dropped (its
    /// typed secrets with it); a failed one stays, with its errors shown.
    pub(crate) fn on_outcome(
        &mut self,
        result: &Result<CommandOutput, CommandError>,
        drafts: &mut Drafts,
    ) {
        match (self.sent.take(), result) {
            (Some(Sent::Write(scope)), Ok(_)) => {
                drafts.get_mut(&self.section, scope).revert();
                self.errors = Default::default();
                self.changed.clear();
            }
            (Some(Sent::Write(_)), Err(e)) => self.errors = route_errors(e, &self.keys),
            (_, Err(e)) => self.errors.1 = vec![e.message.clone()],
            (_, Ok(_)) => {}
        }
    }
}

/// The warning colour, as the Embedding screen uses.
const WARN: Color32 = Color32::from_rgb(230, 160, 40);

/// Whether a key can be edited: a bridge key needs the bridge.
fn editable(def: &SettingDef, bridge_up: bool) -> bool {
    bridge_up || def.process != Process::Bridge
}

fn tab_label(scope: Scope) -> &'static str {
    match scope {
        Scope::Device => "Device",
        Scope::Workgroup => "Workgroup",
        Scope::Workspace => "Workspace",
    }
}

#[cfg(test)]
mod tests {
    use sapphire_backend::protocol::{CurrentWorkspace, StatusReport, SyncStatusResult};
    use sapphire_bridge_api::settings::SecretState;
    use sapphire_bridge_api::{ConfigFieldError, ConfigWriteResult, Entry, Kind, Value};

    use super::super::model::pages;
    use super::super::widgets::tests::{def, drawn_text, every_kind};
    use super::*;
    use crate::client::{ServerState, Snapshot};
    use crate::test_support::render;

    fn snapshot(workspace: bool) -> Snapshot {
        Snapshot {
            fetched: true,
            bridge: Conn::Absent,
            server: Conn::Up(ServerState {
                info: StatusReport {
                    running: true,
                    version: Some("1".into()),
                    pid: Some(1),
                    managed_by: None,
                    app: vec![],
                },
                current: workspace.then(|| CurrentWorkspace {
                    root: "/x/notes".into(),
                    reachable: true,
                    workspace_id: None,
                    sync: SyncStatusResult::not_synced(),
                }),
                update: None,
            }),
        }
    }

    fn cx(snapshot: &Snapshot) -> ViewCtx<'_> {
        ViewCtx {
            snapshot,
            app_name: "app",
            busy: false,
        }
    }

    /// Every kind, allowed in every scope, with a value in the device file and an
    /// effective one from the workgroup.
    fn state() -> ConfigState {
        let schema: Vec<_> = every_kind()
            .into_iter()
            .map(|mut d| {
                d.scopes = vec![Scope::Device, Scope::Workgroup, Scope::Workspace];
                d
            })
            .collect();
        let entry = |key: &str, value, origin| Entry {
            key: key.into(),
            value,
            origin,
            pending_restart: false,
            note: None,
        };
        ConfigState {
            effective: vec![
                entry(
                    "k.string",
                    EntryValue::Value(Value::String("wg".into())),
                    Origin::Scope(Scope::Workgroup),
                ),
                entry(
                    "k.secret",
                    EntryValue::Secret(SecretState::Set),
                    Origin::Scope(Scope::Device),
                ),
            ],
            by_scope: BTreeMap::from([(
                Scope::Device,
                vec![
                    entry(
                        "k.bool",
                        EntryValue::Value(Value::Bool(true)),
                        Origin::Scope(Scope::Device),
                    ),
                    entry(
                        "k.secret",
                        EntryValue::Secret(SecretState::Set),
                        Origin::Scope(Scope::Device),
                    ),
                ],
            )]),
            schema,
        }
    }

    fn spec(state: &ConfigState) -> PageSpec {
        pages(&state.schema, "app", &[]).remove(0)
    }

    #[test]
    fn every_kind_renders() {
        let (state, snapshot) = (state(), snapshot(true));
        let spec = spec(&state);
        assert_eq!(spec.scopes.len(), 3);
        let mut drafts = Drafts::default();
        for tab in spec.scopes.clone() {
            let mut page = ConfigPage::new("S");
            page.tab = tab;
            render([600.0, 1600.0], |ui| {
                assert!(
                    page.ui(ui, &cx(&snapshot), &spec, &state, &mut drafts)
                        .is_none()
                );
            });
            assert_eq!(page.tab, tab);
        }
        assert!(drafts.dirty_sections().is_empty());
    }

    #[test]
    fn a_bridge_key_is_read_only_without_a_bridge() {
        let mut bridge_key = def("embed.x", Kind::String { non_empty: false });
        bridge_key.process = Process::Bridge;
        assert!(!editable(&bridge_key, false));
        assert!(editable(&bridge_key, true));
        assert!(editable(&def("app.x", Kind::Bool), false));
        render([600.0, 400.0], |ui| {
            assert!(value_widget(ui, &bridge_key, None, None, false).is_none());
        });
        let state = ConfigState {
            schema: vec![bridge_key],
            ..Default::default()
        };
        let snapshot = snapshot(false);
        let drawn = drawn_text(|ui| {
            let mut page = ConfigPage::new("S");
            page.ui(
                ui,
                &cx(&snapshot),
                &spec(&state),
                &state,
                &mut Drafts::default(),
            );
        });
        assert!(drawn.contains("sapphire-bridge is not running"), "{drawn}");
    }

    #[test]
    fn no_workspace_disables_the_workspace_tab() {
        let (state, snapshot) = (state(), snapshot(false));
        let spec = spec(&state);
        let mut drafts = Drafts::default();
        drafts.get_mut("S", Scope::Workspace).unset("k.bool");
        let mut page = ConfigPage::new("S");
        page.tab = Scope::Workspace;
        let drawn = drawn_text(|ui| {
            assert!(
                page.ui(ui, &cx(&snapshot), &spec, &state, &mut drafts)
                    .is_none()
            );
        });
        assert_eq!(page.tab, Scope::Device, "falls back to a usable tab");
        assert!(drawn.contains("No workspace is selected"), "{drawn}");
        assert!(
            !drafts.get("S", Scope::Workspace).unwrap().is_empty(),
            "the workspace draft waits"
        );

        // A page with only workspace keys shows the hint and no rows.
        let mut ws_only = state.clone();
        for d in &mut ws_only.schema {
            d.scopes = vec![Scope::Workspace];
        }
        let spec = super::super::model::pages(&ws_only.schema, "app", &[]).remove(0);
        let mut page = ConfigPage::new("S");
        let drawn = drawn_text(|ui| {
            page.ui(ui, &cx(&snapshot), &spec, &ws_only, &mut drafts);
        });
        assert!(drawn.contains("No workspace is selected") && !drawn.contains("k.bool"));
    }

    #[test]
    fn no_app_server_is_unavailable() {
        let state = state();
        let drawn = drawn_text(|ui| {
            let snapshot = Snapshot::default();
            let mut page = ConfigPage::new("S");
            page.ui(
                ui,
                &cx(&snapshot),
                &spec(&state),
                &state,
                &mut Drafts::default(),
            );
        });
        assert!(drawn.contains("The app server is not available"), "{drawn}");
    }

    #[test]
    fn apply_emits_one_write_with_the_tab_scope() {
        let (state, snapshot) = (state(), snapshot(true));
        let spec = spec(&state);
        let mut drafts = Drafts::default();
        let mut page = ConfigPage::new("S");
        assert!(page.apply(&mut drafts).is_none(), "nothing drafted");
        let kind = Kind::Int { min: 1, max: 9 };
        drafts.get_mut("S", Scope::Device).set("k.int", &kind, "3");
        drafts.get_mut("S", Scope::Device).unset("k.bool");
        drafts
            .get_mut("S", Scope::Workgroup)
            .set("k.int", &kind, "4");
        render([600.0, 1600.0], |ui| {
            assert!(
                page.ui(ui, &cx(&snapshot), &spec, &state, &mut drafts)
                    .is_none()
            );
        });
        match page.apply(&mut drafts) {
            Some(Command::ConfigWrite { scope, set, unset }) => {
                assert_eq!(scope, Scope::Device);
                assert_eq!(set.len(), 1);
                assert_eq!(
                    (set[0].key.as_str(), &set[0].value),
                    ("k.int", &Value::Int(3))
                );
                assert_eq!(unset, ["k.bool"]);
            }
            other => panic!("expected a write, got {other:?}"),
        }

        // An invalid field holds Apply back.
        drafts.get_mut("S", Scope::Device).set("k.int", &kind, "99");
        assert!(page.apply(&mut drafts).is_none());
    }

    #[test]
    fn a_failed_write_shows_field_errors_and_keeps_the_draft() {
        let (state, snapshot) = (state(), snapshot(true));
        let spec = spec(&state);
        let mut drafts = Drafts::default();
        let mut page = ConfigPage::new("S");
        render([600.0, 1600.0], |ui| {
            page.ui(ui, &cx(&snapshot), &spec, &state, &mut drafts);
        });
        drafts
            .get_mut("S", Scope::Device)
            .set("k.url", &Kind::Url, "https://y");
        assert!(page.apply(&mut drafts).is_some());
        let err = CommandError {
            message: "rejected".into(),
            fields: vec![
                ConfigFieldError {
                    key: "k.url".into(),
                    message: "unreachable host".into(),
                },
                ConfigFieldError {
                    key: "other.key".into(),
                    message: "bad".into(),
                },
            ],
        };
        page.on_outcome(&Err(err), &mut drafts);
        assert_eq!(page.errors.0["k.url"], "unreachable host");
        assert_eq!(page.errors.1, ["rejected", "other.key: bad"]);
        assert!(!drafts.get("S", Scope::Device).unwrap().is_empty());
        let drawn = drawn_text(|ui| {
            page.ui(ui, &cx(&snapshot), &spec, &state, &mut drafts);
        });
        assert!(drawn.contains("unreachable host") && drawn.contains("other.key: bad"));

        // Applied again, and written: the draft and the errors go.
        assert!(page.apply(&mut drafts).is_some());
        page.on_outcome(
            &Ok(CommandOutput::Written(ConfigWriteResult::default())),
            &mut drafts,
        );
        assert!(drafts.dirty_sections().is_empty());
        assert_eq!(page.errors, Default::default());
    }

    #[test]
    fn a_written_secret_leaves_the_draft() {
        let mut drafts = Drafts::default();
        let mut page = ConfigPage::new("S");
        drafts
            .get_mut("S", Scope::Device)
            .set("k.secret", &Kind::Secret, "hunter2");
        assert!(page.apply(&mut drafts).is_some());
        page.on_outcome(&Ok(CommandOutput::Written(Default::default())), &mut drafts);
        assert!(drafts.get("S", Scope::Device).unwrap().edits.is_empty());
    }

    #[test]
    fn a_failed_load_shows_its_message() {
        let mut drafts = Drafts::default();
        let mut page = ConfigPage::new("S");
        assert!(matches!(page.reload(), Command::ConfigLoad));
        page.on_outcome(&Err("no bridge".into()), &mut drafts);
        assert_eq!(page.errors.1, ["no bridge"]);
    }

    #[test]
    fn a_drafted_key_changed_elsewhere_is_marked() {
        let (mut state, snapshot) = (state(), snapshot(true));
        let spec = spec(&state);
        let mut drafts = Drafts::default();
        let d = drafts.get_mut("S", Scope::Device);
        d.set("k.bool", &Kind::Bool, "false");
        d.observe(&state, Scope::Device);
        state.by_scope.get_mut(&Scope::Device).unwrap()[0].value =
            EntryValue::Value(Value::Bool(false));
        let mut page = ConfigPage::new("S");
        let drawn = drawn_text(|ui| {
            page.ui(ui, &cx(&snapshot), &spec, &state, &mut drafts);
        });
        assert!(page.changed.contains("k.bool"));
        assert!(drawn.contains("changed elsewhere"));
    }
}
