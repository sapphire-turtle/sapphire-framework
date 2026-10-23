//! The pure model behind the settings pages: which pages the schema makes, the drafts a
//! person has typed, and how a write's outcome is routed. No rendering.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use sapphire_bridge_api::settings::Process;
use sapphire_bridge_api::{ConfigEntry, Entry, EntryValue, Kind, Origin, Scope, SettingDef, Value};

use crate::client::{CommandError, ConfigState};

/// The scopes, in tab order.
const SCOPES: [Scope; 3] = [Scope::Device, Scope::Workgroup, Scope::Workspace];

/// Where a page is listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    /// Under the app's heading, before its own pages.
    App,
    /// Inside the existing screen with this label.
    Screen(String),
    /// Under the "Configuration" heading.
    Configuration,
}

/// One settings page: one schema section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageSpec {
    /// The section's name.
    pub section: String,
    /// Where it is listed.
    pub placement: Placement,
    /// The tabs: the scopes any of its keys allow, in tab order.
    pub scopes: Vec<Scope>,
    /// Its keys, in schema order.
    pub keys: Vec<String>,
}

/// The pages the schema makes, one per distinct section, in schema order. `screens` are the
/// labels of the existing screens a section may join.
pub(crate) fn pages(schema: &[SettingDef], app_name: &str, screens: &[&str]) -> Vec<PageSpec> {
    let prefix = format!("{app_name}.");
    let mut sections: Vec<(&str, Vec<&SettingDef>)> = Vec::new();
    for def in schema {
        match sections.iter_mut().find(|(s, _)| *s == def.section) {
            Some((_, defs)) => defs.push(def),
            None => sections.push((&def.section, vec![def])),
        }
    }
    sections
        .into_iter()
        .map(|(section, defs)| {
            let placement = if defs
                .iter()
                .all(|d| d.process == Process::App && d.key.starts_with(&prefix))
            {
                Placement::App
            } else if let Some(screen) = screens.iter().find(|s| s.eq_ignore_ascii_case(section)) {
                Placement::Screen((*screen).to_owned())
            } else {
                Placement::Configuration
            };
            PageSpec {
                section: section.to_owned(),
                placement,
                scopes: SCOPES
                    .into_iter()
                    .filter(|scope| defs.iter().any(|d| d.allows(*scope)))
                    .collect(),
                keys: defs.iter().map(|d| d.key.clone()).collect(),
            }
        })
        .collect()
}

/// One drafted change to a key. Its `Debug` never shows the text or the value: a draft may
/// hold a typed secret.
#[derive(Clone, PartialEq)]
pub(crate) enum Edit {
    /// Set the key to the typed text.
    Set {
        /// What was typed.
        text: String,
        /// The text as a value, or why it is not one.
        parsed: Result<Value, String>,
    },
    /// Remove the key from this scope.
    Unset,
}

impl fmt::Debug for Edit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Edit::Set { parsed, .. } => f
                .debug_struct("Set")
                .field("text", &format_args!("<redacted>"))
                .field(
                    "parsed",
                    &format_args!(
                        "{}",
                        if parsed.is_ok() {
                            "Ok(<redacted>)"
                        } else {
                            "Err(<redacted>)"
                        }
                    ),
                )
                .finish(),
            Edit::Unset => f.write_str("Unset"),
        }
    }
}

/// Parse typed text as a value of `kind`. A list takes one item per line, blank lines
/// dropped and items trimmed; a secret takes any non-empty text.
pub(crate) fn parse_text(kind: &Kind, text: &str) -> Result<Value, String> {
    match kind {
        Kind::Secret => {
            if text.is_empty() {
                Err("must not be empty".into())
            } else {
                Ok(Value::String(text.to_owned()))
            }
        }
        Kind::List { item } => {
            let items = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| parse_text(item, line))
                .collect::<Result<Vec<_>, _>>()?;
            let value = Value::List(items);
            kind.check(&value)?;
            Ok(value)
        }
        other => other.parse(text),
    }
}

/// What a person has typed into one tab of one page. Its `Debug` shows the keys and the
/// edits (redacted), never a value.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Draft {
    /// The drafted changes by key.
    pub edits: BTreeMap<String, Edit>,
    /// What this scope held for each drafted key when it was first seen with a draft; a
    /// reload that finds something else marks the key "changed elsewhere".
    pub base: BTreeMap<String, Option<EntryValue>>,
}

