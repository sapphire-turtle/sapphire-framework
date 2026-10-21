//! The `config.toml` files: one struct per scope, read by key (#314).
//!
//! The device's `<bridge>/config.toml` and a workgroup's `root/config.toml` have the same
//! shape: `[net]` and `[embed]`. The structs accept the union of both scopes' keys and
//! reject anything else; which keys each scope may hold is the registry's decision, not
//! the struct's.

use std::path::{Path, PathBuf};

use sapphire_bridge_api::{LocalModel, ModelSettings, RemoteModel};
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::net::NetConfig;

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

/// Read the device's `config.toml`; a missing file is the defaults.
pub fn read_device(path: &Path) -> Result<DeviceConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(path, &text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DeviceConfig::default()),
        Err(e) => Err(Error::Io(e)),
    }
}

/// Read a workgroup's `config.toml`; a missing file is `None`.
pub fn read_workgroup(path: &Path) -> Result<Option<WorkgroupConfig>> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(path, &text).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(e)),
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
}
