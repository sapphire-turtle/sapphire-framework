//! The app server's settings (#314): the framework's keys and the application's, in one
//! registry.
//!
//! The framework owns `http.*` and `update.check`, all on the device and all read once at
//! start. The application owns the keys under its own name, on the device or in the
//! workspace; it says what they are ([`AppSettings`]) and the framework says where they
//! live:
//!
//! | Scope | File |
//! |---|---|
//! | Device | `<config dir>/config.toml` |
//! | Workspace | `{root}/.{app}/config.toml`, of the workspace selected at the time |
//! | Secrets | `<config dir>/secrets/<key>`, owner-only |
//!
//! `config.toml` replaces `server.toml`, whose layout it keeps: [`migrate`] moves the old
//! file over once.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use sapphire_framework_settings::{
    Apply, FieldError, Kind, Process, Registry, Scope, SettingDef, SettingsOwner, Staged,
    StartState, Value,
};
use sapphire_workspace::AppContext;
use tokio::sync::watch;

use crate::config::{MCP_MOVED, SERVER_CONFIG_FILE};
use crate::error::{Error, Result};
use crate::selection::SelectionFile;

/// The file `config.toml` replaces, and what it is renamed to once migrated.
const OLD_CONFIG_FILE: &str = "server.toml";
const OLD_CONFIG_BACKUP: &str = "server.toml.v1";

/// What `workspace()` says with no workspace to put the file in.
const NO_WORKSPACE: &str = "no workspace is selected";

/// Top-level names the framework keeps for itself, whether or not it defines keys there.
const FRAMEWORK_NAMES: [&str; 5] = ["http", "update", "sync", "retrieve", "workspace"];

/// The bridge's top-level names: owned by no one here, so no key of this server takes them.
const BRIDGE_NAMES: [&str; 2] = ["net", "embed"];

/// An application's own settings: what they are, and a check of the edited files as a
/// whole.
///
/// Every key starts with the application's name and a dot, lives on the device or in the
/// workspace (not in the workgroup), and a secret lives on the device only. Where the files
/// are is the framework's business. `()` is an application with no settings of its own.
pub trait AppSettings: Send + Sync + 'static {
    /// The application's settings, in the order a listing shows them.
    fn defs(&self) -> Vec<SettingDef>;

    /// Check the edited files as a whole, before anything is written; also run once when
    /// the server starts, against the files as they are.
    fn validate(&self, _staged: &Staged) -> std::result::Result<(), Vec<FieldError>> {
        Ok(())
    }
}

impl AppSettings for () {
    fn defs(&self) -> Vec<SettingDef> {
        Vec::new()
    }
}

/// Where the app server's settings live.
#[derive(Clone, Debug)]
pub(crate) struct SettingsFiles {
    config_dir: PathBuf,
    app_name: &'static str,
    selection: SelectionFile,
}

impl SettingsFiles {
    /// `ctx`'s application's files; an error while its config directory is unresolved.
    pub(crate) fn for_app(ctx: &AppContext) -> Result<SettingsFiles> {
        let config_dir = ctx.try_config_dir().ok_or_else(|| {
            Error::Config(format!(
                "{} has no config directory, so no settings",
                ctx.app_name
            ))
        })?;
        Ok(SettingsFiles {
            config_dir: config_dir.to_path_buf(),
            app_name: ctx.app_name,
            selection: SelectionFile::for_app(ctx),
        })
    }

    /// The device's file.
    pub(crate) fn device(&self) -> PathBuf {
        self.config_dir.join(SERVER_CONFIG_FILE)
    }

    /// The selected workspace's file, as the selection is now: a selection that changes
    /// moves the workspace scope with it.
    pub(crate) fn workspace(&self) -> Result<PathBuf> {
        match self.selection.load(self.app_name)? {
            Some(selection) => Ok(selection
                .root
                .join(format!(".{}", self.app_name))
                .join("config.toml")),
            None => Err(Error::Config(NO_WORKSPACE.to_owned())),
        }
    }

    /// The owner-only file of the secret `key`.
    pub(crate) fn secret(&self, key: &str) -> PathBuf {
        self.config_dir.join("secrets").join(key)
    }