impl fmt::Debug for Draft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Draft")
            .field("edits", &self.edits)
            .field("base", &self.base.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// What `scope`'s file holds for `key`.
fn server_value(state: &ConfigState, scope: Scope, key: &str) -> Option<EntryValue> {
    state
        .by_scope
        .get(&scope)?
        .iter()
        .find(|e| e.key == key)
        .map(|e| e.value.clone())
}

impl Draft {
    /// Draft `key` as the typed `text`. A secret with no text is not drafted: it enters the
    /// draft only once typed into.
    pub(crate) fn set(&mut self, key: &str, kind: &Kind, text: &str) {
        if matches!(kind, Kind::Secret) && text.is_empty() {
            self.edits.remove(key);
            self.base.remove(key);
            return;
        }
        self.edits.insert(
            key.to_owned(),
            Edit::Set {
                text: text.to_owned(),
                parsed: parse_text(kind, text),
            },
        );
    }

    /// Draft removing `key` from this scope.
    pub(crate) fn unset(&mut self, key: &str) {
        self.edits.insert(key.to_owned(), Edit::Unset);
    }

    /// Drop `key`'s edit, keeping the rest of the draft.
    pub(crate) fn forget(&mut self, key: &str) {
        self.edits.remove(key);
        self.base.remove(key);
    }

    /// Drop the whole draft.
    pub(crate) fn revert(&mut self) {
        self.edits.clear();
        self.base.clear();
    }

    /// Whether nothing is drafted.
    pub(crate) fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Whether every drafted text parses.
    pub(crate) fn valid(&self) -> bool {
        self.edits
            .values()
            .all(|e| !matches!(e, Edit::Set { parsed: Err(_), .. }))
    }

    /// Remember what `scope` holds now for each drafted key not yet remembered. Call it
    /// after an edit, before the next reload.
    pub(crate) fn observe(&mut self, state: &ConfigState, scope: Scope) {
        for key in self.edits.keys() {
            self.base
                .entry(key.clone())
                .or_insert_with(|| server_value(state, scope, key));
        }
    }

    /// The drafted keys whose value in `scope` differs from what it was when first seen.
    /// The draft itself is kept.
    pub(crate) fn on_reload(&self, state: &ConfigState, scope: Scope) -> BTreeSet<String> {
        self.edits
            .keys()
            .filter(|key| {
                self.base
                    .get(*key)
                    .is_some_and(|base| *base != server_value(state, scope, key))
            })
            .cloned()
            .collect()
    }

    /// The write the draft makes: the entries to set and the keys to unset. `None` while
    /// any text does not parse.
    pub(crate) fn batch(&self, _scope: Scope) -> Option<(Vec<ConfigEntry>, Vec<String>)> {
        let mut set = Vec::new();
        let mut unset = Vec::new();
        for (key, edit) in &self.edits {
            match edit {
                Edit::Set {
                    parsed: Ok(value), ..
                } => set.push(ConfigEntry {
                    key: key.clone(),
                    value: value.clone(),
                }),
                Edit::Set { parsed: Err(_), .. } => return None,
                Edit::Unset => unset.push(key.clone()),
            }
        }
        Some((set, unset))
    }
}

/// Every draft, by page (section) and tab (scope). A draft outlives its page being hidden
/// and its tab being disabled. Its `Debug` is [`Draft`]'s: no value.
#[derive(Default)]
pub(crate) struct Drafts(pub HashMap<(String, Scope), Draft>);

impl fmt::Debug for Drafts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.0.iter()).finish()
    }
}

impl Drafts {
    /// The draft of a tab, made empty when there is none.
    pub(crate) fn get_mut(&mut self, section: &str, scope: Scope) -> &mut Draft {
        self.0.entry((section.to_owned(), scope)).or_default()
    }

    /// The draft of a tab, when there is one.
    pub(crate) fn get(&self, section: &str, scope: Scope) -> Option<&Draft> {
        self.0.get(&(section.to_owned(), scope))
    }

    /// The pages with an unsaved draft.
    pub(crate) fn dirty_sections(&self) -> BTreeSet<String> {
        self.0
            .iter()
            .filter(|(_, d)| !d.is_empty())
            .map(|((section, _), _)| section.clone())
            .collect()
    }
}

