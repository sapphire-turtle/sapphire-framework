//! One settings page: a schema section, a tab per scope, a row per key.

use std::collections::{BTreeMap, BTreeSet};

use crate::theme::Swatch;
use sapphire_bridge_api::settings::{Apply, Process};
use sapphire_bridge_api::{EntryValue, Kind, Origin, Scope, SettingDef};

use crate::client::{Command, CommandError, CommandOutput, ConfigState, Conn};

use super::super::{ViewCtx, error_text_line, register, unavailable};
use super::model::{Drafts, Edit, PageSpec, origin_label, route_errors, tab_enabled};
use super::widgets::{secret_id, value_widget};
use crate::i18n::Message;
use corundum_framework_gui::settings::{ScopeTab, scope_tabs};
use corundum_framework_gui::tr;

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
    /// Whether the page draws its section's name as a heading; off inside a screen that
    /// titles itself.
    pub titled: bool,
    /// The page's keys, for routing a write's field errors.
    keys: Vec<String>,
    /// The page's secret keys, with their schema sections, and the context their fields
    /// are drawn in: a successful write clears what egui kept of the typing.
    secrets: Vec<(String, String)>,
    ctx: Option<egui::Context>,
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
            titled: true,
            keys: Vec::new(),
            secrets: Vec::new(),
            ctx: None,
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
        register(ui);
        let mut out = None;
        self.keys.clone_from(&spec.keys);
        self.secrets = state
            .schema
            .iter()
            .filter(|d| d.kind == Kind::Secret && spec.keys.contains(&d.key))
            .map(|d| (d.section.clone(), d.key.clone()))
            .collect();
        self.ctx = Some(ui.ctx().clone());
        ui.horizontal(|ui| {
            if self.titled {
                ui.heading(&spec.section);
            }
            if cx.busy {
                ui.spinner();
            } else if ui
                .small_button("↻")
                .on_hover_text(tr!(ui, "sapphire-config-reload"))
                .clicked()
            {
                out = Some(self.reload());
            }
        });
        ui.separator();
        let Some(server) = cx.snapshot.server.up() else {
            unavailable(
                ui,
                Message::new("sapphire-unavailable-server").arg("app", cx.app_name),
            );
            return out;
        };
        let workspace = server.current.is_some();

        // Tabs. An unusable open tab yields to the first usable one; its draft is kept.
        let tabs: Vec<ScopeTab<Scope>> = spec
            .scopes
            .iter()
            .map(|&scope| {
                ScopeTab::new(scope, tr!(ui, tab_label(scope)))
                    .enabled(tab_enabled(scope, workspace))
                    .dirty(
                        drafts
                            .get(&spec.section, scope)
                            .is_some_and(|d| !d.is_empty()),
                    )
            })
            .collect();
        let hint = (spec.scopes.contains(&Scope::Workspace) && !workspace)
            .then(|| tr!(ui, "sapphire-config-no-workspace"));
        if scope_tabs(ui, &mut self.tab, &tabs, hint.as_deref()) {
            // Field errors belong to the tab whose write failed.
            self.errors.0.clear();
        }
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
            ui.colored_label(warn(ui), tr!(ui, "sapphire-bridge-not-running"));
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
            let mut forget = false;
            ui.push_id((&spec.section, tab, &def.key), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.monospace(&def.key);
                    if let Some(origin) = origin {
                        ui.small(tr!(ui, origin));
                    }
                    if def.apply == Apply::Restart {
                        ui.small(tr!(ui, "sapphire-config-restart"))
                            .on_hover_text(tr!(ui, "sapphire-config-restart-hover"));
                    }
                    if self.changed.contains(&def.key) {
                        ui.colored_label(warn(ui), tr!(ui, "sapphire-config-changed-elsewhere"));
                    }
                    if matches!(edit, Some(Edit::Unset)) {
                        forget = ui
                            .add_enabled(
                                !cx.busy,
                                egui::Button::new(tr!(ui, "sapphire-config-removed-here")).small(),
                            )
                            .on_hover_text(tr!(ui, "sapphire-config-keep"))
                            .clicked();
                    } else if here.is_some()
                        && ui
                            .add_enabled(editable, egui::Button::new("×").small())
                            .on_hover_text(tr!(
                                ui,
                                "sapphire-config-remove-from",
                                tab = tr!(ui, tab_label(tab))
                            ))
                            .clicked()
                    {
                        new_edit = Some(Edit::Unset);
                    }
                });
                if let Some(e) = value_widget(ui, def, tab, shown, edit, editable) {
                    new_edit = Some(e);
                }
                if !def.doc.is_empty() {
                    ui.weak(&def.doc);
                }
                let error = match edit {
                    Some(Edit::Set { parsed: Err(e), .. }) => Some(e.text(ui.ctx())),
                    _ => self.errors.0.get(&def.key).cloned(),
                };
                if let Some(error) = error {
                    ui.colored_label(crate::theme::current(ui.ctx()).text(Swatch::DANGER), error);
                }
            });
            if forget {
                draft.forget(&def.key);
                continue;
            }
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
        error_text_line(ui, &mut lines);
        if lines.is_none() {
            self.errors.1.clear();
        }
        let draft = drafts.get(&spec.section, tab);
        let has_draft = draft.is_some_and(|d| !d.is_empty());
        let valid = draft.is_some_and(|d| d.valid());
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    has_draft && !cx.busy,
                    egui::Button::new(tr!(ui, "sapphire-config-revert")),
                )
                .clicked()
            {
                drafts.get_mut(&spec.section, tab).revert();
                self.errors.0.clear();
            }
            if ui
                .add_enabled(
                    has_draft && valid && !cx.busy,
                    egui::Button::new(tr!(ui, "sapphire-config-apply")),
                )
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
    /// typed secrets with it, and egui's undo history of them); a failed one stays, with its
    /// errors shown.
    pub(crate) fn on_outcome(
        &mut self,
        result: &Result<CommandOutput, CommandError>,
        drafts: &mut Drafts,
    ) {
        match (self.sent.take(), result) {
            (Some(Sent::Write(scope)), Ok(_)) => {
                // egui keeps every edit, password fields too, for Ctrl+Z. The field may
                // not be drawn again (another page is open, or the section has gone), so
                // its typing goes now rather than then.
                if let Some(ctx) = &self.ctx {
                    for (section, key) in &self.secrets {
                        let id = secret_id(section, scope, key);
                        ctx.data_mut(|d| d.remove::<egui::text_edit::TextEditState>(id));
                    }
                }
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

/// The theme's warning colour, as the Embedding screen uses.
fn warn(ui: &egui::Ui) -> egui::Color32 {
    crate::theme::current(ui.ctx()).text(Swatch::WARNING)
}

/// Whether a key can be edited: a bridge key needs the bridge.
fn editable(def: &SettingDef, bridge_up: bool) -> bool {
    bridge_up || def.process != Process::Bridge
}

/// The message naming a tab.
fn tab_label(scope: Scope) -> &'static str {
    super::super::appearance::scope_id(scope)
}

#[cfg(test)]
mod tests {
    use sapphire_backend::protocol::{CurrentWorkspace, StatusReport, SyncStatusResult};
    use sapphire_bridge_api::settings::SecretState;
    use sapphire_bridge_api::{ConfigFieldError, ConfigWriteResult, Entry, Kind, Value};

    use super::super::model::pages;
    use super::super::widgets::secret_id;
    use super::super::widgets::tests::{def, drawn_text, every_kind, remembered};
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
        pages(&state.schema, &[]).remove(0)
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
            assert!(value_widget(ui, &bridge_key, Scope::Device, None, None, false).is_none());
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
        page.errors
            .0
            .insert("k.bool".into(), "workspace-only error".into());
        let drawn = drawn_text(|ui| {
            assert!(
                page.ui(ui, &cx(&snapshot), &spec, &state, &mut drafts)
                    .is_none()
            );
        });
        assert_eq!(page.tab, Scope::Device, "falls back to a usable tab");
        assert!(page.errors.0.is_empty(), "the old tab's field errors go");
        assert!(!drawn.contains("workspace-only error"));
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
        let spec = super::super::model::pages(&ws_only.schema, &[]).remove(0);
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
    fn the_page_follows_the_language() {
        use super::super::widgets::tests::drawn_text_in;
        let (state, snapshot) = (state(), snapshot(false));
        let spec = spec(&state);
        let mut drafts = Drafts::default();
        drafts.get_mut("S", Scope::Device).unset("k.bool");
        let ctx = crate::test_support::japanese_ctx();
        let mut page = ConfigPage::new("S");
        let drawn = drawn_text_in(&ctx, |ui| {
            page.ui(ui, &cx(&snapshot), &spec, &state, &mut drafts);
        });
        for ja in [
            "デバイス •",
            "ワークスペースが選択されていません",
            "適用時にここから削除 ↺",
            "元に戻す",
            "適用",
        ] {
            assert!(drawn.contains(ja), "{ja} missing from {drawn}");
        }
        assert!(!drawn.contains("Revert"), "{drawn}");
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
    fn a_written_secret_leaves_no_undo_history_without_a_redraw() {
        let (state, snapshot) = (state(), snapshot(true));
        let spec = spec(&state);
        let mut drafts = Drafts::default();
        let mut page = ConfigPage::new("S");
        let ctx = egui::Context::default();
        let id = secret_id("S", Scope::Device, "k.secret");
        let mut frame = |time: f64, events: Vec<egui::Event>, page: &mut ConfigPage| {
            let input = egui::RawInput {
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                page.ui(ui, &cx(&snapshot), &spec, &state, &mut drafts);
            });
            output.textures_delta.clear();
        };
        frame(0.0, vec![], &mut page);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(0.1, vec![], &mut page);
        frame(0.2, vec![egui::Event::Text("hunter2".into())], &mut page);
        // Long enough for egui to make "hunter2" an undo point.
        frame(5.0, vec![], &mut page);
        frame(10.0, vec![], &mut page);
        assert!(remembered(&ctx, id).iter().any(|t| t == "hunter2"));

        // Applied and written while the page is not drawn (another page is open, or the
        // section is gone after the reload): the typing goes all the same.
        assert!(page.apply(&mut drafts).is_some());
        page.on_outcome(&Ok(CommandOutput::Written(Default::default())), &mut drafts);
        let kept = remembered(&ctx, id);
        assert!(!kept.iter().any(|t| t.contains("hunter2")), "{kept:?}");
    }

    #[test]
    fn a_drafted_unset_offers_to_keep_the_key() {
        let (state, snapshot) = (state(), snapshot(true));
        let spec = spec(&state);
        let mut drafts = Drafts::default();
        drafts.get_mut("S", Scope::Device).unset("k.bool");
        let mut page = ConfigPage::new("S");
        let drawn = drawn_text(|ui| {
            page.ui(ui, &cx(&snapshot), &spec, &state, &mut drafts);
        });
        assert!(drawn.contains("removed here on Apply ↺"), "{drawn}");
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
