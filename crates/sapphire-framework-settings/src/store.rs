//! Writing settings: validated, all-or-nothing batches that keep the files' comments.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use sapphire_framework_settings_api::{Apply, Kind, Scope, SettingDef, Value};
use toml_edit::DocumentMut;

use crate::owner::{SettingsOwner, Staged};
use crate::{Error, Registry, Result};

/// One change in a batch.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Set `key` to `value`.
    Set {
        /// The key.
        key: String,
        /// The new value.
        value: Value,
    },
    /// Remove `key`; a prefix removes every key under it that the target scope allows.
    Unset {
        /// The key, or a prefix of several keys.
        key: String,
    },
}

/// What a batch changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// The concrete keys the batch touched (prefixes expanded).
    pub keys: Vec<String>,
    /// Whether any touched key only takes effect after a restart.
    pub restart: bool,
    /// Whether any touched key takes effect live.
    pub live: bool,
}

/// What to do with a secret's file once the batch is validated.
enum SecretEdit {
    Write(String),
    Remove,
}

/// Apply `ops` to the scope files, all or nothing.
///
/// `scope: None` targets each key's home scope; `Some(s)` must be allowed for every key.
/// Every op is checked and every touched owner validates the edited files before anything
/// is written; then the files are written atomically, then the secrets.
pub fn apply(reg: &Registry, scope: Option<Scope>, ops: Vec<Op>) -> Result<Applied> {
    let mut applied = Applied::default();
    let mut docs: BTreeMap<PathBuf, DocumentMut> = BTreeMap::new();
    let mut secrets: BTreeMap<PathBuf, SecretEdit> = BTreeMap::new();
    let mut owners: Vec<&dyn SettingsOwner> = Vec::new();

    for op in ops {
        let targets: Vec<(&SettingDef, &dyn SettingsOwner, Scope)> = match &op {
            Op::Set { key, .. } => {
                let (def, owner) = reg.def(key)?;
                vec![(def, owner, target_scope(def, scope)?)]
            }
            Op::Unset { key } => expand(reg, scope, key)?,
        };
        for (def, owner, target) in targets {
            if !owners.iter().any(|o| std::ptr::addr_eq(*o, owner)) {
                owners.push(owner);
            }
            match &op {
                Op::Set { value, .. } => {
                    def.kind
                        .check(value)
                        .map_err(|message| Error::InvalidValue {
                            key: def.key.clone(),
                            message,
                        })?;
                    if def.kind == Kind::Secret {
                        let text = match value {
                            Value::String(s) if !s.trim().is_empty() => s.clone(),
                            _ => {
                                return Err(Error::InvalidValue {
                                    key: def.key.clone(),
                                    message: "a secret must be a non-empty string".into(),
                                });
                            }
                        };
                        secrets.insert(owner.secret_file(&def.key)?, SecretEdit::Write(text));
                    } else {
                        let doc = load(&mut docs, &owner.file(target)?, true)?.expect("created");
                        set(doc, &def.key, value)?;
                    }
                }
                Op::Unset { .. } => {
                    if def.kind == Kind::Secret {
                        secrets.insert(owner.secret_file(&def.key)?, SecretEdit::Remove);
                    } else if let Some(doc) = load(&mut docs, &owner.file(target)?, false)? {
                        unset(doc, &def.key);
                    }
                }
            }
            if !applied.keys.contains(&def.key) {
                applied.keys.push(def.key.clone());
            }
            match def.apply {
                Apply::Restart => applied.restart = true,
                Apply::Live => applied.live = true,
            }
        }
    }

    let staged = Staged {
        files: docs
            .iter()
            .map(|(p, d)| (p.clone(), d.to_string()))
            .collect(),
    };
    for owner in owners {
        owner.validate(&staged).map_err(Error::Rejected)?;
    }

    for (path, text) in &staged.files {
        write_atomic(path, text)?;
    }
    for (path, edit) in &secrets {
        match edit {
            SecretEdit::Write(text) => write_private(path, text)?,
            SecretEdit::Remove => match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            },
        }
    }
    Ok(applied)
}

/// The scope a key is written in: the requested one if the key allows it, else (with none
/// requested) its home.
fn target_scope(def: &SettingDef, scope: Option<Scope>) -> Result<Scope> {
    let target = scope.unwrap_or_else(|| def.home());
    if def.allows(target) {
        Ok(target)
    } else {
        Err(Error::ScopeNotAllowed {
            key: def.key.clone(),
            scope: target,
            allowed: def.scopes.clone(),
        })
    }
}

