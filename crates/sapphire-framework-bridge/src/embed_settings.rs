//! Where the embedding settings live, and what they resolve to (#186, #314).
//!
//! Three places, by owner, all under `[embed]` of a `config.toml` but the key:
//!
//! - the workgroup's synced `root/config.toml` holds the two model slots;
//! - this host's `config.toml` holds its switch per slot, the local model's cache
//!   directory, and — only on a host without a workgroup — the model slots too;
//! - this host's `embedding.key` holds the remote slot's API key, readable by the owner only.
//!
//! [`load`] reads all three and [`resolve`]s them. Changes are settings batches: the RPCs
//! and the bridge's offline CLI turn their request into one with [`embed_ops`] and carry it
//! out with [`apply`].

use std::path::{Path, PathBuf};

use sapphire_bridge_api::{
    ApiKey, DeviceSettings, EmbedDeviceSetParams, EmbedInfoResult, EmbedModelSetParams, EmbedNote,
    EmbedRequest, EmbedSettingsResult, LocalModel, ModelSettings, ModelSource, RemoteModel, Slot,
    SlotModel,
};
use sapphire_framework_settings::{Op, Scope, Value};

use crate::config_file::{EmbedTable, read_device, read_workgroup};
use crate::dir::BridgeDir;
use crate::error::{Error, Result};
use crate::workgroup::Workgroup;

/// The API key's file name, in the bridge directory.
pub const KEY_FILE: &str = "embedding.key";

/// The key of the API key in the settings registry.
const KEY_SETTING: &str = "embed.remote.key";

/// What the bridge serves, and with what: the input of the [`EmbedFactory`](crate::EmbedFactory).
///
/// Compared with the last one applied, so the provider is rebuilt only when this changes.
/// `Debug` never shows the key ([`ApiKey`] hides it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbedConfig {
    /// The local slot.
    Local {
        /// Its model.
        model: LocalModel,
        /// Where its files are cached; `None` is the host's default.
        cache_dir: Option<PathBuf>,
    },
    /// The remote slot.
    Remote {
        /// Its model.
        model: RemoteModel,
        /// This device's key, if one is stored.
        key: Option<ApiKey>,
    },
}

/// The settings, read and resolved: everything [`EmbedSettingsResult`] reports but `info`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolved {
    /// The model settings in effect.
    pub models: ModelSettings,
    /// Where they come from.
    pub source: Option<ModelSource>,
    /// This host's own models are ignored because the workgroup's settings win.
    pub shadowed: bool,
    /// This host's switches and cache directory, as written.
    pub device: DeviceSettings,
    /// The local slot's effective switch.
    pub local_enabled: bool,
    /// The remote slot's effective switch.
    pub remote_enabled: bool,
    /// The slot served.
    pub active: Option<Slot>,
    /// Whether the CPU has AVX2.
    pub avx2: bool,
    /// Whether a key is stored.
    pub key_set: bool,
    /// Why embedding is off, or what it lacks.
    pub note: Option<EmbedNote>,
}

impl Resolved {
    /// The report, with what `embed.info` answers now.
    pub fn report(&self, info: EmbedInfoResult) -> EmbedSettingsResult {
        EmbedSettingsResult {
            models: self.models.clone(),
            source: self.source,
            shadowed_device_models: self.shadowed,
            device: self.device.clone(),
            local_enabled: self.local_enabled,
            remote_enabled: self.remote_enabled,
            active: self.active,
            avx2: self.avx2,
            key_set: self.key_set,
            info,
        }
    }
}

/// Whether this CPU can run the local model in useful time: AVX2 on x86_64. Other
/// architectures count as capable.
pub fn avx2() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        true
    }
}

