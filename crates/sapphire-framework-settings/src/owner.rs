//! The owner trait and the staged-edit types it validates.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use sapphire_framework_settings_api::{Scope, SettingDef};

use crate::Result;

/// A process that has settings: it defines them and says where its scope files live.
pub trait SettingsOwner: Send + Sync {
    /// The settings this owner defines.
    fn defs(&self) -> Vec<SettingDef>;

    /// The `config.toml` holding `scope`'s settings. An error means the owner has no such
    /// scope (the bridge has no workspace).
    fn file(&self, scope: Scope) -> Result<PathBuf>;

    /// The owner-only file holding the secret `key`.
    fn secret_file(&self, key: &str) -> Result<PathBuf>;

    /// Check the edited files as a whole, before anything is written.
    fn validate(&self, staged: &Staged) -> std::result::Result<(), Vec<FieldError>>;

    /// A remark shown beside values that come from `scope`.
    fn scope_note(&self, _scope: Scope) -> Option<String> {
        None
    }
}

/// The edited text of the files a batch touches, before it is written.
#[derive(Clone, Debug, Default)]
pub struct Staged {
    /// Edited file text by path.
    pub files: BTreeMap<PathBuf, String>,
}

impl Staged {
    /// The edited text of a touched file; `None` means untouched (read it from disk).
    pub fn text(&self, path: &Path) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    /// The staged text if the file is touched, else the file on disk, else `None`.
    pub fn read(&self, path: &Path) -> Result<Option<String>> {
        if let Some(text) = self.text(path) {
            return Ok(Some(text.to_string()));
        }
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

/// One problem an owner found while validating.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldError {
    /// The setting at fault, if it is one.
    pub key: Option<String>,
    /// The file at fault, if it is one.
    pub file: Option<PathBuf>,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.key, &self.file) {
            (Some(key), _) => write!(f, "{key}: {}", self.message),
            (None, Some(file)) => write!(f, "{}: {}", file.display(), self.message),
            (None, None) => f.write_str(&self.message),
        }
    }
}

/// A test owner over a temp dir: `dev.toml` for Device, `wg.toml` for Workgroup, no
/// Workspace, and secrets under `secret-<key>`.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::{Apply, Kind, Scope, SettingDef};

    pub struct TestOwner {
        pub dir: tempfile::TempDir,
        pub defs: Vec<SettingDef>,
    }

    impl TestOwner {
        pub fn new(defs: Vec<SettingDef>) -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
                defs,
            }
        }
        pub fn write(&self, name: &str, text: &str) {
            std::fs::write(self.dir.path().join(name), text).unwrap();
        }
    }

    impl SettingsOwner for TestOwner {
        fn defs(&self) -> Vec<SettingDef> {
            self.defs.clone()
        }
        fn file(&self, scope: Scope) -> crate::Result<PathBuf> {
            match scope {
                Scope::Device => Ok(self.dir.path().join("dev.toml")),
                Scope::Workgroup => Ok(self.dir.path().join("wg.toml")),
                Scope::Workspace => Err(crate::Error::Io(std::io::Error::other("no workspace"))),
            }
        }
        fn secret_file(&self, key: &str) -> crate::Result<PathBuf> {
            Ok(self.dir.path().join(format!("secret-{key}")))
        }
        fn validate(&self, _: &Staged) -> std::result::Result<(), Vec<FieldError>> {
            Ok(())
        }
        fn scope_note(&self, scope: Scope) -> Option<String> {
            (scope == Scope::Workgroup).then(|| "wg note".to_string())
        }
    }

    pub fn def(key: &str, kind: Kind) -> SettingDef {
        SettingDef {
            key: key.into(),
            kind,
            scopes: vec![Scope::Workgroup],
            overridable: false,
            apply: Apply::Live,
            default: None,
            section: "test".into(),
            doc: "test".into(),
        }
    }
}