/// The keys an `Unset` of `key` removes, each with its owner and target scope.
fn expand<'a>(
    reg: &'a Registry,
    scope: Option<Scope>,
    key: &str,
) -> Result<Vec<(&'a SettingDef, &'a dyn SettingsOwner, Scope)>> {
    if let Ok((def, owner)) = reg.def(key) {
        return Ok(vec![(def, owner, target_scope(def, scope)?)]);
    }
    let under = reg.under(key);
    if under.is_empty() {
        return Err(Error::UnknownKey(key.to_string()));
    }
    let chosen: Vec<&SettingDef> = match scope {
        Some(s) => under.into_iter().filter(|d| d.allows(s)).collect(),
        None => under,
    };
    if chosen.is_empty() {
        // Nothing under the prefix may live in that scope; name the scopes that could.
        let first = reg.under(key)[0];
        return Err(Error::ScopeNotAllowed {
            key: key.to_string(),
            scope: scope.unwrap_or_else(|| first.home()),
            allowed: first.scopes.clone(),
        });
    }
    let target = match scope {
        Some(s) => s,
        None => {
            let home = chosen[0].home();
            if chosen.iter().any(|d| d.home() != home) {
                return Err(Error::InvalidValue {
                    key: key.to_string(),
                    message: "keys under it live in more than one scope; \
                              name one with --device or --workgroup"
                        .into(),
                });
            }
            home
        }
    };
    chosen
        .into_iter()
        .map(|def| Ok((def, reg.def(&def.key)?.1, target)))
        .collect()
}

/// The editable document of `path`, loaded once. An absent file is an empty document if
/// `create`, else `None`.
fn load<'a>(
    docs: &'a mut BTreeMap<PathBuf, DocumentMut>,
    path: &Path,
    create: bool,
) -> Result<Option<&'a mut DocumentMut>> {
    if !docs.contains_key(path) {
        let doc = match std::fs::read_to_string(path) {
            Ok(text) => text.parse::<DocumentMut>().map_err(|e| Error::Unparsable {
                file: path.to_path_buf(),
                message: e.to_string(),
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !create {
                    return Ok(None);
                }
                DocumentMut::new()
            }
            Err(e) => return Err(e.into()),
        };
        docs.insert(path.to_path_buf(), doc);
    }
    Ok(docs.get_mut(path))
}

fn to_edit(value: &Value) -> toml_edit::Value {
    match value {
        Value::Bool(b) => (*b).into(),
        Value::Int(i) => (*i).into(),
        Value::String(s) => s.as_str().into(),
        Value::List(items) => items
            .iter()
            .map(to_edit)
            .collect::<toml_edit::Array>()
            .into(),
    }
}

/// Set `key` in `doc`, creating the tables on its path and keeping an old value's
/// surrounding comments and spacing.
fn set(doc: &mut DocumentMut, key: &str, value: &Value) -> Result<()> {
    let mut segments: Vec<&str> = key.split('.').collect();
    let last = segments.pop().unwrap_or_default();
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for segment in segments {
        if !table.contains_key(segment) {
            let mut new = toml_edit::Table::new();
            new.set_implicit(true);
            table.insert(segment, toml_edit::Item::Table(new));
        }
        table = table
            .get_mut(segment)
            .and_then(toml_edit::Item::as_table_like_mut)
            .ok_or_else(|| Error::InvalidValue {
                key: key.to_string(),
                message: format!("`{segment}` is already a value, not a table"),
            })?;
    }
    let mut new = to_edit(value);
    if let Some(old) = table.get(last).and_then(toml_edit::Item::as_value) {
        *new.decor_mut() = old.decor().clone();
    }
    table.insert(last, toml_edit::Item::Value(new));
    Ok(())
}

/// Remove `key` from `doc` and the tables it leaves empty.
fn unset(doc: &mut DocumentMut, key: &str) {
    fn go(table: &mut dyn toml_edit::TableLike, segments: &[&str]) {
        match segments {
            [] => {}
            [last] => {
                table.remove(last);
            }
            [head, rest @ ..] => {
                if let Some(child) = table
                    .get_mut(head)
                    .and_then(toml_edit::Item::as_table_like_mut)
                {
                    go(child, rest);
                    if child.is_empty() {
                        table.remove(head);
                    }
                }
            }
        }
    }
    let segments: Vec<&str> = key.split('.').collect();
    go(doc.as_table_mut(), &segments);
}

/// Write `text` to `path` through a temp file and a rename, creating the directory.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    write_file(path, text, false)
}

/// Like [`write_atomic`], but the file is readable by its owner only (0600 on unix).
pub fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    write_file(path, text, true)
}