/// The note on a value that does not come from the tab's scope; `None` when it does.
pub(crate) fn origin_label(origin: Origin, tab: Scope) -> Option<&'static str> {
    match origin {
        Origin::Scope(scope) if scope == tab => None,
        Origin::Scope(Scope::Device) => Some("from device"),
        Origin::Scope(Scope::Workgroup) => Some("from workgroup"),
        Origin::Scope(Scope::Workspace) => Some("from workspace"),
        Origin::Default => Some("default"),
    }
}

/// Split a failed write: the field errors for keys on the page, by key, and the lines for
/// everything else (the message, then the field errors for other keys).
pub(crate) fn route_errors(
    err: &CommandError,
    keys: &[String],
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut fields = BTreeMap::new();
    let mut lines = Vec::new();
    if !err.message.is_empty() {
        lines.push(err.message.clone());
    }
    for field in &err.fields {
        if keys.contains(&field.key) {
            fields.insert(field.key.clone(), field.message.clone());
        } else {
            lines.push(format!("{}: {}", field.key, field.message));
        }
    }
    (fields, lines)
}

/// The keys awaiting a restart, by the process that owns them.
pub(crate) fn pending_restarts(state: &ConfigState) -> BTreeMap<Process, Vec<String>> {
    let mut out: BTreeMap<Process, Vec<String>> = BTreeMap::new();
    for Entry {
        key,
        pending_restart,
        ..
    } in &state.effective
    {
        if !pending_restart {
            continue;
        }
        if let Some(def) = state.schema.iter().find(|d| d.key == *key) {
            out.entry(def.process).or_default().push(key.clone());
        }
    }
    out
}

