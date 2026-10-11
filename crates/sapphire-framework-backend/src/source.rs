//! Choosing a workspace and opening a backend for it.
//!
//! [`WorkspaceLocator`] is a parsed workspace reference. It is a path only:
//! every workspace is local, with zero or more peers (the sync spec §1), so
//! the remote URL form and [`WorkspaceSource::Remote`] variant are gone.
//! [`WorkspaceSource`] holds the opened resource and produces a
//! `Box<dyn WorkspaceBackend>`, so a CLI or GUI can open "a workspace"
//! through a single call site.
//!
//! Opening the underlying [`WorkspaceState`] stays the caller's job — it needs
//! the app's `AppContext` and workspace marker, which are application concerns.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use indexmap::IndexMap;
use sapphire_workspace::WorkspaceState;
use serde::{Deserialize, Serialize};

use crate::{Error, LocalBackend, Result, WorkspaceBackend};

/// A parsed workspace reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkspaceLocator {
    /// A local filesystem workspace root.
    Local(PathBuf),
}

impl WorkspaceLocator {
    /// Parse a reference into a locator. Every reference is a local path;
    /// whatever the string holds (including an `http(s)://` URL) is taken
    /// literally.
    ///
    /// ```
    /// # use sapphire_backend::{WorkspaceLocator, DEFAULT_ID};
    /// use std::path::PathBuf;
    /// assert_eq!(
    ///     WorkspaceLocator::parse("/data/ws"),
    ///     WorkspaceLocator::Local(PathBuf::from("/data/ws"))
    /// );
    /// assert_eq!(
    ///     WorkspaceLocator::parse("relative/dir"),
    ///     WorkspaceLocator::Local(PathBuf::from("relative/dir"))
    /// );
    /// let _ = DEFAULT_ID;
    /// ```
    pub fn parse(s: &str) -> Self {
        Self::Local(PathBuf::from(s))
    }
}

/// The registry id of the default workspace.
pub const DEFAULT_ID: &str = "default";

/// One registered workspace: a local path. Serialised as a
/// `[workspace.<id>]` TOML table so the CLI config and the GUI share one
/// representation across apps (timer / journal / ledger).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    /// Display name (defaults to the registry id when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Local workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
}

impl WorkspaceEntry {
    /// An entry at `path`.
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
            ..Default::default()
        }
    }

    /// Resolve this entry to a [`WorkspaceLocator`]. Errors when no path is
    /// set.
    pub fn locator(&self) -> Result<WorkspaceLocator> {
        match &self.path {
            Some(p) => Ok(WorkspaceLocator::Local(p.clone())),
            None => Err(Error::InvalidWorkspace("entry has no `path`".into())),
        }
    }
}

/// A set of named workspaces, keyed by id. Embed in an app's config with
/// `#[serde(default)] pub workspace: WorkspaceRegistry` so it reads as
/// `[workspace.<id>]` tables.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceRegistry(pub IndexMap<String, WorkspaceEntry>);

/// A caller's workspace choice: ad-hoc CLI arguments plus an optional registry
/// id. Passed to [`WorkspaceRegistry::resolve`].
#[derive(Clone, Debug, Default)]
pub struct WorkspaceSelection<'a> {
    /// `--workspace <id>`: look this id up in the registry.
    pub id: Option<&'a str>,
    /// An ad-hoc local path (e.g. `--timer-dir`). Highest precedence.
    pub ad_hoc_path: Option<&'a Path>,
}

impl WorkspaceRegistry {
    /// Look up an entry by id.
    pub fn get(&self, id: &str) -> Option<&WorkspaceEntry> {
        self.0.get(id)
    }

    /// Insert or replace an entry.
    pub fn insert(&mut self, id: impl Into<String>, entry: WorkspaceEntry) {
        self.0.insert(id.into(), entry);
    }

    /// Remove an entry, returning it if present.
    pub fn remove(&mut self, id: &str) -> Option<WorkspaceEntry> {
        self.0.shift_remove(id)
    }

    /// Whether no entries are registered.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Registered workspace ids, in insertion order.
    pub fn ids(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }

    /// Display name for `id` (its `name`, falling back to the id itself).
    pub fn display_name(&self, id: &str) -> String {
        self.get(id)
            .and_then(|e| e.name.clone())
            .unwrap_or_else(|| id.to_owned())
    }