/// Resolve the three sources. Pure: the files are read by [`load`].
///
/// `shared` is the workgroup's models: `None` when the host has no workgroup, an `Err` when
/// its file cannot be read as models. `device` is this host's `[embed]`; its models count
/// only without a workgroup, and are reported as shadowed with one.
fn resolve(
    shared: Option<std::result::Result<ModelSettings, String>>,
    device: &EmbedTable,
    key_set: bool,
    avx2: bool,
) -> Resolved {
    let own = device.models();
    // A slot half-written on this device still counts as configured here: it is shadowed.
    let own_configured = !matches!(&own, Ok(models) if models.is_empty());
    let (models, source, shadowed, invalid) = match shared {
        Some(Ok(models)) => (models, Some(ModelSource::Workgroup), own_configured, None),
        Some(Err(why)) => (
            ModelSettings::default(),
            Some(ModelSource::Workgroup),
            own_configured,
            Some(why),
        ),
        None => match own {
            Ok(models) if models.is_empty() => (ModelSettings::default(), None, false, None),
            Ok(models) => (models, Some(ModelSource::Device), false, None),
            Err(why) => (
                ModelSettings::default(),
                Some(ModelSource::Device),
                false,
                Some(why),
            ),
        },
    };
    // A file that parses but breaks the rules is as unusable as one that does not parse.
    let invalid = invalid.or_else(|| models.validate().err());
    let usable = invalid.is_none();

    let settings = device_settings(device);
    let local_enabled = usable && models.local.is_some() && settings.local_enabled.unwrap_or(avx2);
    let remote_enabled =
        usable && models.remote.is_some() && settings.remote_enabled.unwrap_or(true);
    let active = if remote_enabled {
        Some(Slot::Remote)
    } else if local_enabled {
        Some(Slot::Local)
    } else {
        None
    };

    let note = if let Some(why) = invalid {
        Some(EmbedNote::Invalid(why))
    } else if models.is_empty() {
        Some(EmbedNote::NotConfigured)
    } else if active.is_none() {
        let only_auto_local = models.remote.is_none() && settings.local_enabled.is_none();
        Some(if only_auto_local && !avx2 {
            EmbedNote::NoAvx2
        } else {
            EmbedNote::DisabledOnDevice
        })
    } else if active == Some(Slot::Remote) && !key_set {
        Some(EmbedNote::KeyMissing)
    } else {
        None
    };

    Resolved {
        models,
        source,
        shadowed,
        device: settings,
        local_enabled,
        remote_enabled,
        active,
        avx2,
        key_set,
        note,
    }
}

/// This host's switches and cache directory, as written in its `[embed]`.
fn device_settings(device: &EmbedTable) -> DeviceSettings {
    DeviceSettings {
        local_enabled: device.local.as_ref().and_then(|t| t.enabled),
        remote_enabled: device.remote.as_ref().and_then(|t| t.enabled),
        cache_dir: device.cache_dir.clone(),
    }
}

/// Read and resolve the settings; the config is what the bridge should serve.
pub fn load(dir: &BridgeDir, avx2: bool) -> Result<(Resolved, Option<EmbedConfig>)> {
    let shared = match Workgroup::open(dir)? {
        Some(wg) => Some(read_shared(&wg.config_toml())?),
        None => None,
    };
    let device = read_device_embed(&dir.config_toml());
    let key = read_key(&dir.embedding_key())?;
    let resolved = resolve(shared, &device, key.is_some(), avx2);
    let config = match resolved.active {
        Some(Slot::Remote) => resolved
            .models
            .remote
            .clone()
            .map(|model| EmbedConfig::Remote { model, key }),
        Some(Slot::Local) => resolved
            .models
            .local
            .clone()
            .map(|model| EmbedConfig::Local {
                model,
                cache_dir: device.cache_dir.clone(),
            }),
        None => None,
    };
    Ok((resolved, config))
}