    /// The file of `scope`, as an owner answers it.
    fn scope_file(&self, scope: Scope) -> sapphire_framework_settings::Result<PathBuf> {
        match scope {
            Scope::Device => Ok(self.device()),
            Scope::Workspace => self
                .workspace()
                .map_err(|e| std::io::Error::other(e.to_string()).into()),
            Scope::Workgroup => Err(std::io::Error::other(format!(
                "the {} server has no workgroup scope",
                self.app_name
            ))
            .into()),
        }
    }
}

/// The framework's settings, in the order a listing shows them.
fn framework_defs() -> Vec<SettingDef> {
    let def = |key: &str, kind: Kind, default: Option<Value>, doc: &str| SettingDef {
        key: key.into(),
        kind,
        scopes: vec![Scope::Device],
        overridable: false,
        apply: Apply::Restart,
        default,
        section: "Server".into(),
        doc: doc.into(),
        process: Process::App,
    };
    vec![
        def(
            "http.listen",
            Kind::SocketAddr,
            None,
            "Where the HTTP listener binds; unset = the application's default address",
        ),
        def(
            "http.allowed_hosts",
            Kind::List {
                item: Box::new(Kind::String { non_empty: true }),
            },
            None,
            "The Host names clients reach the server by, beyond loopback",
        ),
        def(
            "http.mcp.enabled",
            Kind::Bool,
            Some(Value::Bool(false)),
            "Serve the application's MCP tools at /mcp, behind an external device's token",
        ),
        def(
            "http.metrics.enabled",
            Kind::Bool,
            Some(Value::Bool(false)),
            "Serve Prometheus metrics at /metrics, without a token",
        ),
        def(
            "update.check",
            Kind::Bool,
            Some(Value::Bool(true)),
            "Look for newer releases of the application, at start and once a day",
        ),
    ]
}

/// The framework as a settings owner.
struct FrameworkOwner {
    files: SettingsFiles,
}

impl SettingsOwner for FrameworkOwner {
    fn defs(&self) -> Vec<SettingDef> {
        framework_defs()
    }

    fn file(&self, scope: Scope) -> sapphire_framework_settings::Result<PathBuf> {
        self.files.scope_file(scope)
    }

    fn secret_file(&self, key: &str) -> sapphire_framework_settings::Result<PathBuf> {
        Ok(self.files.secret(key))
    }

    fn validate(&self, staged: &Staged) -> std::result::Result<(), Vec<FieldError>> {
        check_device_file(&self.files, staged)
    }
}

/// The application as a settings owner, in the framework's files.
struct AppOwner {
    files: SettingsFiles,
    app: Arc<dyn AppSettings>,
}

impl SettingsOwner for AppOwner {
    fn defs(&self) -> Vec<SettingDef> {
        self.app
            .defs()
            .into_iter()
            .map(|def| SettingDef {
                process: Process::App,
                ..def
            })
            .collect()
    }

    fn file(&self, scope: Scope) -> sapphire_framework_settings::Result<PathBuf> {
        self.files.scope_file(scope)
    }

    fn secret_file(&self, key: &str) -> sapphire_framework_settings::Result<PathBuf> {
        Ok(self.files.secret(key))
    }

    fn validate(&self, staged: &Staged) -> std::result::Result<(), Vec<FieldError>> {
        self.app.validate(staged)
    }
}

/// The app server's registry: the framework (owner 0) and the application (owner 1).
///
/// An application key outside `<app_name>.`, or one the workgroup may hold, is
/// [`InvalidDef`](sapphire_framework_settings::Error::InvalidDef).
pub(crate) fn registry(
    ctx: &AppContext,
    files: &SettingsFiles,
    app: Arc<dyn AppSettings>,
) -> sapphire_framework_settings::Result<Registry> {
    let prefix = format!("{}.", ctx.app_name);
    for def in app.defs() {
        let invalid = |message: String| sapphire_framework_settings::Error::InvalidDef {
            key: def.key.clone(),
            message,
        };
        if !def.key.starts_with(&prefix) {
            return Err(invalid(format!(
                "an application's key starts with `{prefix}`"
            )));
        }
        if def.scopes.contains(&Scope::Workgroup) {
            return Err(invalid(
                "an application's key lives on the device or in the workspace, not in the workgroup"
                    .to_owned(),
            ));
        }
    }
    // No owner has this index: the bridge's names are reserved against both.
    const NO_OWNER: usize = usize::MAX;
    let mut reserved: Vec<(&str, usize)> = FRAMEWORK_NAMES.iter().map(|n| (*n, 0)).collect();
    reserved.push((ctx.app_name, 1));
    reserved.extend(BRIDGE_NAMES.iter().map(|n| (*n, NO_OWNER)));
    Registry::new(
        vec![
            Arc::new(FrameworkOwner {
                files: files.clone(),
            }),
            Arc::new(AppOwner {
                files: files.clone(),
                app,
            }),
        ],
        &reserved,
    )
}