    /// Resolve a [`WorkspaceSelection`] to a [`WorkspaceLocator`], shared by all
    /// apps' CLIs.
    ///
    /// Precedence: ad-hoc path → registry id → the `default` entry → the
    /// built-in default local path `<data_dir>/workspaces/default`.
    pub fn resolve(
        &self,
        sel: &WorkspaceSelection<'_>,
        data_dir: &Path,
    ) -> Result<WorkspaceLocator> {
        if let Some(path) = sel.ad_hoc_path {
            return Ok(WorkspaceLocator::Local(path.to_path_buf()));
        }
        if let Some(id) = sel.id {
            let entry = self
                .get(id)
                .ok_or_else(|| Error::InvalidWorkspace(format!("unknown workspace id '{id}'")))?;
            return entry.locator();
        }
        if let Some(entry) = self.get(DEFAULT_ID) {
            return entry.locator();
        }
        Ok(WorkspaceLocator::Local(
            data_dir.join("workspaces").join(DEFAULT_ID),
        ))
    }
}

/// Opened resources for a workspace, ready to become a backend.
pub enum WorkspaceSource {
    /// A local workspace, driven directly.
    Local {
        /// The opened local workspace state.
        state: Arc<WorkspaceState>,
    },
}

impl WorkspaceSource {
    /// Build the concrete backend behind a trait object, so callers hold one
    /// `Box<dyn WorkspaceBackend>`.
    pub fn into_backend(self) -> Box<dyn WorkspaceBackend> {
        match self {
            WorkspaceSource::Local { state } => Box::new(LocalBackend::new(state)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_local_path() {
        assert_eq!(
            WorkspaceLocator::parse("/home/me/notes"),
            WorkspaceLocator::Local(PathBuf::from("/home/me/notes"))
        );
        assert_eq!(
            WorkspaceLocator::parse("relative/dir"),
            WorkspaceLocator::Local(PathBuf::from("relative/dir"))
        );
    }

    #[test]
    fn a_url_is_just_a_path_now() {
        // No remote form is left: the string is taken literally.
        assert_eq!(
            WorkspaceLocator::parse("https://example.com#work"),
            WorkspaceLocator::Local(PathBuf::from("https://example.com#work"))
        );
    }

    #[test]
    fn entry_locator_requires_a_path() {
        assert_eq!(
            WorkspaceEntry::local("/data/ws").locator().unwrap(),
            WorkspaceLocator::Local(PathBuf::from("/data/ws"))
        );
        assert!(WorkspaceEntry::default().locator().is_err());
    }

    #[test]
    fn registry_toml_roundtrip() {
        let toml = r#"
[default]
path = "/home/me/ws"

[work]
name = "Work"
path = "/home/me/work"
"#;
        let reg: WorkspaceRegistry = toml::from_str(toml).unwrap();
        assert_eq!(reg.ids().count(), 2);
        assert_eq!(reg.display_name("work"), "Work");
        assert_eq!(reg.display_name("default"), "default");
        let back = toml::to_string(&reg).unwrap();
        let reg2: WorkspaceRegistry = toml::from_str(&back).unwrap();
        assert_eq!(reg, reg2);
    }

    /// An old config that still carries `url` (and optionally `token`) loads:
    /// serde ignores the unknown fields, and the entry simply has no path.
    #[test]
    fn an_entry_from_before_the_remote_form_still_loads() {
        let entry: WorkspaceEntry = toml::from_str(r#"url = "https://example.com#work""#).unwrap();
        assert_eq!(entry.path, None);
        assert!(entry.locator().is_err());
    }

    #[test]
    fn resolve_precedence() {
        let mut reg = WorkspaceRegistry::default();
        reg.insert("work", WorkspaceEntry::local("/work/ws"));
        let data = Path::new("/data");

        // ad-hoc path wins over everything.
        let sel = WorkspaceSelection {
            id: Some("work"),
            ad_hoc_path: Some(Path::new("/tmp/ws")),
        };
        assert_eq!(
            reg.resolve(&sel, data).unwrap(),
            WorkspaceLocator::Local("/tmp/ws".into())
        );

        // registry id next.
        let sel = WorkspaceSelection {
            id: Some("work"),
            ..Default::default()
        };
        assert_eq!(
            reg.resolve(&sel, data).unwrap(),
            WorkspaceLocator::Local("/work/ws".into())
        );

        // unknown id errors.
        let sel = WorkspaceSelection {
            id: Some("nope"),
            ..Default::default()
        };
        assert!(reg.resolve(&sel, data).is_err());

        // fall back to the built-in default path.
        assert_eq!(
            reg.resolve(&WorkspaceSelection::default(), data).unwrap(),
            WorkspaceLocator::Local(PathBuf::from("/data/workspaces/default"))
        );

        // an explicit `default` entry overrides the built-in path.
        reg.insert(DEFAULT_ID, WorkspaceEntry::local("/custom/default"));
        assert_eq!(
            reg.resolve(&WorkspaceSelection::default(), data).unwrap(),
            WorkspaceLocator::Local(PathBuf::from("/custom/default"))
        );
    }
}
