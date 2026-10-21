//! Reading scope files and resolving each key's effective value.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use sapphire_bridge_api::settings::{
    Apply, Entry, EntryValue, Kind, Origin, Scope, SecretState, SettingDef, Value,
};

use crate::owner::SettingsOwner;
use crate::{Error, Registry, Result};

/// The effective values of the restart-only keys when the process started, to tell which
/// changes are written but not yet in effect.
#[derive(Clone, Debug, Default)]
pub struct StartState(BTreeMap<String, Option<Value>>);

impl StartState {
    /// Capture the current effective value of every `Apply::Restart` key.
    pub fn capture(reg: &Registry) -> Result<StartState> {
        let mut cache = Cache::default();
        let mut values = BTreeMap::new();
        for (def, owner) in reg.entries() {
            if def.apply == Apply::Restart {
                let entry = resolve(def, owner, &mut cache)?;
                values.insert(def.key.clone(), plain(&entry.value));
            }
        }
        Ok(StartState(values))
    }
}

/// The value of an entry, if it has one a restart could change.
fn plain(value: &EntryValue) -> Option<Value> {
    match value {
        EntryValue::Value(v) => Some(v.clone()),
        _ => None,
    }
}

/// Parsed scope files, so one listing reads each file once.
#[derive(Default)]
struct Cache(HashMap<PathBuf, Option<toml::Table>>);

impl Cache {
    /// The parsed table of `path`; a missing file is an empty scope (`None`).
    fn table(&mut self, path: &Path) -> Result<Option<&toml::Table>> {
        if !self.0.contains_key(path) {
            let table = match std::fs::read_to_string(path) {
                Ok(text) => Some(text.parse::<toml::Table>().map_err(|e| Error::Unparsable {
                    file: path.to_path_buf(),
                    message: e.to_string(),
                })?),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            self.0.insert(path.to_path_buf(), table);
        }
        Ok(self.0[path].as_ref())
    }

    /// The value `key` holds in `owner`'s `scope` file.
    fn value(
        &mut self,
        owner: &dyn SettingsOwner,
        scope: Scope,
        key: &str,
    ) -> Result<Option<Value>> {
        let path = owner.file(scope)?;
        let Some(table) = self.table(&path)? else {
            return Ok(None);
        };
        let mut segments = key.split('.');
        let mut current = table;
        let last = segments.next_back().unwrap_or_default();
        for segment in segments {
            match current.get(segment) {
                Some(toml::Value::Table(t)) => current = t,
                _ => return Ok(None),
            }
        }
        current
            .get(last)
            .map(|v| {
                convert(v).map_err(|m| Error::Unparsable {
                    file: path.clone(),
                    message: format!("`{key}`: {m}"),
                })
            })
            .transpose()
    }
}

/// Convert a TOML value; floats, datetimes and tables are not setting values.
fn convert(value: &toml::Value) -> std::result::Result<Value, String> {
    match value {
        toml::Value::String(s) => Ok(Value::String(s.clone())),
        toml::Value::Integer(i) => Ok(Value::Int(*i)),
        toml::Value::Boolean(b) => Ok(Value::Bool(*b)),
        toml::Value::Array(items) => Ok(Value::List(
            items
                .iter()
                .map(convert)
                .collect::<std::result::Result<_, _>>()?,
        )),
        other => Err(format!("unsupported value type `{}`", other.type_str())),
    }
}

/// A secret is set when its file has content.
fn secret_state(owner: &dyn SettingsOwner, key: &str) -> Result<SecretState> {
    match std::fs::read(owner.secret_file(key)?) {
        Ok(bytes) if !bytes.is_empty() => Ok(SecretState::Set),
        Ok(_) => Ok(SecretState::Unset),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SecretState::Unset),
        Err(e) => Err(e.into()),
    }
}

/// Resolve one key: device (if overridable), then the home scope, then the default.
fn resolve(def: &SettingDef, owner: &dyn SettingsOwner, cache: &mut Cache) -> Result<Entry> {
    let entry = |value, origin: Origin| Entry {
        key: def.key.clone(),
        value,
        note: match origin {
            Origin::Scope(scope) => owner.scope_note(scope),
            Origin::Default => None,
        },
        origin,
        pending_restart: false,
    };

    if def.kind == Kind::Secret {
        return Ok(match secret_state(owner, &def.key)? {
            SecretState::Set => entry(
                EntryValue::Secret(SecretState::Set),
                Origin::Scope(Scope::Device),
            ),
            SecretState::Unset => entry(EntryValue::Secret(SecretState::Unset), Origin::Default),
        });
    }

    let mut layers = Vec::new();
    if def.overridable {
        layers.push(Scope::Device);
    }
    if !layers.contains(&def.home()) {
        layers.push(def.home());
    }
    for scope in layers {
        // An owner without this scope (the bridge has no workspace) contributes nothing.
        if owner.file(scope).is_err() {
            continue;
        }
        if let Some(value) = cache.value(owner, scope, &def.key)? {
            return Ok(entry(EntryValue::Value(value), Origin::Scope(scope)));
        }
    }
    Ok(match &def.default {
        Some(value) => entry(EntryValue::Value(value.clone()), Origin::Default),
        None => entry(EntryValue::Unset, Origin::Default),
    })
}