/// The device file's framework part: the old `[mcp]` table, unknown keys under the
/// framework's names or at the top, and framework values of the wrong kind.
///
/// Values are never quoted back: the message names the key and the file only.
fn check_device_file(
    files: &SettingsFiles,
    staged: &Staged,
) -> std::result::Result<(), Vec<FieldError>> {
    let path = files.device();
    let failure = |key: Option<String>, message: String| FieldError {
        key,
        file: Some(path.clone()),
        message,
    };
    let text = match staged.read(&path) {
        Ok(Some(text)) => text,
        Ok(None) => return Ok(()),
        Err(e) => return Err(vec![failure(None, e.to_string())]),
    };
    let table = match text.parse::<toml::Table>() {
        Ok(table) => table,
        Err(e) => return Err(vec![failure(None, e.to_string())]),
    };
    let defs = framework_defs();
    let mut errors = Vec::new();
    for (name, item) in &table {
        if name == "mcp" {
            errors.push(failure(None, MCP_MOVED.to_owned()));
        } else if FRAMEWORK_NAMES.contains(&name.as_str()) {
            let mut leaves = Vec::new();
            leaves_of(name.clone(), item, &mut leaves);
            for (key, value) in leaves {
                let in_file = format!("in {}", path.display());
                match defs.iter().find(|d| d.key == key) {
                    None => errors.push(failure(Some(key), format!("unknown setting {in_file}"))),
                    Some(def) => {
                        if !accepts(&def.kind, value) {
                            errors.push(failure(
                                Some(key),
                                format!("the value {in_file} is not a valid one"),
                            ));
                        }
                    }
                }
            }
        } else if name != files.app_name {
            errors.push(failure(
                Some(name.clone()),
                format!("unknown setting in {}", path.display()),
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Every non-table value under `item`, by its dotted path.
fn leaves_of<'a>(path: String, item: &'a toml::Value, out: &mut Vec<(String, &'a toml::Value)>) {
    match item {
        toml::Value::Table(table) => {
            for (name, child) in table {
                leaves_of(format!("{path}.{name}"), child, out);
            }
        }
        other => out.push((path, other)),
    }
}

/// Whether a TOML value is one `kind` accepts.
fn accepts(kind: &Kind, value: &toml::Value) -> bool {
    to_value(value).is_some_and(|v| kind.check(&v).is_ok())
}

/// A TOML value as a setting value; floats, datetimes and tables are none.
fn to_value(value: &toml::Value) -> Option<Value> {
    match value {
        toml::Value::String(s) => Some(Value::String(s.clone())),
        toml::Value::Integer(i) => Some(Value::Int(*i)),
        toml::Value::Boolean(b) => Some(Value::Bool(*b)),
        toml::Value::Array(items) => items
            .iter()
            .map(to_value)
            .collect::<Option<_>>()
            .map(Value::List),
        _ => None,
    }
}

/// Move `<config_dir>/server.toml` to `config.toml`, text unchanged, keeping the old file
/// as `server.toml.v1`. Nothing happens once `config.toml` exists.
pub(crate) fn migrate(config_dir: &Path) -> Result<()> {
    let new = config_dir.join(SERVER_CONFIG_FILE);
    let old = config_dir.join(OLD_CONFIG_FILE);
    if new.exists() {
        return Ok(());
    }
    let text = match std::fs::read_to_string(&old) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::Io(e)),
    };
    sapphire_framework_settings::write_atomic(&new, &text)?;
    std::fs::rename(&old, config_dir.join(OLD_CONFIG_BACKUP))?;
    tracing::info!(
        "{} moved to {}; the old file is kept as {OLD_CONFIG_BACKUP}",
        old.display(),
        new.display()
    );
    Ok(())
}

/// A Rust type a setting's value reads as, for [`Settings::get_as`].
pub trait FromValue: Sized {
    /// `value` as this type, or `None` when it is of another kind.
    fn from_value(value: Value) -> Option<Self>;
}

impl FromValue for bool {
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }
}

impl FromValue for i64 {
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Int(i) => Some(i),
            _ => None,
        }
    }
}