/// The settings batch that carries out `request`, with the scope it targets (`None`: each
/// key's home); `None` for [`EmbedRequest::Show`], which changes nothing.
///
/// Setting a model sets every field of its slot, so the new model replaces the old one
/// whole. Clearing one removes the slot's model keys from the scope that holds the models
/// and keeps this device's switch beside them. A model given for the other slot, or an
/// empty key, is an error.
pub fn embed_ops(request: EmbedRequest) -> Result<Option<(Option<Scope>, Vec<Op>)>> {
    fn set(key: &str, value: Value) -> Op {
        Op::Set {
            key: key.to_owned(),
            value,
        }
    }
    fn unset(key: &str) -> Op {
        Op::Unset {
            key: key.to_owned(),
        }
    }
    // An integer the registry does not accept is refused by its range check, not here.
    fn int(n: impl TryInto<i64>) -> Value {
        Value::Int(n.try_into().unwrap_or(i64::MAX))
    }
    let batch = match request {
        EmbedRequest::Show => return Ok(None),
        EmbedRequest::ModelSet(EmbedModelSetParams { slot, model }) => match model {
            Some(model) if model.slot() != slot => {
                return Err(Error::Config(format!(
                    "a {} model cannot go in the {slot} slot",
                    model.slot()
                )));
            }
            Some(SlotModel::Local(m)) => (
                None,
                vec![
                    set("embed.local.model", Value::String(m.model)),
                    set("embed.local.dimension", int(m.dimension)),
                    set("embed.local.max_tokens", int(m.max_tokens)),
                ],
            ),
            Some(SlotModel::Remote(m)) => (
                None,
                vec![
                    set("embed.remote.endpoint", Value::String(m.endpoint)),
                    set("embed.remote.model", Value::String(m.model)),
                    set("embed.remote.dimension", int(m.dimension)),
                ],
            ),
            // The slot's table also holds this device's switch, which lives in the device
            // scope: name the models' scope, or the prefix spans two.
            None => (
                Some(Scope::Workgroup),
                vec![unset(&format!("embed.{slot}"))],
            ),
        },
        EmbedRequest::DeviceSet(EmbedDeviceSetParams { slot, enabled }) => {
            let key = format!("embed.{slot}.enabled");
            (
                None,
                vec![match enabled {
                    Some(on) => set(&key, Value::Bool(on)),
                    None => unset(&key),
                }],
            )
        }
        EmbedRequest::KeySet(key) => {
            if key.is_empty() {
                return Err(Error::Config("the API key is empty".to_owned()));
            }
            (
                None,
                vec![set(KEY_SETTING, Value::String(key.expose().to_owned()))],
            )
        }
        EmbedRequest::KeyClear => (None, vec![unset(KEY_SETTING)]),
    };
    Ok(Some(batch))
}

/// Carry out `request` on the settings files of `dir`, as one batch.
pub fn apply(dir: &BridgeDir, request: EmbedRequest) -> Result<()> {
    if let Some((scope, ops)) = embed_ops(request)? {
        sapphire_framework_settings::apply(&crate::settings::registry(dir)?, scope, ops)?;
    }
    Ok(())
}

// ── files ───────────────────────────────────────────────────────────────────

/// The workgroup's models: none when its file is absent, `Err` when the file does not
/// parse or its slots break the rules.
fn read_shared(path: &Path) -> Result<std::result::Result<ModelSettings, String>> {
    match read_workgroup(path) {
        Ok(Some(config)) => Ok(config
            .embed
            .models()
            .map_err(|why| format!("{}: {why}", path.display()))),
        Ok(None) => Ok(Ok(ModelSettings::default())),
        Err(Error::Config(why)) => Ok(Err(why)),
        Err(e) => Err(e),
    }
}

/// This host's `[embed]` for reading: absent or unparsable means the defaults (logged), so
/// a broken file never stops the bridge from serving. A batch still refuses to rewrite it.
fn read_device_embed(path: &Path) -> EmbedTable {
    read_device(path).map(|c| c.embed).unwrap_or_else(|err| {
        tracing::warn!(
            target: crate::logging::BRIDGE_TARGET,
            "{err}; using the default embedding settings for this device"
        );
        EmbedTable::default()
    })
}

