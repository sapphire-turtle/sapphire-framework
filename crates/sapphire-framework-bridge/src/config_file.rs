//! The `config.toml` files: one struct per scope, read by key (#314).
//!
//! The device's `<bridge>/config.toml` and a workgroup's `root/config.toml` have the same
//! shape: `[net]` and `[embed]`. The structs accept the union of both scopes' keys and
//! reject anything else; which keys each scope may hold is the registry's decision, not
//! the struct's.

use std::path::{Path, PathBuf};

use sapphire_bridge_api::{LocalModel, ModelSettings, RemoteModel};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use toml_edit::{DocumentMut, Item, Table};

use crate::dir::BridgeDir;
use crate::error::{Error, Result};
use crate::net::NetConfig;
use crate::workgroup::Workgroup;

/// This host's `config.toml`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceConfig {
    /// `[net]`.
    #[serde(default)]
    pub net: NetConfig,
    /// `[embed]`.
    #[serde(default)]
    pub embed: EmbedTable,
}

/// A workgroup's `root/config.toml`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkgroupConfig {
    /// `[net]`, what the workgroup announces.
    #[serde(default)]
    pub net: NetConfig,
    /// `[embed]`, the models.
    #[serde(default)]
    pub embed: EmbedTable,
}

/// `[embed]`: the device's switches and cache, and the models of whichever scope holds them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbedTable {
    /// Where the local model's files are cached.
    pub cache_dir: Option<PathBuf>,
    /// `[embed.local]`.
    pub local: Option<LocalTable>,
    /// `[embed.remote]`.
    pub remote: Option<RemoteTable>,
}

/// `[embed.local]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalTable {
    /// The slot's switch; unset = auto.
    pub enabled: Option<bool>,
    /// The model's name.
    pub model: Option<String>,
    /// Output dimension.
    pub dimension: Option<u32>,
    /// Content tokens kept.
    pub max_tokens: Option<usize>,
}

/// `[embed.remote]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteTable {
    /// The slot's switch; unset = auto.
    pub enabled: Option<bool>,
    /// The endpoint's base URL.
    pub endpoint: Option<String>,
    /// The model's name at the endpoint.
    pub model: Option<String>,
    /// Output dimension.
    pub dimension: Option<u32>,
}

impl EmbedTable {
    /// The model slots this table configures, checked.
    ///
    /// The local slot is configured when `local.model` is set, with the defaults of
    /// [`LocalModel`] for the rest; the remote slot when `remote.endpoint` is set. A table
    /// holding only `enabled` configures nothing. A remote slot with some of its fields
    /// missing is an error naming the missing field.
    pub fn models(&self) -> std::result::Result<ModelSettings, String> {
        let local = self.local.as_ref().and_then(|t| {
            let model = t.model.clone()?;
            let defaults = LocalModel::default();
            Some(LocalModel {
                model,
                dimension: t.dimension.unwrap_or(defaults.dimension),
                max_tokens: t.max_tokens.unwrap_or(defaults.max_tokens),
            })
        });
        let remote = match &self.remote {
            Some(t) => match &t.endpoint {
                Some(endpoint) => Some(RemoteModel {
                    endpoint: endpoint.clone(),
                    model: t.model.clone().ok_or_else(|| {
                        "remote: `model` is required when `endpoint` is set".to_owned()
                    })?,
                    dimension: t.dimension.ok_or_else(|| {
                        "remote: `dimension` is required when `endpoint` is set".to_owned()
                    })?,
                }),
                None if t.model.is_some() || t.dimension.is_some() => {
                    return Err(
                        "remote: `endpoint` is required when `model` or `dimension` \
                         is set"
                            .to_owned(),
                    );
                }
                None => None,
            },
            None => None,
        };
        let models = ModelSettings { local, remote };
        models.validate()?;
        Ok(models)
    }
}

/// Parse `text` as the file at `path`.
pub(crate) fn parse<T: for<'de> Deserialize<'de>>(path: &Path, text: &str) -> Result<T> {
    toml::from_str(text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
}

/// Read one table of the `config.toml` at `path`, `[net]` or `[embed]`, and nothing else.
///
/// A missing file is `None`, a file without the table its defaults. The rest of the file
/// is not looked at: the network must not stop on an `[embed]` key this build does not
/// know — a newer device's, synced into a workgroup root — nor embedding on a broken
/// `[net]`. The file must still be valid TOML. Writes check the whole file
/// ([`DeviceConfig`] / [`WorkgroupConfig`]).
pub(crate) fn read_table<T: DeserializeOwned + Default>(
    path: &Path,
    name: &str,
) -> Result<Option<T>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::Io(e)),
    };
    let mut file: toml::Table = parse(path, &text)?;
    match file.remove(name) {
        Some(table) => table
            .try_into()
            .map(Some)
            .map_err(|e| Error::Config(format!("{}: [{name}]: {e}", path.display()))),
        None => Ok(Some(T::default())),
    }
}