impl FromValue for String {
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
}

impl FromValue for Vec<String> {
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::List(items) => items.into_iter().map(String::from_value).collect(),
            _ => None,
        }
    }
}

/// The app server's settings as the running server sees them: a cheap, clonable handle.
///
/// Keys that apply on a restart keep the values they had when the server started; keys
/// that apply live follow every write, and [`changed`](Self::changed) announces it.
#[derive(Clone)]
pub struct Settings {
    inner: Arc<Inner>,
}

struct Inner {
    registry: Registry,
    start: StartState,
    /// Effective values, secrets and unset keys left out.
    values: RwLock<BTreeMap<String, Value>>,
    changed: watch::Sender<u64>,
}

impl std::fmt::Debug for Settings {
    // Never the values: they are the user's.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings").finish_non_exhaustive()
    }
}

impl Settings {
    /// Read `ctx`'s application's settings: migrate `server.toml`, build the registry,
    /// check the device file and the application's files, and read every value.
    pub(crate) fn load(ctx: &AppContext, app: Arc<dyn AppSettings>) -> Result<Settings> {
        let files = SettingsFiles::for_app(ctx)?;
        migrate(&files.config_dir)?;
        let registry = registry(ctx, &files, Arc::clone(&app))?;
        let mut errors = Vec::new();
        for result in [
            check_device_file(&files, &Staged::default()),
            app.validate(&Staged::default()),
        ] {
            if let Err(e) = result {
                errors.extend(e);
            }
        }
        if !errors.is_empty() {
            return Err(sapphire_framework_settings::Error::Rejected(errors).into());
        }
        let values = read(&registry)?;
        let start = StartState::capture(&registry)?;
        Ok(Settings {
            inner: Arc::new(Inner {
                registry,
                start,
                values: RwLock::new(values),
                changed: watch::channel(0).0,
            }),
        })
    }

    /// The effective value of `key`: `None` for a secret, an unset key or an unknown one.
    pub fn get(&self, key: &str) -> Option<Value> {
        self.inner
            .values
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }

    /// The effective value of `key` as `T`; `None` as for [`get`](Self::get), or when the
    /// value is of another kind.
    pub fn get_as<T: FromValue>(&self, key: &str) -> Option<T> {
        self.get(key).and_then(T::from_value)
    }

    /// Bumped after every write that changes the effective value of a key that applies
    /// live.
    pub fn changed(&self) -> watch::Receiver<u64> {
        self.inner.changed.subscribe()
    }

    /// Read the files again after a write: live keys take their new values, and
    /// [`changed`](Self::changed) is bumped if any did; restart keys keep their start
    /// values.
    #[allow(dead_code)] // `config.*` (#314) is its first caller.
    pub(crate) fn refresh(&self) -> Result<()> {
        let new = read(&self.inner.registry)?;
        let mut values = self.inner.values.write().unwrap_or_else(|e| e.into_inner());
        let mut any = false;
        for def in self.inner.registry.schema() {
            if def.apply != Apply::Live || values.get(&def.key) == new.get(&def.key) {
                continue;
            }
            any = true;
            match new.get(&def.key) {
                Some(value) => values.insert(def.key, value.clone()),
                None => values.remove(&def.key),
            };
        }
        drop(values);
        if any {
            self.inner.changed.send_modify(|n| *n = n.wrapping_add(1));
        }
        Ok(())
    }