/// Whether a tab can be used: the workspace tab needs a selected workspace.
pub(crate) fn tab_enabled(scope: Scope, workspace_selected: bool) -> bool {
    scope != Scope::Workspace || workspace_selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use sapphire_bridge_api::ConfigFieldError;
    use sapphire_bridge_api::settings::{Apply, SecretState};

    fn def(key: &str, section: &str, process: Process, scopes: &[Scope], kind: Kind) -> SettingDef {
        SettingDef {
            key: key.into(),
            kind,
            scopes: scopes.to_vec(),
            overridable: false,
            apply: Apply::Live,
            default: None,
            section: section.into(),
            doc: String::new(),
            process,
        }
    }

    fn string() -> Kind {
        Kind::String { non_empty: false }
    }

    fn entry(key: &str, value: EntryValue, pending: bool) -> Entry {
        Entry {
            key: key.into(),
            value,
            origin: Origin::Scope(Scope::Device),
            pending_restart: pending,
            note: None,
        }
    }

    fn schema() -> Vec<SettingDef> {
        vec![
            def(
                "notes.color",
                "Notes",
                Process::App,
                &[Scope::Device],
                string(),
            ),
            def(
                "embed.model",
                "embedding",
                Process::Bridge,
                &[Scope::Device],
                string(),
            ),
            def(
                "sync.name",
                "Sync",
                Process::Bridge,
                &[Scope::Workgroup],
                string(),
            ),
            def(
                "notes.shared",
                "Notes",
                Process::App,
                &[Scope::Workspace],
                string(),
            ),
            def("notes.x", "Mixed", Process::App, &[Scope::Device], string()),
            def(
                "sync.y",
                "Mixed",
                Process::Bridge,
                &[Scope::Workgroup],
                string(),
            ),
        ]
    }

    #[test]
    fn pages_follow_the_schema_order_and_placement() {
        let p = pages(&schema(), "notes", &["Embedding", "Sync"]);
        let got: Vec<_> = p
            .iter()
            .map(|p| (p.section.as_str(), p.placement.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Notes", Placement::App),
                ("embedding", Placement::Screen("Embedding".into())),
                ("Sync", Placement::Screen("Sync".into())),
                ("Mixed", Placement::Configuration),
            ]
        );
        assert_eq!(p[0].keys, ["notes.color", "notes.shared"]);
        // A bridge-owned section is not the app's even with the prefix.
        let other = pages(&schema(), "sync", &[]);
        assert_eq!(other[2].placement, Placement::Configuration);
        // Nor is the app process's own section without it: the server's `http.*`.
        let mut s = schema();
        s.push(def(
            "http.listen",
            "Server",
            Process::App,
            &[Scope::Device],
            string(),
        ));
        let server = pages(&s, "notes", &[]).pop().unwrap();
        assert_eq!(
            (server.section.as_str(), server.placement),
            ("Server", Placement::Configuration)
        );
    }

    #[test]
    fn tabs_are_the_scopes_the_keys_allow_in_order() {
        let p = pages(&schema(), "notes", &[]);
        assert_eq!(p[0].scopes, [Scope::Device, Scope::Workspace]);
        assert_eq!(p[3].scopes, [Scope::Device, Scope::Workgroup]);
        let mut s = schema();
        s[1].overridable = true;
        assert_eq!(pages(&s, "notes", &[])[1].scopes, [Scope::Device]);
        s[2].overridable = true;
        assert_eq!(
            pages(&s, "notes", &[])[2].scopes,
            [Scope::Device, Scope::Workgroup]
        );
    }

    #[test]
    fn list_text_drops_blank_lines_and_checks_each_item() {
        let kind = Kind::List {
            item: Box::new(Kind::Int { min: 1, max: 9 }),
        };
        assert_eq!(
            parse_text(&kind, "1\n\n  2 \n3\n"),
            Ok(Value::List(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3)
            ]))
        );
        assert_eq!(parse_text(&kind, ""), Ok(Value::List(vec![])));
        assert!(parse_text(&kind, "1\nx").is_err());
        assert!(parse_text(&kind, "1\n10").is_err());
        assert_eq!(parse_text(&Kind::Bool, "on"), Ok(Value::Bool(true)));
    }

    #[test]
    fn a_secret_is_in_the_draft_only_once_typed() {
        let mut d = Draft::default();
        d.set("k", &Kind::Secret, "");
        assert!(d.is_empty());
        d.set("k", &Kind::Secret, "hunter2");
        assert!(!d.is_empty() && d.valid());
        d.set("k", &Kind::Secret, "");
        assert!(d.is_empty());
    }

    #[test]
    fn a_reload_keeps_a_draft_and_marks_a_changed_value() {
        let mut state = ConfigState::default();
        state.by_scope.insert(
            Scope::Device,
            vec![entry(
                "a",
                EntryValue::Value(Value::String("1".into())),
                false,
            )],
        );
        let mut d = Draft::default();
        d.set("a", &string(), "mine");
        d.set("b", &string(), "new");
        d.observe(&state, Scope::Device);
        assert!(d.on_reload(&state, Scope::Device).is_empty());

        state.by_scope.insert(
            Scope::Device,
            vec![entry(
                "a",
                EntryValue::Value(Value::String("2".into())),
                false,
            )],
        );
        let changed = d.on_reload(&state, Scope::Device);
        assert_eq!(changed.into_iter().collect::<Vec<_>>(), ["a"]);
        assert_eq!(d.edits.len(), 2, "a reload never discards a draft");

        // A key someone else set meanwhile also counts.
        state.by_scope.get_mut(&Scope::Device).unwrap().push(entry(
            "b",
            EntryValue::Secret(SecretState::Set),
            false,
        ));
        assert_eq!(d.on_reload(&state, Scope::Device).len(), 2);
        let mut drafts = Drafts::default();
        *drafts.get_mut("S", Scope::Device) = d;
        assert_eq!(
            drafts.dirty_sections().into_iter().collect::<Vec<_>>(),
            ["S"]
        );
        drafts.get_mut("S", Scope::Device).revert();
        assert!(drafts.dirty_sections().is_empty());
    }

    #[test]
    fn debug_never_shows_a_typed_value() {
        let mut state = ConfigState::default();
        state.by_scope.insert(
            Scope::Device,
            vec![entry(
                "plain",
                EntryValue::Value(Value::String("visible-value".into())),
                false,
            )],
        );
        let mut drafts = Drafts::default();
        let d = drafts.get_mut("S", Scope::Device);
        d.set("token", &Kind::Secret, "hunter2-secret");
        d.set("plain", &string(), "typed-text");
        d.set("n", &Kind::Int { min: 0, max: 1 }, "typed-int-99");
        d.observe(&state, Scope::Device);
        let shown = format!("{drafts:?} {:?}", drafts.get("S", Scope::Device));
        for value in [
            "hunter2-secret",
            "typed-text",
            "typed-int-99",
            "visible-value",
        ] {
            assert!(!shown.contains(value), "{value} leaked into {shown}");
        }
        assert!(shown.contains("token") && shown.contains("<redacted>"));
    }

    #[test]
    fn observe_keeps_the_first_base() {
        let mut state = ConfigState::default();
        let one = |v: &str| {
            vec![entry(
                "a",
                EntryValue::Value(Value::String(v.into())),
                false,
            )]
        };
        state.by_scope.insert(Scope::Device, one("1"));
        let mut d = Draft::default();
        d.set("a", &string(), "mine");
        d.observe(&state, Scope::Device);
        state.by_scope.insert(Scope::Device, one("2"));
        d.set("a", &string(), "mine again");
        d.observe(&state, Scope::Device);
        assert_eq!(
            d.base["a"],
            Some(EntryValue::Value(Value::String("1".into()))),
            "a later edit does not move the base"
        );
        assert_eq!(d.on_reload(&state, Scope::Device).len(), 1);
    }

    #[test]
    fn forget_drops_one_key_and_its_base() {
        let mut d = Draft::default();
        d.unset("a");
        d.set("b", &Kind::Bool, "on");
        d.observe(&ConfigState::default(), Scope::Device);
        d.forget("a");
        assert_eq!(d.edits.keys().collect::<Vec<_>>(), ["b"]);
        assert_eq!(d.base.keys().collect::<Vec<_>>(), ["b"]);
        d.forget("b");
        assert!(d.is_empty() && d.base.is_empty());
    }

    #[test]
    fn batch_splits_set_and_unset_and_refuses_an_invalid_draft() {
        let mut d = Draft::default();
        d.set("a", &Kind::Bool, "on");
        d.unset("b");
        let (set, unset) = d.batch(Scope::Device).unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(
            (set[0].key.as_str(), &set[0].value),
            ("a", &Value::Bool(true))
        );
        assert_eq!(unset, ["b"]);
        d.set("c", &Kind::Int { min: 0, max: 1 }, "7");
        assert!(!d.valid());
        assert!(d.batch(Scope::Device).is_none());
    }

    #[test]
    fn origin_labels() {
        let dev = Origin::Scope(Scope::Device);
        assert_eq!(origin_label(dev, Scope::Device), None);
        assert_eq!(origin_label(dev, Scope::Workspace), Some("from device"));
        assert_eq!(
            origin_label(Origin::Scope(Scope::Workgroup), Scope::Device),
            Some("from workgroup")
        );
        assert_eq!(
            origin_label(Origin::Scope(Scope::Workspace), Scope::Device),
            Some("from workspace")
        );
        assert_eq!(
            origin_label(Origin::Default, Scope::Device),
            Some("default")
        );
    }

    #[test]
    fn field_errors_go_to_their_fields_and_the_rest_to_lines() {
        let err = CommandError {
            message: "rejected".into(),
            fields: vec![
                ConfigFieldError {
                    key: "a".into(),
                    message: "bad".into(),
                },
                ConfigFieldError {
                    key: "z".into(),
                    message: "worse".into(),
                },
            ],
        };
        let (fields, lines) = route_errors(&err, &["a".into(), "b".into()]);
        assert_eq!(fields.get("a").map(String::as_str), Some("bad"));
        assert_eq!(fields.len(), 1);
        assert_eq!(lines, ["rejected", "z: worse"]);
    }

    #[test]
    fn pending_restarts_by_process() {
        let mut state = ConfigState {
            schema: schema(),
            ..Default::default()
        };
        state.effective = vec![
            entry("notes.color", EntryValue::Unset, true),
            entry("embed.model", EntryValue::Unset, true),
            entry("sync.name", EntryValue::Unset, false),
            entry("notes.x", EntryValue::Unset, true),
            entry("nope", EntryValue::Unset, true),
        ];
        let p = pending_restarts(&state);
        assert_eq!(p[&Process::App], ["notes.color", "notes.x"]);
        assert_eq!(p[&Process::Bridge], ["embed.model"]);
    }

    #[test]
    fn a_workspace_draft_waits_for_a_workspace() {
        assert!(tab_enabled(Scope::Device, false));
        assert!(tab_enabled(Scope::Workgroup, false));
        assert!(!tab_enabled(Scope::Workspace, false));
        assert!(tab_enabled(Scope::Workspace, true));
        let mut drafts = Drafts::default();
        drafts.get_mut("S", Scope::Workspace).unset("k");
        // The workspace goes away: the draft is still there, and the page still marked.
        assert!(!tab_enabled(Scope::Workspace, false));
        assert_eq!(drafts.dirty_sections().len(), 1);
        assert!(!drafts.get("S", Scope::Workspace).unwrap().is_empty());
    }
}