/// Read the device's whole `config.toml`; a missing file is the defaults.
pub fn read_device(path: &Path) -> Result<DeviceConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(path, &text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DeviceConfig::default()),
        Err(e) => Err(Error::Io(e)),
    }
}

/// Read a workgroup's whole `config.toml`; a missing file is `None`.
pub fn read_workgroup(path: &Path) -> Result<Option<WorkgroupConfig>> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(path, &text).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(e)),
    }
}

// --- Migration from format 1 (#314) ---
//
// Format 1 kept the settings in separate files: `net.toml` and `embedding.toml` in the
// bridge directory, and `root/net.toml` and `root/embedding.toml` in a workgroup's root.
// Both migrations move the old files' entries into `[net]` / `[embed]` of `config.toml`
// with `toml_edit`, so comments travel with them, and never replace a table `config.toml`
// already has: that makes running one twice, or after a stop half-way, harmless.

/// `[net]` of a `config.toml`.
pub(crate) const NET: &str = "net";
/// `[embed]` of a `config.toml`.
pub(crate) const EMBED: &str = "embed";

/// Fold a format-1 bridge directory's `net.toml` and `embedding.toml` into `config.toml`.
///
/// `net.toml` becomes `[net]`; `embedding.toml`'s `cache_dir` and `[local]` / `[remote]`
/// switches become `[embed]`, and its `[models.local]` / `[models.remote]` join the same
/// slot tables — except on a host whose workgroup, still in format 1, has no models of
/// its own: it receives them instead ([`promote_models`]). A table `config.toml` already
/// holds is kept as it is.
/// The old files are renamed to `*.toml.v1` once `config.toml` is written, so a user can
/// see what was moved. Idempotent: with the old files gone there is nothing to do, and a
/// run stopped before the rename finds its tables already there.
pub(crate) fn migrate_device(dir: &BridgeDir) -> Result<()> {
    let net_path = dir.legacy_net_toml();
    let embedding_path = dir.legacy_embedding_toml();
    let old_net = read_doc(&net_path)?;
    let old_embedding = read_doc(&embedding_path)?;
    if old_net.is_none() && old_embedding.is_none() {
        return Ok(());
    }
    let path = dir.config_toml();
    let mut doc = read_doc(&path)?.unwrap_or_default();

    if let Some(old) = &old_net
        && !doc.contains_key(NET)
    {
        doc.insert(NET, Item::Table(moved(old.as_table())));
    }
    if let Some(old) = &old_embedding {
        let at = |key: &str| -> Result<Option<Table>> {
            old.get(key)
                .map(|item| table_of(item, &embedding_path, key))
                .transpose()
        };
        let mut local = at("local")?;
        let mut remote = at("remote")?;
        let (mut model_local, model_remote) = match old.get("models") {
            Some(item) => {
                let models = table_of(item, &embedding_path, "models")?;
                let slot = |key: &str| {
                    models
                        .get(key)
                        .map(|item| table_of(item, &embedding_path, key))
                        .transpose()
                };
                (slot("local")?, slot("remote")?)
            }
            None => (None, None),
        };
        // A format-1 `[models.local]` meant "configured", with the model's default name; in
        // format 2 a local slot is configured only when it names its model.
        if let Some(t) = &mut model_local {
            with_local_model(t);
        }
        let has_models = model_local.is_some() || model_remote.is_some();
        if !has_models || !promote_models(dir, model_local.as_ref(), model_remote.as_ref())? {
            local = joined(local, model_local);
            remote = joined(remote, model_remote);
        }

        // `config.toml`'s own `[embed]` is edited in place, so its layout stays.
        let created = !doc.contains_key(EMBED);
        if created {
            let mut t = Table::new();
            t.set_implicit(true);
            doc.insert(EMBED, Item::Table(t));
        }
        if let Some(Item::Value(toml_edit::Value::InlineTable(t))) = doc.get(EMBED) {
            let t = t.clone().into_table();
            doc.insert(EMBED, Item::Table(t));
        }
        let embed = doc[EMBED].as_table_mut().ok_or_else(|| {
            Error::Config(format!("{}: `{EMBED}` is not a table", path.display()))
        })?;
        if !embed.contains_key("cache_dir")
            && let Some((key, item)) = old.as_table().get_key_value("cache_dir")
        {
            embed.insert_formatted(key, item.clone());
        }
        for (slot, table) in [("local", local), ("remote", remote)] {
            if let Some(table) = table
                && !embed.contains_key(slot)
            {
                embed.insert(slot, Item::Table(table));
            }
        }
        if created && embed.is_empty() {
            doc.remove(EMBED);
        }
    }

    let text = doc.to_string();
    parse::<DeviceConfig>(&path, &text).map_err(|e| {
        Error::Config(format!(
            "could not migrate {} and {} into config.toml: {e}",
            net_path.display(),
            embedding_path.display()
        ))
    })?;
    sapphire_framework_settings::write_atomic(&path, &text)?;
    for old in [&net_path, &embedding_path] {
        match std::fs::rename(old, old.with_extension("toml.v1")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Ok(())
}

/// Give a joined host's own format-1 models to its workgroup when the workgroup has none.
///
/// In format 1 a workgroup without `root/embedding.toml` let each host use its own
/// `[models]`; in format 2 a joined host's workgroup wins even when it holds no models. So
/// a host that embedded with its own models before the upgrade would stop. Writing them
/// into the workgroup's `[embed]` keeps it embedding — and every other device of the
/// workgroup with it, which is what a workgroup's models mean.
///
/// Only a workgroup still in format 1 receives them — one whose root has no `config.toml`
/// yet. A root that has one was migrated already, by another device, and a missing
/// `[embed]` there is a workgroup whose models were cleared: promoting would bring back
/// what was removed. The root's own format-1 files are migrated first, so a workgroup with
/// a `root/embedding.toml` keeps its models (or its empty file's "none"). Returns whether
/// the models moved. A workgroup that cannot be found or migrated is left alone — the
/// models stay with the device and the error surfaces where the workgroup is used.
fn promote_models(dir: &BridgeDir, local: Option<&Table>, remote: Option<&Table>) -> Result<bool> {
    let wg = match Workgroup::find(dir) {
        Ok(Some(wg)) => wg,
        Ok(None) => return Ok(false),
        Err(e) => {
            tracing::warn!(
                target: crate::logging::BRIDGE_TARGET,
                "could not open the workgroup to give it this host's embedding models: {e}"
            );
            return Ok(false);
        }
    };
    let path = wg.config_toml();
    if path.exists() {
        return Ok(false);
    }
    if let Err(e) = migrate_workgroup(&wg) {
        tracing::warn!(
            target: crate::logging::BRIDGE_TARGET,
            workgroup = %wg.name,
            "could not migrate the workgroup's format-1 settings; this host keeps its own \
             embedding models: {e}"
        );
        return Ok(false);
    }
    let mut doc = read_doc(&path)?.unwrap_or_default();
    if doc.contains_key(EMBED) {
        return Ok(false);
    }
    let mut embed = Table::new();
    embed.set_implicit(true);
    for (slot, table) in [("local", local), ("remote", remote)] {
        if let Some(table) = table {
            embed.insert(slot, Item::Table(table.clone()));
        }
    }
    doc.insert(EMBED, Item::Table(embed));
    let text = doc.to_string();
    parse::<WorkgroupConfig>(&path, &text)?;
    sapphire_framework_settings::write_atomic(&path, &text)?;
    tracing::info!(
        target: crate::logging::BRIDGE_TARGET,
        workgroup = %wg.name,
        "the workgroup had no embedding models; this host's own models are now the workgroup's"
    );
    Ok(true)
}

/// Fold a workgroup root's format-1 `net.toml` and `embedding.toml` into its `config.toml`.
///
/// `net.toml` becomes `[net]` and `embedding.toml`'s `[local]` / `[remote]` become
/// `[embed.local]` / `[embed.remote]`; then the old files are deleted, which syncs. The
/// root is synced, so an old file can arrive late from a device not yet upgraded: a table
/// `config.toml` already has is never replaced, and the old file is deleted anyway, so
/// what the workgroup set in format 2 stays. Such a file is deleted unread, so one that is
/// not even TOML cannot get in the way. Two bridges migrating at once write the same
/// content, and last-writer-wins converges.
pub(crate) fn migrate_workgroup(wg: &Workgroup) -> Result<()> {
    let net_path = wg.legacy_net_toml();
    let embedding_path = wg.legacy_embedding_toml();
    if !net_path.exists() && !embedding_path.exists() {
        return Ok(());
    }
    let path = wg.config_toml();
    let mut doc = read_doc(&path)?.unwrap_or_default();
    // An old file whose table `config.toml` already has is only deleted: read it only when
    // its entries have somewhere to go.
    let old_net = if doc.contains_key(NET) {
        None
    } else {
        read_doc(&net_path)?
    };
    let old_embedding = if doc.contains_key(EMBED) {
        None
    } else {
        read_doc(&embedding_path)?
    };
    let mut changed = false;

    if let Some(old) = &old_net {
        doc.insert(NET, Item::Table(moved(old.as_table())));
        changed = true;
    }
    if let Some(old) = &old_embedding {
        let mut embed = Table::new();
        embed.set_implicit(true);
        for slot in ["local", "remote"] {
            if let Some(item) = old.get(slot) {
                let mut table = table_of(item, &embedding_path, slot)?;
                if slot == "local" {
                    with_local_model(&mut table);
                }
                embed.insert(slot, Item::Table(table));
            }
        }
        // An empty file still said "the workgroup decides, and has no models": keep that
        // as an empty `[embed]`, which an implicit table would not write.
        if embed.is_empty() {
            embed.set_implicit(false);
        }
        doc.insert(EMBED, Item::Table(embed));
        changed = true;
    }

    if changed {
        let text = doc.to_string();
        parse::<WorkgroupConfig>(&path, &text).map_err(|e| {
            Error::Config(format!(
                "could not migrate {} and {} into config.toml: {e}",
                net_path.display(),
                embedding_path.display()
            ))
        })?;
        sapphire_framework_settings::write_atomic(&path, &text)?;
    }
    for old in [&net_path, &embedding_path] {
        match std::fs::remove_file(old) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Ok(())
}

/// Read `path` as an editable document; a missing file is `None`.
fn read_doc(path: &Path) -> Result<Option<DocumentMut>> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse::<DocumentMut>()
            .map(Some)
            .map_err(|e| Error::Config(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(e)),
    }
}

/// `item`, the value of `key` in the file at `path`, as a table under a header of its own
/// in another document — however it was written.
fn table_of(item: &Item, path: &Path, key: &str) -> Result<Table> {
    match item {
        Item::Table(t) => Ok(moved(t)),
        Item::Value(toml_edit::Value::InlineTable(t)) => Ok(moved(&t.clone().into_table())),
        _ => Err(Error::Config(format!(
            "{}: `{key}` is not a table",
            path.display()
        ))),
    }
}

/// A copy of `from`, comments and all, ready to sit under a header of its own in another
/// document: its place in the old document's order means nothing there.
fn moved(from: &Table) -> Table {
    fn forget_position(table: &mut Table) {
        table.set_position(None);
        for (_, item) in table.iter_mut() {
            if let Item::Table(sub) = item {
                forget_position(sub);
            }
        }
    }
    let mut table = from.clone();
    table.set_implicit(false);
    table.set_dotted(false);
    forget_position(&mut table);
    table
}

/// The two halves of a format-1 slot — its switch and its model — as one table.
fn joined(switch: Option<Table>, model: Option<Table>) -> Option<Table> {
    match (switch, model) {
        (Some(mut switch), Some(model)) => {
            for (key, _) in model.iter() {
                if let Some((key, item)) = model.get_key_value(key) {
                    switch.insert_formatted(key, item.clone());
                }
            }
            Some(switch)
        }
        (switch, model) => switch.or(model),
    }
}

/// Name the local model in a format-1 local model table that left it to the default.
fn with_local_model(table: &mut Table) {
    if !table.contains_key("model") {
        table.insert("model", toml_edit::value(sapphire_bridge_api::LOCAL_MODEL));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> EmbedTable {
        toml::from_str::<DeviceConfig>(text).unwrap().embed
    }

    #[test]
    fn a_table_with_only_switches_configures_no_slot() {
        let t = table("[embed.local]\nenabled = false\n[embed.remote]\nenabled = true\n");
        assert!(t.models().unwrap().is_empty());
    }

    #[test]
    fn a_local_model_alone_takes_the_defaults() {
        let t = table(&format!(
            "[embed.local]\nmodel = \"{}\"\n",
            sapphire_bridge_api::LOCAL_MODEL
        ));
        assert_eq!(t.models().unwrap().local, Some(LocalModel::default()));
    }

    #[test]
    fn a_half_remote_slot_names_the_missing_field() {
        let t = table("[embed.remote]\nendpoint = \"https://x.example\"\n");
        assert!(t.models().unwrap_err().contains("`model`"));
        let t = table("[embed.remote]\nmodel = \"m\"\ndimension = 8\n");
        assert!(t.models().unwrap_err().contains("`endpoint`"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        assert!(toml::from_str::<DeviceConfig>("[embed]\nbogus = 1\n").is_err());
        assert!(toml::from_str::<DeviceConfig>("[sync]\nx = 1\n").is_err());
    }

    #[test]
    fn a_missing_workgroup_file_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(
            read_workgroup(&tmp.path().join("config.toml"))
                .unwrap()
                .is_none()
        );
        assert!(
            read_device(&tmp.path().join("config.toml"))
                .unwrap()
                .embed
                .local
                .is_none()
        );
    }

    // --- Migration from format 1 (#314) ---

    const NODE_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

    /// A bridge directory stamped format 1, holding `files` (name, text).
    fn v1_dir(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bridge");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("format"), "1").unwrap();
        for (name, text) in files {
            std::fs::write(root.join(name), text).unwrap();
        }
        (tmp, root)
    }

    /// A format-1 bridge directory that already belongs to a workgroup.
    fn v1_dir_with_workgroup(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf, Workgroup) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bridge");
        let dir = BridgeDir::at(root.clone()).unwrap();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        std::fs::write(root.join("format"), "1").unwrap();
        for (name, text) in files {
            std::fs::write(root.join(name), text).unwrap();
        }
        (tmp, root, wg)
    }

    const V1_NET: &str = "# my relays\nrelays = [\"https://r.example\"]\nwake_on_sync = false\n";
    const V1_EMBEDDING: &str = "cache_dir = \"/c\"\n[local]\nenabled = false\n\
                                [models.remote]\nendpoint = \"https://e\"\nmodel = \"m\"\n\
                                dimension = 8\n";

    #[test]
    fn a_v1_directory_is_folded_into_config_toml() {
        let (_tmp, root) = v1_dir(&[("net.toml", V1_NET), ("embedding.toml", V1_EMBEDDING)]);
        let old_net: NetConfig = toml::from_str(V1_NET).unwrap();

        let dir = BridgeDir::at(root.clone()).unwrap();

        let config = read_device(&dir.config_toml()).unwrap();
        assert_eq!(
            toml::to_string(&config.net).unwrap(),
            toml::to_string(&old_net).unwrap()
        );
        assert_eq!(config.embed.cache_dir, Some(PathBuf::from("/c")));
        assert_eq!(config.embed.local.unwrap().enabled, Some(false));
        assert_eq!(
            config.embed.remote.unwrap().endpoint.as_deref(),
            Some("https://e")
        );
        let text = std::fs::read_to_string(dir.config_toml()).unwrap();
        assert!(text.contains("# my relays"), "{text}");
        assert!(root.join("net.toml.v1").is_file());
        assert!(root.join("embedding.toml.v1").is_file());
        assert!(!root.join("net.toml").exists());
        assert!(!root.join("embedding.toml").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("format")).unwrap().trim(),
            "2"
        );
    }

    #[test]
    fn migration_is_idempotent() {
        let (_tmp, root) = v1_dir(&[("net.toml", V1_NET), ("embedding.toml", V1_EMBEDDING)]);
        let dir = BridgeDir::at(root.clone()).unwrap();
        let first = std::fs::read_to_string(dir.config_toml()).unwrap();

        // Twice more: once with the old files gone, once with them back (as if the first
        // run had stopped before renaming them).
        migrate_device(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.config_toml()).unwrap(), first);
        std::fs::write(root.join("net.toml"), V1_NET).unwrap();
        std::fs::write(root.join("embedding.toml"), V1_EMBEDDING).unwrap();
        migrate_device(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.config_toml()).unwrap(), first);
    }

    #[test]
    fn an_existing_config_toml_is_merged_not_replaced() {
        let (_tmp, root) = v1_dir(&[
            ("config.toml", "# mine\n[embed.local]\nenabled = true\n"),
            ("embedding.toml", V1_EMBEDDING),
        ]);
        let dir = BridgeDir::at(root).unwrap();

        let text = std::fs::read_to_string(dir.config_toml()).unwrap();
        assert!(text.contains("# mine"), "{text}");
        let config = read_device(&dir.config_toml()).unwrap();
        assert_eq!(config.embed.local.unwrap().enabled, Some(true));
        assert_eq!(config.embed.cache_dir, Some(PathBuf::from("/c")));
        assert_eq!(
            config.embed.remote.unwrap().endpoint.as_deref(),
            Some("https://e")
        );
    }

    #[test]
    fn a_workgroup_root_is_migrated_and_the_old_files_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        std::fs::write(wg.legacy_embedding_toml(), "[local]\ndimension = 512\n").unwrap();
        std::fs::write(wg.legacy_net_toml(), "relays = [\"https://r\"]\n").unwrap();

        let wg = Workgroup::open(&dir).unwrap().unwrap();

        let config = read_workgroup(&wg.config_toml()).unwrap().unwrap();
        let local = config.embed.local.unwrap();
        assert_eq!(local.dimension, Some(512));
        assert_eq!(
            local.model.as_deref(),
            Some(sapphire_bridge_api::LOCAL_MODEL)
        );
        assert_eq!(config.net.relays, vec!["https://r".to_owned()]);
        assert!(!wg.legacy_embedding_toml().exists());
        assert!(!wg.legacy_net_toml().exists());
    }

    #[test]
    fn a_late_synced_old_file_does_not_override_config_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        let current = "[embed.remote]\nendpoint = \"https://new\"\nmodel = \"m\"\ndimension = 8\n";
        std::fs::write(wg.config_toml(), current).unwrap();
        std::fs::write(
            wg.legacy_embedding_toml(),
            "[local]\ndimension = 512\n[remote]\nendpoint = \"https://old\"\nmodel = \"o\"\n\
             dimension = 4\n",
        )
        .unwrap();

        let wg = Workgroup::open(&dir).unwrap().unwrap();

        assert_eq!(std::fs::read_to_string(wg.config_toml()).unwrap(), current);
        assert!(!wg.legacy_embedding_toml().exists());
    }

    #[test]
    fn a_malformed_old_file_is_deleted_unread_when_config_toml_has_its_table() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        std::fs::write(wg.config_toml(), "[embed]\n").unwrap();
        std::fs::write(wg.legacy_embedding_toml(), "this is [not toml").unwrap();

        let wg = Workgroup::open(&dir).unwrap().unwrap();

        assert!(!wg.legacy_embedding_toml().exists());
        assert_eq!(
            std::fs::read_to_string(wg.config_toml()).unwrap(),
            "[embed]\n"
        );
    }

    #[test]
    fn a_malformed_old_file_without_a_home_does_not_stop_the_open() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        std::fs::write(wg.legacy_embedding_toml(), "this is [not toml").unwrap();

        let wg = Workgroup::open(&dir).unwrap().unwrap();

        assert!(
            wg.legacy_embedding_toml().is_file(),
            "a file that could not be moved is left for the user"
        );
    }

    #[test]
    fn a_newer_format_still_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("bridge");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("format"), "3").unwrap();
        std::fs::write(root.join("net.toml"), V1_NET).unwrap();

        let err = BridgeDir::at(root.clone()).unwrap_err();
        assert!(err.to_string().contains("newer"), "{err}");
        assert!(
            root.join("net.toml").is_file(),
            "a refused directory is left alone"
        );
    }

    const V1_DEVICE_MODELS: &str = "[local]\nenabled = true\n[models.local]\ndimension = 512\n\
                                    [models.remote]\nendpoint = \"https://e\"\nmodel = \"m\"\n\
                                    dimension = 8\n";

    #[test]
    fn a_joined_hosts_own_models_are_promoted_when_the_workgroup_has_none() {
        let (_tmp, root, wg) = v1_dir_with_workgroup(&[("embedding.toml", V1_DEVICE_MODELS)]);

        let dir = BridgeDir::at(root).unwrap();

        let shared = read_workgroup(&wg.config_toml()).unwrap().unwrap();
        let models = shared.embed.models().unwrap();
        let local = models.local.unwrap();
        assert_eq!(local.dimension, 512);
        assert_eq!(local.model, sapphire_bridge_api::LOCAL_MODEL);
        assert_eq!(models.remote.unwrap().endpoint, "https://e");
        let text = std::fs::read_to_string(wg.config_toml()).unwrap();
        assert!(
            text.contains(&format!("model = \"{}\"", sapphire_bridge_api::LOCAL_MODEL)),
            "the local model is written explicitly: {text}"
        );
        // The device keeps its switch; the models now live in the workgroup only.
        let device = read_device(&dir.config_toml()).unwrap();
        assert_eq!(device.embed.local.as_ref().unwrap().enabled, Some(true));
        assert!(device.embed.models().unwrap().is_empty());
    }

    #[test]
    fn a_joined_hosts_own_models_are_promoted_beside_a_format_1_net_file() {
        // The workgroup is being migrated from format 1 now: its `root/net.toml` becomes
        // `config.toml`, which still holds no models.
        let (_tmp, root, wg) = v1_dir_with_workgroup(&[("embedding.toml", V1_DEVICE_MODELS)]);
        std::fs::write(wg.legacy_net_toml(), "relays = [\"https://r\"]\n").unwrap();

        let dir = BridgeDir::at(root).unwrap();

        let shared = read_workgroup(&wg.config_toml()).unwrap().unwrap();
        assert_eq!(shared.net.relays, vec!["https://r".to_owned()]);
        assert_eq!(shared.embed.models().unwrap().local.unwrap().dimension, 512);
        let device = read_device(&dir.config_toml()).unwrap();
        assert!(device.embed.models().unwrap().is_empty());
    }

    #[test]
    fn a_workgroup_already_in_format_2_gets_no_models_back() {
        // `root/config.toml` without `[embed]`: the workgroup is in format 2 already and
        // its models were cleared. A device upgrading late must not bring its old ones
        // back for every device.
        let (_tmp, root, wg) = v1_dir_with_workgroup(&[("embedding.toml", V1_DEVICE_MODELS)]);
        let shared = "[net]\nrelays = [\"https://r\"]\n";
        std::fs::write(wg.config_toml(), shared).unwrap();

        let dir = BridgeDir::at(root).unwrap();

        assert_eq!(std::fs::read_to_string(wg.config_toml()).unwrap(), shared);
        let own = read_device(&dir.config_toml())
            .unwrap()
            .embed
            .models()
            .unwrap();
        assert_eq!(own.local.unwrap().dimension, 512);
        assert_eq!(own.remote.unwrap().endpoint, "https://e");
    }

    #[test]
    fn a_joined_hosts_own_models_are_not_promoted_when_the_workgroup_has_models() {
        let (_tmp, root, wg) = v1_dir_with_workgroup(&[("embedding.toml", V1_DEVICE_MODELS)]);
        std::fs::write(
            wg.legacy_embedding_toml(),
            "[remote]\nendpoint = \"https://shared\"\nmodel = \"s\"\ndimension = 4\n",
        )
        .unwrap();

        let dir = BridgeDir::at(root).unwrap();

        let shared = read_workgroup(&wg.config_toml()).unwrap().unwrap();
        let models = shared.embed.models().unwrap();
        assert!(models.local.is_none());
        assert_eq!(models.remote.unwrap().endpoint, "https://shared");
        let device = read_device(&dir.config_toml()).unwrap();
        let own = device.embed.models().unwrap();
        assert_eq!(own.local.unwrap().dimension, 512);
        assert_eq!(own.remote.unwrap().endpoint, "https://e");
    }

    #[test]
    fn an_empty_workgroup_embedding_file_still_means_the_workgroup_decides() {
        let (_tmp, root, wg) = v1_dir_with_workgroup(&[("embedding.toml", V1_DEVICE_MODELS)]);
        std::fs::write(wg.legacy_embedding_toml(), "").unwrap();

        let dir = BridgeDir::at(root).unwrap();

        let shared = read_workgroup(&wg.config_toml()).unwrap().unwrap();
        assert_eq!(shared.embed, EmbedTable::default());
        assert!(
            std::fs::read_to_string(wg.config_toml())
                .unwrap()
                .contains("[embed]")
        );
        let own = read_device(&dir.config_toml())
            .unwrap()
            .embed
            .models()
            .unwrap();
        assert_eq!(own.local.unwrap().dimension, 512);
    }
}