    /// The registry the server reads and writes through.
    #[allow(dead_code)] // `config.*` (#314) is its first caller.
    pub(crate) fn registry(&self) -> &Registry {
        &self.inner.registry
    }

    /// The restart keys' values at start, to tell which writes wait for a restart.
    #[allow(dead_code)] // `config.*` (#314) is its first caller.
    pub(crate) fn start(&self) -> &StartState {
        &self.inner.start
    }
}

/// Every key's effective value, secrets and unset keys left out.
fn read(registry: &Registry) -> Result<BTreeMap<String, Value>> {
    Ok(sapphire_framework_settings::list(registry, None, None)?
        .into_iter()
        .filter_map(|entry| match entry.value {
            sapphire_framework_settings::EntryValue::Value(value) => Some((entry.key, value)),
            _ => None,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use crate::selection::Selection;
    use sapphire_framework_settings::{Op, apply};
    use sapphire_workspace::AppContext;
    use std::path::Path;
    use std::sync::Arc;

    const APP: &str = "settingstest";

    /// A context of its own for each test, its config directory in `tmp`.
    fn context(tmp: &Path) -> AppContext {
        let ctx = AppContext::new(APP);
        let dir = tmp.join("config");
        std::fs::create_dir_all(&dir).unwrap();
        ctx.set_config_dir(dir);
        ctx
    }

    struct TestApp(Vec<SettingDef>);

    impl AppSettings for TestApp {
        fn defs(&self) -> Vec<SettingDef> {
            self.0.clone()
        }
    }

    fn app_def(key: &str, kind: Kind, scopes: Vec<Scope>, apply: Apply) -> SettingDef {
        SettingDef {
            key: key.into(),
            kind,
            scopes,
            overridable: false,
            apply,
            default: None,
            section: "Test".into(),
            doc: "test".into(),
            // Left as the default on purpose: the framework marks the app's defs.
            process: Process::Bridge,
        }
    }

    fn one(def: SettingDef) -> Arc<dyn AppSettings> {
        Arc::new(TestApp(vec![def]))
    }

    fn set(key: &str, value: Value) -> Op {
        Op::Set {
            key: key.into(),
            value,
        }
    }

    fn select(ctx: &AppContext, root: &Path) {
        std::fs::create_dir_all(root).unwrap();
        SelectionFile::for_app(ctx)
            .save(&Selection {
                root: root.to_path_buf(),
                sync: false,
            })
            .unwrap();
    }

    #[test]
    fn server_toml_becomes_config_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let text = "# mine\n[update]\ncheck = false\n";
        std::fs::write(tmp.path().join("server.toml"), text).unwrap();
        migrate(tmp.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("config.toml")).unwrap(),
            text
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("server.toml.v1")).unwrap(),
            text
        );
        assert!(!tmp.path().join("server.toml").exists());
        // Once done, it is done.
        migrate(tmp.path()).unwrap();
        assert!(!tmp.path().join("server.toml").exists());
    }

    #[test]
    fn an_existing_config_toml_is_not_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("server.toml"), "[update]\ncheck = false\n").unwrap();
        std::fs::write(tmp.path().join("config.toml"), "[update]\ncheck = true\n").unwrap();
        migrate(tmp.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("config.toml")).unwrap(),
            "[update]\ncheck = true\n"
        );
        assert!(tmp.path().join("server.toml").exists());
        assert!(!tmp.path().join("server.toml.v1").exists());
    }

    #[test]
    fn a_missing_server_toml_migrates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        migrate(tmp.path()).unwrap();
        assert!(!tmp.path().join("config.toml").exists());
    }

    #[test]
    fn the_old_mcp_table_still_says_where_it_went() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        std::fs::write(ctx.config_dir().join("server.toml"), "[mcp]\nhttp = true\n").unwrap();
        let err = Settings::load(&ctx, Arc::new(()))
            .err()
            .unwrap()
            .to_string();
        assert!(
            err.contains("[mcp] http moved to [http.mcp] enabled"),
            "{err}"
        );
        assert!(
            err.contains(&ctx.config_dir().join("config.toml").display().to_string()),
            "{err}"
        );
    }

    #[test]
    fn server_config_comes_from_the_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        std::fs::write(
            ctx.config_dir().join("config.toml"),
            "[http]\nlisten = \"127.0.0.1:9000\"\nallowed_hosts = [\"box.example\"]\n\n\
             [http.mcp]\nenabled = true\n\n[update]\ncheck = false\n",
        )
        .unwrap();
        let settings = Settings::load(&ctx, Arc::new(())).unwrap();
        let config = ServerConfig::from_settings(&settings).unwrap();
        assert_eq!(config.http.listen, Some("127.0.0.1:9000".parse().unwrap()));
        assert_eq!(config.http.allowed_hosts, vec!["box.example".to_owned()]);
        assert!(config.http.mcp.enabled);
        assert!(!config.http.metrics.enabled);
        assert!(!config.update.check);
    }

    #[test]
    fn a_misspelt_framework_setting_is_an_error_not_a_default() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        std::fs::write(
            ctx.config_dir().join("config.toml"),
            "[http.mcp]\nenabeld = true\n",
        )
        .unwrap();
        let err = Settings::load(&ctx, Arc::new(()))
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("http.mcp.enabeld"), "{err}");
    }

    #[test]
    fn a_framework_value_of_the_wrong_kind_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        std::fs::write(
            ctx.config_dir().join("config.toml"),
            "[http]\nlisten = \"nowhere\"\n",
        )
        .unwrap();
        let err = Settings::load(&ctx, Arc::new(()))
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("http.listen"), "{err}");
    }

    #[test]
    fn an_app_key_outside_its_name_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        let files = SettingsFiles::for_app(&ctx).unwrap();
        let def = app_def("other.x", Kind::Bool, vec![Scope::Device], Apply::Live);
        let err = registry(&ctx, &files, one(def)).err().unwrap();
        assert!(
            matches!(&err, sapphire_framework_settings::Error::InvalidDef { key, .. } if key == "other.x"),
            "{err}"
        );
    }

    #[test]
    fn an_app_key_in_the_workgroup_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        let files = SettingsFiles::for_app(&ctx).unwrap();
        for scopes in [
            vec![Scope::Workgroup],
            vec![Scope::Workspace, Scope::Workgroup],
        ] {
            let def = app_def("settingstest.x", Kind::Bool, scopes, Apply::Live);
            let err = registry(&ctx, &files, one(def)).err().unwrap();
            assert!(
                matches!(&err, sapphire_framework_settings::Error::InvalidDef { key, .. } if key == "settingstest.x"),
                "{err}"
            );
        }
    }

    #[test]
    fn an_app_key_named_http_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        let files = SettingsFiles::for_app(&ctx).unwrap();
        let def = app_def("http.port", Kind::Bool, vec![Scope::Device], Apply::Live);
        assert!(registry(&ctx, &files, one(def)).is_err());
    }

    #[test]
    fn every_def_is_the_app_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        let files = SettingsFiles::for_app(&ctx).unwrap();
        let def = app_def(
            "settingstest.x",
            Kind::Bool,
            vec![Scope::Device],
            Apply::Live,
        );
        let reg = registry(&ctx, &files, one(def)).unwrap();
        let schema = reg.schema();
        assert_eq!(
            schema.iter().map(|d| d.key.as_str()).collect::<Vec<_>>(),
            vec![
                "http.listen",
                "http.allowed_hosts",
                "http.mcp.enabled",
                "http.metrics.enabled",
                "update.check",
                "settingstest.x",
            ]
        );
        assert!(schema.iter().all(|d| d.process == Process::App));
    }

    #[test]
    fn the_workspace_scope_follows_the_selection() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        let def = app_def(
            "settingstest.note",
            Kind::String { non_empty: false },
            vec![Scope::Workspace],
            Apply::Live,
        );
        let settings = Settings::load(&ctx, one(def)).unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        let file = |root: &Path| root.join(format!(".{APP}")).join("config.toml");

        select(&ctx, &a);
        apply(
            settings.registry(),
            None,
            vec![set("settingstest.note", Value::String("first".into()))],
        )
        .unwrap();
        select(&ctx, &b);
        apply(
            settings.registry(),
            None,
            vec![set("settingstest.note", Value::String("second".into()))],
        )
        .unwrap();

        let a_text = std::fs::read_to_string(file(&a)).unwrap();
        let b_text = std::fs::read_to_string(file(&b)).unwrap();
        assert!(
            a_text.contains("first") && !a_text.contains("second"),
            "{a_text}"
        );
        assert!(b_text.contains("second"), "{b_text}");
    }

    #[test]
    fn no_selection_means_no_workspace_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        let files = SettingsFiles::for_app(&ctx).unwrap();
        let err = files.workspace().unwrap_err().to_string();
        assert_eq!(err, "no workspace is selected");

        let def = app_def(
            "settingstest.note",
            Kind::String { non_empty: false },
            vec![Scope::Workspace],
            Apply::Live,
        );
        let settings = Settings::load(&ctx, one(def)).unwrap();
        // Reading falls back to the default; writing says why it cannot.
        assert_eq!(settings.get("settingstest.note"), None);
        let err = apply(
            settings.registry(),
            None,
            vec![set("settingstest.note", Value::String("x".into()))],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("no workspace is selected"), "{err}");
    }

    #[test]
    fn a_live_change_fires_changed_and_a_restart_change_does_not() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        let app = TestApp(vec![
            app_def(
                "settingstest.live",
                Kind::Int { min: 0, max: 10 },
                vec![Scope::Device],
                Apply::Live,
            ),
            app_def(
                "settingstest.slow",
                Kind::Int { min: 0, max: 10 },
                vec![Scope::Device],
                Apply::Restart,
            ),
        ]);
        let settings = Settings::load(&ctx, Arc::new(app)).unwrap();
        let mut changed = settings.changed();

        apply(
            settings.registry(),
            None,
            vec![set("settingstest.slow", Value::Int(3))],
        )
        .unwrap();
        settings.refresh().unwrap();
        assert!(!changed.has_changed().unwrap());
        assert_eq!(
            settings.get("settingstest.slow"),
            None,
            "kept until restart"
        );

        apply(
            settings.registry(),
            None,
            vec![set("settingstest.live", Value::Int(4))],
        )
        .unwrap();
        settings.refresh().unwrap();
        assert!(changed.has_changed().unwrap());
        changed.mark_unchanged();
        assert_eq!(settings.get_as::<i64>("settingstest.live"), Some(4));
        // A clone is the same handle.
        assert_eq!(
            settings.clone().get("settingstest.live"),
            Some(Value::Int(4))
        );

        // Nothing changed, nothing announced.
        settings.refresh().unwrap();
        assert!(!changed.has_changed().unwrap());
    }

    #[test]
    fn an_unparsable_config_names_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        let path = ctx.config_dir().join("config.toml");
        std::fs::write(&path, "this is = = not toml").unwrap();
        let err = Settings::load(&ctx, Arc::new(()))
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains(&path.display().to_string()), "{err}");
    }

    #[test]
    fn a_secret_is_never_read_back_and_lives_under_secrets() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        let def = app_def(
            "settingstest.token",
            Kind::Secret,
            vec![Scope::Device],
            Apply::Live,
        );
        let settings = Settings::load(&ctx, one(def)).unwrap();
        apply(
            settings.registry(),
            None,
            vec![set("settingstest.token", Value::String("hunter2".into()))],
        )
        .unwrap();
        let path = ctx.config_dir().join("secrets").join("settingstest.token");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hunter2");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        settings.refresh().unwrap();
        assert_eq!(settings.get("settingstest.token"), None);
    }

    #[test]
    fn values_convert_to_their_rust_types() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        std::fs::write(
            ctx.config_dir().join("config.toml"),
            "[http]\nallowed_hosts = [\"a\", \"b\"]\n",
        )
        .unwrap();
        let settings = Settings::load(&ctx, Arc::new(())).unwrap();
        assert_eq!(settings.get_as::<bool>("update.check"), Some(true));
        assert_eq!(
            settings.get_as::<Vec<String>>("http.allowed_hosts"),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(
            settings.get_as::<String>("update.check"),
            None,
            "wrong type"
        );
        assert_eq!(settings.get_as::<i64>("http.listen"), None, "unset");
        assert_eq!(settings.get("no.such.key"), None);
    }
}