fn read_key(path: &Path) -> Result<Option<ApiKey>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(ApiKey::new(text)).filter(|k| !k.is_empty())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_file::{LocalTable, RemoteTable};
    use sapphire_bridge_api::{EmbedDeviceSetParams, LOCAL_MODEL};

    fn local() -> LocalModel {
        LocalModel::default()
    }

    fn remote() -> RemoteModel {
        RemoteModel {
            endpoint: "https://api.example.com".into(),
            model: "m".into(),
            dimension: 8,
        }
    }

    fn both() -> ModelSettings {
        ModelSettings {
            local: Some(local()),
            remote: Some(remote()),
        }
    }

    fn device(local: Option<bool>, remote: Option<bool>) -> EmbedTable {
        EmbedTable {
            local: Some(LocalTable {
                enabled: local,
                ..LocalTable::default()
            }),
            remote: Some(RemoteTable {
                enabled: remote,
                ..RemoteTable::default()
            }),
            ..EmbedTable::default()
        }
    }

    #[test]
    fn nothing_configured_is_off() {
        let r = resolve(None, &EmbedTable::default(), false, true);
        assert_eq!((r.active, r.source), (None, None));
        assert_eq!(r.note, Some(EmbedNote::NotConfigured));
    }

    #[test]
    fn the_remote_slot_wins_when_both_are_on() {
        let r = resolve(Some(Ok(both())), &EmbedTable::default(), true, true);
        assert_eq!(r.active, Some(Slot::Remote));
        assert!(r.local_enabled && r.remote_enabled);
        assert_eq!(r.note, None);
    }

    #[test]
    fn auto_turns_the_local_slot_off_without_avx2() {
        let local_only = ModelSettings {
            local: Some(local()),
            remote: None,
        };
        let r = resolve(
            Some(Ok(local_only.clone())),
            &EmbedTable::default(),
            false,
            false,
        );
        assert_eq!(r.active, None);
        assert_eq!(r.note, Some(EmbedNote::NoAvx2));

        // An explicit switch overrides auto.
        let r = resolve(
            Some(Ok(local_only)),
            &device(Some(true), None),
            false,
            false,
        );
        assert_eq!(r.active, Some(Slot::Local));
    }

    #[test]
    fn the_remote_slot_stays_on_without_avx2() {
        let r = resolve(Some(Ok(both())), &EmbedTable::default(), true, false);
        assert!(!r.local_enabled && r.remote_enabled);
        assert_eq!(r.active, Some(Slot::Remote));
    }

    #[test]
    fn switching_the_remote_slot_off_falls_back_to_local() {
        let r = resolve(Some(Ok(both())), &device(None, Some(false)), true, true);
        assert_eq!(r.active, Some(Slot::Local));
        let r = resolve(
            Some(Ok(both())),
            &device(Some(false), Some(false)),
            true,
            true,
        );
        assert_eq!(r.active, None);
        assert_eq!(r.note, Some(EmbedNote::DisabledOnDevice));
    }

    #[test]
    fn a_remote_slot_without_a_key_still_serves_but_says_so() {
        let r = resolve(Some(Ok(both())), &EmbedTable::default(), false, true);
        assert_eq!(r.active, Some(Slot::Remote));
        assert_eq!(r.note, Some(EmbedNote::KeyMissing));
    }

    #[test]
    fn the_workgroup_wins_over_the_device_models() {
        let d = EmbedTable {
            local: Some(LocalTable {
                model: Some(LOCAL_MODEL.to_owned()),
                ..LocalTable::default()
            }),
            ..EmbedTable::default()
        };
        let shared = ModelSettings {
            local: None,
            remote: Some(remote()),
        };
        let r = resolve(Some(Ok(shared)), &d, true, true);
        assert_eq!(r.source, Some(ModelSource::Workgroup));
        assert!(r.shadowed);
        assert_eq!(r.active, Some(Slot::Remote));

        // Without a workgroup, the device's own apply.
        let r = resolve(None, &d, false, true);
        assert_eq!(r.source, Some(ModelSource::Device));
        assert_eq!(r.active, Some(Slot::Local));
    }

    #[test]
    fn a_table_with_only_enabled_configures_no_slot() {
        let d = device(Some(true), Some(true));
        // No workgroup: the switches alone are no model.
        let r = resolve(None, &d, true, true);
        assert_eq!((r.source, r.active), (None, None));
        assert_eq!(r.note, Some(EmbedNote::NotConfigured));
        // With one: nothing of the device's is shadowed.
        let r = resolve(Some(Ok(both())), &d, true, true);
        assert!(!r.shadowed);
        assert_eq!(r.active, Some(Slot::Remote));
    }

    #[test]
    fn invalid_settings_turn_embedding_off() {
        let r = resolve(Some(Err("bad".into())), &EmbedTable::default(), true, true);
        assert_eq!(r.active, None);
        assert_eq!(r.note, Some(EmbedNote::Invalid("bad".into())));

        let mut broken = both();
        broken.remote.as_mut().unwrap().dimension = 0;
        let r = resolve(Some(Ok(broken)), &EmbedTable::default(), true, true);
        assert_eq!(r.active, None);
        assert!(matches!(r.note, Some(EmbedNote::Invalid(_))));
    }

    fn bridge_dir() -> (tempfile::TempDir, BridgeDir) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().to_path_buf()).unwrap();
        (tmp, dir)
    }

    /// Carry out `request` as the RPCs and the offline CLI do: as one settings batch.
    fn run(dir: &BridgeDir, request: EmbedRequest) -> Result<()> {
        let (scope, ops) = embed_ops(request)?.expect("a change");
        sapphire_framework_settings::apply(&crate::settings::registry(dir)?, scope, ops)?;
        Ok(())
    }

    fn model_set(slot: Slot, model: Option<SlotModel>) -> EmbedRequest {
        EmbedRequest::ModelSet(EmbedModelSetParams { slot, model })
    }

    fn device_set(slot: Slot, enabled: Option<bool>) -> EmbedRequest {
        EmbedRequest::DeviceSet(EmbedDeviceSetParams { slot, enabled })
    }

    #[test]
    fn show_is_no_change() {
        assert!(embed_ops(EmbedRequest::Show).unwrap().is_none());
    }

    #[test]
    fn without_a_workgroup_models_go_in_the_device_file() {
        let (_tmp, dir) = bridge_dir();
        run(
            &dir,
            model_set(Slot::Remote, Some(SlotModel::Remote(remote()))),
        )
        .unwrap();
        run(&dir, device_set(Slot::Local, Some(false))).unwrap();
        run(&dir, EmbedRequest::KeySet(ApiKey::new("sk-x"))).unwrap();

        let (r, config) = load(&dir, true).unwrap();
        assert_eq!(r.source, Some(ModelSource::Device));
        assert_eq!(r.device.local_enabled, Some(false));
        assert_eq!(
            config,
            Some(EmbedConfig::Remote {
                model: remote(),
                key: Some(ApiKey::new("sk-x")),
            })
        );

        // Clearing the only slot removes its fields and keeps this device's switches.
        run(&dir, model_set(Slot::Remote, None)).unwrap();
        let text = std::fs::read_to_string(dir.config_toml()).unwrap();
        assert!(!text.contains("[embed.remote]"), "{text}");
        assert!(!text.contains("endpoint"), "{text}");
        assert!(text.contains("[embed.local]"), "{text}");
        assert!(text.contains("enabled = false"), "{text}");
        assert_eq!(load(&dir, true).unwrap().0.source, None);
    }

    #[test]
    fn clearing_a_slot_keeps_its_device_switch() {
        let (_tmp, dir) = bridge_dir();
        run(
            &dir,
            model_set(Slot::Remote, Some(SlotModel::Remote(remote()))),
        )
        .unwrap();
        run(&dir, device_set(Slot::Remote, Some(false))).unwrap();
        run(&dir, model_set(Slot::Remote, None)).unwrap();
        let (r, _) = load(&dir, true).unwrap();
        assert_eq!(r.models.remote, None);
        assert_eq!(r.device.remote_enabled, Some(false));
    }

    #[test]
    fn setting_a_model_replaces_every_field_of_its_slot() {
        let (_tmp, dir) = bridge_dir();
        run(
            &dir,
            model_set(
                Slot::Local,
                Some(SlotModel::Local(LocalModel {
                    dimension: 512,
                    max_tokens: 128,
                    ..local()
                })),
            ),
        )
        .unwrap();
        run(
            &dir,
            model_set(Slot::Local, Some(SlotModel::Local(local()))),
        )
        .unwrap();
        let (r, _) = load(&dir, true).unwrap();
        assert_eq!(r.models.local, Some(local()));
    }

    #[test]
    fn with_a_workgroup_models_go_in_its_root() {
        let (_tmp, dir) = bridge_dir();
        let wg = Workgroup::create(&dir, "wg", "this", "node").unwrap();
        run(
            &dir,
            model_set(Slot::Local, Some(SlotModel::Local(local()))),
        )
        .unwrap();
        assert!(wg.config_toml().is_file());
        assert!(!dir.config_toml().exists());
        let (r, config) = load(&dir, true).unwrap();
        assert_eq!(r.source, Some(ModelSource::Workgroup));
        assert!(matches!(config, Some(EmbedConfig::Local { .. })));

        // Clearing leaves the workgroup with no model.
        run(&dir, model_set(Slot::Local, None)).unwrap();
        let text = std::fs::read_to_string(wg.config_toml()).unwrap();
        assert!(!text.contains("[embed.local]"), "{text}");
        assert_eq!(
            load(&dir, true).unwrap().0.note,
            Some(EmbedNote::NotConfigured)
        );
    }

    #[test]
    fn with_a_workgroup_the_device_models_are_shadowed() {
        let (_tmp, dir) = bridge_dir();
        run(
            &dir,
            model_set(Slot::Local, Some(SlotModel::Local(local()))),
        )
        .unwrap();
        Workgroup::create(&dir, "wg", "this", "node").unwrap();
        let (r, config) = load(&dir, true).unwrap();
        assert_eq!(r.source, Some(ModelSource::Workgroup));
        assert!(r.shadowed);
        assert_eq!(config, None);
    }

    #[test]
    fn a_model_must_match_its_slot_and_validate() {
        let (_tmp, dir) = bridge_dir();
        let err = run(
            &dir,
            model_set(Slot::Local, Some(SlotModel::Remote(remote()))),
        )
        .unwrap_err();
        assert!(err.to_string().contains("remote model"), "{err}");
        let err = run(
            &dir,
            model_set(
                Slot::Remote,
                Some(SlotModel::Remote(RemoteModel {
                    dimension: 0,
                    ..remote()
                })),
            ),
        )
        .unwrap_err();
        assert!(err.to_string().contains("dimension"), "{err}");
        assert!(!dir.config_toml().exists(), "nothing written");
    }

    #[test]
    fn the_key_file_is_private_and_clearable() {
        let (_tmp, dir) = bridge_dir();
        run(&dir, EmbedRequest::KeySet(ApiKey::new("sk-secret\n"))).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.embedding_key()).unwrap(),
            "sk-secret"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.embedding_key())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        run(&dir, EmbedRequest::KeyClear).unwrap();
        run(&dir, EmbedRequest::KeyClear).unwrap();
        assert!(!dir.embedding_key().exists());
        assert!(run(&dir, EmbedRequest::KeySet(ApiKey::new(" "))).is_err());
    }

    #[test]
    fn the_config_never_prints_the_key() {
        let config = EmbedConfig::Remote {
            model: remote(),
            key: Some(ApiKey::new("sk-secret")),
        };
        assert!(!format!("{config:?}").contains("sk-secret"));
    }

    #[test]
    fn an_unparsable_device_file_reads_as_defaults_but_is_not_overwritten() {
        let (_tmp, dir) = bridge_dir();
        let bad = "enabled = true\nprovider = \"local\"\n";
        std::fs::write(dir.config_toml(), bad).unwrap();
        let (r, _) = load(&dir, true).unwrap();
        assert_eq!(r.device, DeviceSettings::default());
        assert!(run(&dir, device_set(Slot::Local, Some(true))).is_err());
        assert_eq!(std::fs::read_to_string(dir.config_toml()).unwrap(), bad);
    }

    #[test]
    fn a_device_net_section_survives_an_embedding_change() {
        let (_tmp, dir) = bridge_dir();
        std::fs::write(dir.config_toml(), "[net]\n# keep\ndiscovery = false\n").unwrap();
        run(&dir, device_set(Slot::Local, Some(false))).unwrap();
        let text = std::fs::read_to_string(dir.config_toml()).unwrap();
        assert!(
            text.starts_with("[net]\n# keep\ndiscovery = false\n"),
            "{text}"
        );
        assert!(text.contains("[embed.local]"), "{text}");
        assert!(
            !crate::net::NetConfig::load(&dir.config_toml())
                .unwrap()
                .discovery
        );
    }
}