fn write_file(path: &Path, text: &str, private: bool) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(
        "{}.tmp.{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut file = options.open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner::FieldError;
    use crate::owner::test_support::{TestOwner, def};
    use std::sync::Arc;

    /// A test owner whose `validate` rejects `a.max < a.min` in the workgroup file.
    struct Checked(TestOwner);

    impl SettingsOwner for Checked {
        fn defs(&self) -> Vec<SettingDef> {
            self.0.defs()
        }
        fn file(&self, scope: Scope) -> Result<PathBuf> {
            self.0.file(scope)
        }
        fn secret_file(&self, key: &str) -> Result<PathBuf> {
            self.0.secret_file(key)
        }
        fn validate(&self, staged: &Staged) -> std::result::Result<(), Vec<FieldError>> {
            let path = self.0.file(Scope::Workgroup).unwrap();
            let text = staged.read(&path).unwrap().unwrap_or_default();
            let table: toml::Table = text.parse().unwrap();
            let get = |k: &str| table.get("a")?.get(k)?.as_integer();
            match (get("min"), get("max")) {
                (Some(min), Some(max)) if max < min => Err(vec![FieldError {
                    key: Some("a.max".into()),
                    file: None,
                    message: "max is below min".into(),
                }]),
                _ => Ok(()),
            }
        }
    }

    fn int() -> Kind {
        Kind::Int { min: 0, max: 100 }
    }

    fn setup(defs: Vec<SettingDef>) -> (Arc<Checked>, Registry) {
        let owner = Arc::new(Checked(TestOwner::new(defs)));
        let reg = Registry::new(vec![owner.clone() as Arc<dyn SettingsOwner>], &[]).unwrap();
        (owner, reg)
    }

    fn set_op(key: &str, value: Value) -> Op {
        Op::Set {
            key: key.into(),
            value,
        }
    }

    fn unset_op(key: &str) -> Op {
        Op::Unset { key: key.into() }
    }

    fn wg(owner: &Checked) -> PathBuf {
        owner.0.dir.path().join("wg.toml")
    }

    fn dev(owner: &Checked) -> PathBuf {
        owner.0.dir.path().join("dev.toml")
    }

    #[test]
    fn set_keeps_comments_and_order() {
        let (owner, reg) = setup(vec![def("net.discovery", Kind::Bool)]);
        let before = "# mine\n[net]\n# relays\nrelays = []\ndiscovery = true # on\n";
        owner.0.write("wg.toml", before);
        apply(
            &reg,
            None,
            vec![set_op("net.discovery", Value::Bool(false))],
        )
        .unwrap();
        let after = std::fs::read_to_string(wg(&owner)).unwrap();
        assert_eq!(after, before.replace("true", "false"));
    }

    #[test]
    fn set_creates_missing_tables_and_the_file() {
        let (owner, reg) = setup(vec![def("a.b.c", int())]);
        apply(&reg, None, vec![set_op("a.b.c", Value::Int(3))]).unwrap();
        let table: toml::Table = std::fs::read_to_string(wg(&owner))
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(table["a"]["b"]["c"].as_integer(), Some(3));
    }

    #[test]
    fn lists_are_written_as_inline_arrays() {
        let kind = Kind::List {
            item: Box::new(Kind::Url),
        };
        let (owner, reg) = setup(vec![def("net.relays", kind)]);
        let list = Value::List(vec![Value::String("https://r".into())]);
        apply(&reg, None, vec![set_op("net.relays", list)]).unwrap();
        let text = std::fs::read_to_string(wg(&owner)).unwrap();
        assert!(text.contains("relays = [\"https://r\"]"), "{text}");
    }

    #[test]
    fn unset_removes_the_key_and_empty_tables() {
        let (owner, reg) = setup(vec![def("a.b.c", int())]);
        owner.0.write("wg.toml", "[a.b]\nc = 1\n");
        apply(&reg, None, vec![unset_op("a.b.c")]).unwrap();
        assert_eq!(std::fs::read_to_string(wg(&owner)).unwrap().trim(), "");
    }

    #[test]
    fn unsetting_in_an_absent_file_creates_nothing() {
        let (owner, reg) = setup(vec![def("a.b", int())]);
        apply(&reg, None, vec![unset_op("a.b")]).unwrap();
        assert!(!wg(&owner).exists());
    }

    fn remote_defs() -> Vec<SettingDef> {
        let mut enabled = def("embed.remote.enabled", Kind::Bool);
        enabled.scopes = vec![Scope::Device];
        vec![enabled, def("embed.remote.endpoint", Kind::Url)]
    }

    #[test]
    fn a_prefix_unset_removes_only_the_target_scopes_keys() {
        let (owner, reg) = setup(remote_defs());
        owner
            .0
            .write("dev.toml", "[embed.remote]\nenabled = false\n");
        owner
            .0
            .write("wg.toml", "[embed.remote]\nendpoint = \"https://x\"\n");
        let applied = apply(&reg, Some(Scope::Workgroup), vec![unset_op("embed.remote")]).unwrap();
        assert_eq!(applied.keys, ["embed.remote.endpoint"]);
        assert_eq!(
            std::fs::read_to_string(dev(&owner)).unwrap(),
            "[embed.remote]\nenabled = false\n"
        );
        assert_eq!(std::fs::read_to_string(wg(&owner)).unwrap().trim(), "");
    }

    #[test]
    fn a_prefix_unset_across_scopes_needs_a_scope() {
        let (owner, reg) = setup(remote_defs());
        owner
            .0
            .write("dev.toml", "[embed.remote]\nenabled = false\n");
        let err = apply(&reg, None, vec![unset_op("embed.remote")]).unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }), "{err:?}");
        assert!(err.to_string().contains("--workgroup"));
        assert_eq!(
            std::fs::read_to_string(dev(&owner)).unwrap(),
            "[embed.remote]\nenabled = false\n"
        );
    }

    #[test]
    fn an_unknown_prefix_is_an_unknown_key() {
        let (_owner, reg) = setup(remote_defs());
        let err = apply(&reg, None, vec![unset_op("nope")]).unwrap_err();
        assert!(matches!(err, Error::UnknownKey(_)));
    }

    #[test]
    fn a_rejected_batch_writes_nothing() {
        let (owner, reg) = setup(vec![def("a.min", int()), def("a.max", int())]);
        let before = "# keep\n[a]\nmin = 1\nmax = 9\n";
        owner.0.write("wg.toml", before);
        let err = apply(
            &reg,
            None,
            vec![
                set_op("a.min", Value::Int(10)),
                set_op("a.max", Value::Int(5)),
            ],
        )
        .unwrap_err();
        assert!(matches!(err, Error::Rejected(_)), "{err:?}");
        assert_eq!(std::fs::read_to_string(wg(&owner)).unwrap(), before);
        assert!(!dev(&owner).exists());
    }

    #[test]
    fn a_disallowed_scope_names_the_allowed_ones() {
        let (_owner, reg) = setup(vec![def("a.min", int())]);
        let err = apply(
            &reg,
            Some(Scope::Device),
            vec![set_op("a.min", Value::Int(1))],
        )
        .unwrap_err();
        match &err {
            Error::ScopeNotAllowed { allowed, .. } => {
                assert_eq!(allowed, &[Scope::Workgroup]);
            }
            other => panic!("{other:?}"),
        }
        assert!(err.to_string().contains("workgroup"));
    }

    #[test]
    fn a_value_of_the_wrong_kind_is_refused() {
        let (_owner, reg) = setup(vec![def("a.min", int())]);
        let err = apply(&reg, None, vec![set_op("a.min", Value::String("x".into()))]).unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }));
    }

    #[test]
    fn an_unparsable_file_is_not_overwritten() {
        let (owner, reg) = setup(vec![def("a.min", int())]);
        owner.0.write("wg.toml", "[[[");
        let err = apply(&reg, None, vec![set_op("a.min", Value::Int(1))]).unwrap_err();
        assert!(matches!(err, Error::Unparsable { .. }));
        assert_eq!(std::fs::read_to_string(wg(&owner)).unwrap(), "[[[");
    }

    #[test]
    fn secrets_are_written_private_and_deleted_on_unset() {
        let mut secret = def("embed.remote.token", Kind::Secret);
        secret.scopes = vec![Scope::Device];
        let (owner, reg) = setup(vec![secret]);
        let path = owner.0.dir.path().join("secret-embed.remote.token");

        // Unsetting an absent secret is fine.
        apply(&reg, None, vec![unset_op("embed.remote.token")]).unwrap();

        let empty = apply(
            &reg,
            None,
            vec![set_op("embed.remote.token", Value::String(" ".into()))],
        );
        assert!(matches!(empty, Err(Error::InvalidValue { .. })));
        assert!(!path.exists());

        apply(
            &reg,
            None,
            vec![set_op(
                "embed.remote.token",
                Value::String("hunter2".into()),
            )],
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hunter2");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        apply(&reg, None, vec![unset_op("embed.remote.token")]).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn applied_reports_restart_and_live() {
        let mut restart = def("a.r", int());
        restart.apply = Apply::Restart;
        let (_owner, reg) = setup(vec![restart, def("a.l", int())]);
        let live = apply(&reg, None, vec![set_op("a.l", Value::Int(1))]).unwrap();
        assert_eq!((live.restart, live.live), (false, true));
        let both = apply(
            &reg,
            None,
            vec![set_op("a.l", Value::Int(1)), set_op("a.r", Value::Int(2))],
        )
        .unwrap();
        assert_eq!(both.keys, ["a.l", "a.r"]);
        assert_eq!((both.restart, both.live), (true, true));
    }
}