/// Mark a restart-only entry whose value differs from the start.
fn mark_pending(def: &SettingDef, entry: &mut Entry, start: Option<&StartState>) {
    if def.apply != Apply::Restart {
        return;
    }
    if let Some(before) = start.and_then(|s| s.0.get(&def.key)) {
        entry.pending_restart = *before != plain(&entry.value);
    }
}

/// List settings: every key's effective value, or with a scope only what that scope's
/// file holds (shown with the value it holds). `start` marks pending restarts.
pub fn list(
    reg: &Registry,
    scope: Option<Scope>,
    start: Option<&StartState>,
) -> Result<Vec<Entry>> {
    let mut cache = Cache::default();
    let mut entries = Vec::new();
    for (def, owner) in reg.entries() {
        let mut entry = resolve(def, owner, &mut cache)?;
        mark_pending(def, &mut entry, start);
        let Some(scope) = scope else {
            entries.push(entry);
            continue;
        };
        // Two scopes can share a file (a host without a workgroup): a key the scope does
        // not allow is not the scope's, whatever the file holds.
        if !def.allows(scope) {
            continue;
        }
        if def.kind == Kind::Secret {
            if scope == Scope::Device && entry.value == EntryValue::Secret(SecretState::Set) {
                entries.push(entry);
            }
            continue;
        }
        // The owner's error (it has no such scope) is the caller's to see here.
        if let Some(value) = cache.value(owner, scope, &def.key)? {
            entry.value = EntryValue::Value(value);
            entry.origin = Origin::Scope(scope);
            entry.note = owner.scope_note(scope);
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// The effective value of one key.
pub fn get(reg: &Registry, key: &str) -> Result<Entry> {
    let (def, owner) = reg.def(key)?;
    resolve(def, owner, &mut Cache::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner::test_support::{TestOwner, def};
    use crate::{Apply, Error, Kind, Registry, SettingsOwner};
    use std::sync::Arc;

    fn int() -> Kind {
        Kind::Int { min: 0, max: 100 }
    }

    fn setup(d: SettingDef) -> (Arc<TestOwner>, Registry) {
        let owner = Arc::new(TestOwner::new(vec![d]));
        let reg = Registry::new(vec![owner.clone() as Arc<dyn SettingsOwner>], &[]).unwrap();
        (owner, reg)
    }

    #[test]
    fn the_home_scope_wins_over_the_default() {
        let mut d = def("a.b", int());
        d.default = Some(Value::Int(1));
        let (o, reg) = setup(d);
        o.write("wg.toml", "[a]\nb = 3\n");
        let e = get(&reg, "a.b").unwrap();
        assert_eq!(e.value, EntryValue::Value(Value::Int(3)));
        assert_eq!(e.origin, Origin::Scope(Scope::Workgroup));
        assert_eq!(e.note.as_deref(), Some("wg note"));
    }

    #[test]
    fn a_device_value_overrides_only_when_overridable() {
        let mut d = def("a.b", int());
        let (o, reg) = setup(d.clone());
        o.write("wg.toml", "[a]\nb = 3\n");
        o.write("dev.toml", "[a]\nb = 9\n");
        assert_eq!(
            get(&reg, "a.b").unwrap().value,
            EntryValue::Value(Value::Int(3))
        );
        d.overridable = true;
        let (o, reg) = setup(d);
        o.write("wg.toml", "[a]\nb = 3\n");
        o.write("dev.toml", "[a]\nb = 9\n");
        let e = get(&reg, "a.b").unwrap();
        assert_eq!(e.value, EntryValue::Value(Value::Int(9)));
        assert_eq!(e.origin, Origin::Scope(Scope::Device));
    }

    #[test]
    fn unset_without_default_is_unset() {
        let (_o, reg) = setup(def("a.b", int()));
        let e = get(&reg, "a.b").unwrap();
        assert_eq!(e.value, EntryValue::Unset);
        assert_eq!(e.origin, Origin::Default);
    }

    #[test]
    fn a_missing_scope_is_not_an_error_and_unknown_keys_are() {
        let mut d = def("a.b", int());
        d.scopes = vec![Scope::Workspace];
        d.overridable = true;
        let (o, reg) = setup(d);
        o.write("dev.toml", "[a]\nb = 4\n");
        // The owner has no Workspace file, so only the device layer contributes.
        assert_eq!(
            get(&reg, "a.b").unwrap().value,
            EntryValue::Value(Value::Int(4))
        );
        assert!(matches!(
            get(&reg, "x").err().unwrap(),
            Error::UnknownKey(_)
        ));
        assert!(list(&reg, Some(Scope::Workspace), None).is_err());
    }

    #[test]
    fn secrets_report_only_whether_they_are_set() {
        let mut d = def("net.token", Kind::Secret);
        d.scopes = vec![Scope::Device];
        let (o, reg) = setup(d);
        let e = get(&reg, "net.token").unwrap();
        assert_eq!(e.value, EntryValue::Secret(SecretState::Unset));
        o.write("secret-net.token", "hunter2-sekrit");
        let entries = list(&reg, None, None).unwrap();
        assert_eq!(entries[0].value, EntryValue::Secret(SecretState::Set));
        assert!(!format!("{entries:?}").contains("hunter2"));
    }

    #[test]
    fn list_with_a_scope_shows_only_what_it_holds() {
        let owner = Arc::new(TestOwner::new(vec![def("a.b", int()), def("a.c", int())]));
        let reg = Registry::new(vec![owner.clone() as Arc<dyn SettingsOwner>], &[]).unwrap();
        owner.write("wg.toml", "[a]\nb = 3\n");
        let entries = list(&reg, Some(Scope::Workgroup), None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "a.b");
        assert_eq!(list(&reg, None, None).unwrap().len(), 2);
        assert!(list(&reg, Some(Scope::Device), None).unwrap().is_empty());
    }

    #[test]
    fn list_with_a_scope_shows_only_keys_the_scope_allows() {
        // A host without a workgroup keeps both scopes in one file: each scope's listing
        // must still hold only its own keys.
        let mut device_only = def("d.x", int());
        device_only.scopes = vec![Scope::Device];
        let workgroup_only = def("w.y", int());
        let owner = Arc::new(TestOwner::new(vec![device_only, workgroup_only]));
        let reg = Registry::new(vec![owner.clone() as Arc<dyn SettingsOwner>], &[]).unwrap();
        let both = "[d]\nx = 1\n[w]\ny = 2\n";
        owner.write("dev.toml", both);
        owner.write("wg.toml", both);
        let keys = |scope| {
            list(&reg, Some(scope), None)
                .unwrap()
                .into_iter()
                .map(|e| e.key)
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(Scope::Workgroup), vec!["w.y".to_owned()]);
        assert_eq!(keys(Scope::Device), vec!["d.x".to_owned()]);
    }

    #[test]
    fn lists_and_strings_convert_from_toml() {
        let k = Kind::List {
            item: Box::new(Kind::Url),
        };
        let (o, reg) = setup(def("a.urls", k));
        o.write("wg.toml", "[a]\nurls = [\"https://x\", \"https://y\"]\n");
        assert_eq!(
            get(&reg, "a.urls").unwrap().value,
            EntryValue::Value(Value::List(vec![
                Value::String("https://x".into()),
                Value::String("https://y".into())
            ]))
        );
    }

    #[test]
    fn an_unparsable_file_is_an_error_naming_it() {
        let (o, reg) = setup(def("a.b", int()));
        o.write("wg.toml", "this is = = not toml");
        match get(&reg, "a.b").err().unwrap() {
            Error::Unparsable { file, .. } => assert_eq!(file, o.dir.path().join("wg.toml")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_value_of_an_unsupported_type_is_unparsable() {
        let (o, reg) = setup(def("a.b", int()));
        o.write("wg.toml", "[a]\nb = 1.5\n");
        assert!(matches!(
            get(&reg, "a.b").err().unwrap(),
            Error::Unparsable { .. }
        ));
    }

    #[test]
    fn pending_restart_compares_with_the_start() {
        let mut r = def("a.r", int());
        r.apply = Apply::Restart;
        let owner = Arc::new(TestOwner::new(vec![r, def("a.l", int())]));
        let reg = Registry::new(vec![owner.clone() as Arc<dyn SettingsOwner>], &[]).unwrap();
        owner.write("wg.toml", "[a]\nr = 1\nl = 1\n");
        let start = StartState::capture(&reg).unwrap();
        let pending = |reg: &Registry| {
            list(reg, None, Some(&start))
                .unwrap()
                .into_iter()
                .map(|e| (e.key, e.pending_restart))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            pending(&reg),
            vec![("a.r".to_string(), false), ("a.l".to_string(), false)]
        );
        owner.write("wg.toml", "[a]\nr = 2\nl = 2\n");
        assert_eq!(
            pending(&reg),
            vec![("a.r".to_string(), true), ("a.l".to_string(), false)]
        );
    }
}
