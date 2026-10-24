//! The bridge as a settings owner: its keys, its files and its final check (#314).
//!
//! The keys are `[net]`, `[embed]` and `[gui]` of two `config.toml` files — the device's, in the
//! bridge directory, and the workgroup's, in its synced root. A host without a workgroup
//! keeps the workgroup's keys in the device file, under the same paths, so joining one later
//! moves nothing the user typed.

use std::path::PathBuf;
use std::sync::Arc;

use sapphire_bridge_api::{DEFAULT_DIMENSION, DEFAULT_MAX_TOKENS, LOCAL_MODEL, MAX_TOKENS, QType};
use sapphire_framework_settings::{
    Apply, FieldError, Kind, Process, Registry, Scope, SettingDef, SettingsOwner, Staged, Value,
    gui,
};

use crate::config_file::{DeviceConfig, WorkgroupConfig};
use crate::dir::BridgeDir;
use crate::net::NetConfig;
use crate::relay::relays;
use crate::workgroup::Workgroup;

/// The remark on workgroup keys stored in the device file.
const NO_WORKGROUP: &str = "this host has no workgroup; stored on this device";

/// The bridge's settings, in the order `list` shows them.
pub fn bridge_defs() -> Vec<SettingDef> {
    let net = NetConfig::default();
    let def = |key: &str,
               kind: Kind,
               scope: Scope,
               apply: Apply,
               default: Option<Value>,
               section: &str,
               doc: &str| SettingDef {
        key: key.into(),
        kind,
        scopes: vec![scope],
        overridable: false,
        apply,
        default,
        section: section.into(),
        doc: doc.into(),
        process: Process::Bridge,
    };
    let int = |min: i64, max: i64| Kind::Int { min, max };
    vec![
        def(
            "net.wake_on_sync",
            Kind::Bool,
            Scope::Device,
            Apply::Restart,
            Some(Value::Bool(net.wake_on_sync)),
            "Network",
            "Start a stopped app server when a peer asks for one of its workspaces",
        ),
        def(
            "net.discovery",
            Kind::Bool,
            Scope::Device,
            Apply::Restart,
            Some(Value::Bool(net.discovery)),
            "Network",
            "Use discovery services to find peers",
        ),
        def(
            "net.relays",
            Kind::List {
                item: Box::new(Kind::Url),
            },
            Scope::Device,
            Apply::Restart,
            Some(Value::List(
                net.relays.iter().cloned().map(Value::String).collect(),
            )),
            "Network",
            "Relay URLs to use, in addition to the public ones",
        ),
        def(
            "net.use_default_relays",
            Kind::Bool,
            Scope::Device,
            Apply::Restart,
            Some(Value::Bool(net.use_default_relays)),
            "Network",
            "Keep the public relays on alongside `net.relays`",
        ),
        def(
            "embed.cache_dir",
            Kind::Path,
            Scope::Device,
            Apply::Live,
            None,
            "Embedding",
            "Where the local model's files are cached",
        ),
        def(
            "embed.local.enabled",
            Kind::Bool,
            Scope::Device,
            Apply::Live,
            None,
            "Embedding",
            "Switch for the local model; unset = on when it is configured",
        ),
        def(
            "embed.local.qtype",
            Kind::Enum {
                choices: QType::ALL.iter().map(|q| q.as_str().to_owned()).collect(),
            },
            Scope::Device,
            Apply::Live,
            None,
            "Embedding",
            "How this device holds the local model's weights; unset = f16",
        ),
        def(
            "embed.remote.enabled",
            Kind::Bool,
            Scope::Device,
            Apply::Live,
            None,
            "Embedding",
            "Switch for the remote model; unset = on when it is configured",
        ),
        def(
            "embed.local.model",
            Kind::Enum {
                choices: vec![LOCAL_MODEL.to_owned()],
            },
            Scope::Workgroup,
            Apply::Live,
            None,
            "Embedding",
            "The local model; setting it configures the local slot",
        ),
        def(
            "embed.local.dimension",
            int(64, 2048),
            Scope::Workgroup,
            Apply::Live,
            Some(Value::Int(i64::from(DEFAULT_DIMENSION))),
            "Embedding",
            "Output dimension of the local model",
        ),
        def(
            "embed.local.max_tokens",
            int(1, MAX_TOKENS as i64),
            Scope::Workgroup,
            Apply::Live,
            Some(Value::Int(DEFAULT_MAX_TOKENS as i64)),
            "Embedding",
            "Content tokens the local model keeps",
        ),
        def(
            "embed.remote.endpoint",
            Kind::Url,
            Scope::Workgroup,
            Apply::Live,
            None,
            "Embedding",
            "The remote endpoint's base URL; setting it configures the remote slot",
        ),
        def(
            "embed.remote.model",
            Kind::String { non_empty: true },
            Scope::Workgroup,
            Apply::Live,
            None,
            "Embedding",
            "The remote model's name at the endpoint",
        ),
        def(
            "embed.remote.dimension",
            int(1, 65536),
            Scope::Workgroup,
            Apply::Live,
            None,
            "Embedding",
            "Output dimension of the remote model",
        ),
        def(
            "embed.remote.key",
            Kind::Secret,
            Scope::Device,
            Apply::Live,
            None,
            "Embedding",
            "The remote endpoint's API key",
        ),
    ]
    .into_iter()
    .chain(gui_defs())
    .collect()
}

/// The theme and the language of every sapphire app's window: the workgroup's, which a device
/// may override.
/// The bridge stores them and never reads them.
fn gui_defs() -> Vec<SettingDef> {
    let def = |key: &str, kind: Kind, default: Value, doc: &str| SettingDef {
        key: key.into(),
        kind,
        scopes: vec![Scope::Workgroup],
        overridable: true,
        apply: Apply::Live,
        default: Some(default),
        section: "Appearance".into(),
        doc: doc.into(),
        process: Process::Bridge,
    };
    let choices = |c: &[&str]| Kind::Enum {
        choices: c.iter().map(|s| (*s).to_owned()).collect(),
    };
    vec![
        def(
            gui::THEME_PREFERENCE,
            choices(&["system", "light", "dark"]),
            Value::String("system".into()),
            "Light, dark, or the system's",
        ),
        def(
            gui::THEME_CONTRAST,
            choices(&["low", "medium", "high"]),
            Value::String("medium".into()),
            "How far apart text and background are",
        ),
        def(
            gui::THEME_ACCENT,
            choices(&gui::ACCENT_CHOICES),
            Value::String("default".into()),
            "The accent: the default, one of the theme's colours, or custom (its hue and chroma)",
        ),
        def(
            gui::THEME_ACCENT_HUE,
            Kind::Int { min: 0, max: 360 },
            Value::Int(255),
            "A custom accent's hue, in degrees",
        ),
        def(
            gui::THEME_ACCENT_CHROMA,
            Kind::Int { min: 0, max: 250 },
            Value::Int(150),
            "A custom accent's chroma, in thousandths",
        ),
        def(
            gui::LOCALE,
            Kind::String { non_empty: true },
            Value::String("system".into()),
            "The language: system, or a language tag such as ja or en-US",
        ),
    ]
}

/// The bridge's settings owner.
pub struct BridgeOwner {
    dir: BridgeDir,
}

impl BridgeOwner {
    /// The owner of the settings in `dir` (and of its workgroup's, if it has one).
    pub fn new(dir: BridgeDir) -> BridgeOwner {
        BridgeOwner { dir }
    }
}

/// A registry with the bridge as its only owner.
pub fn registry(dir: &BridgeDir) -> sapphire_framework_settings::Result<Registry> {
    Registry::new(
        vec![Arc::new(BridgeOwner::new(dir.clone()))],
        &[("net", 0), ("embed", 0), ("gui", 0)],
    )
}

fn failure(file: &std::path::Path, message: impl ToString) -> FieldError {
    FieldError {
        key: None,
        file: Some(file.to_path_buf()),
        message: message.to_string(),
    }
}

/// Parse `text` as a whole config file, naming only the line and column on failure.
///
/// TOML's own error text quotes the offending line, which may hold any setting's value.
fn parse_quiet<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    toml::from_str(text).map_err(|e: toml::de::Error| {
        sapphire_framework_settings::describe_toml_error(text, e.span(), e.message())
    })
}

impl SettingsOwner for BridgeOwner {
    fn defs(&self) -> Vec<SettingDef> {
        bridge_defs()
    }

    fn file(&self, scope: Scope) -> sapphire_framework_settings::Result<PathBuf> {
        match scope {
            Scope::Device => Ok(self.dir.config_toml()),
            Scope::Workgroup => Ok(match Workgroup::open(&self.dir) {
                Ok(Some(workgroup)) => workgroup.config_toml(),
                Ok(None) => self.dir.config_toml(),
                Err(e) => return Err(std::io::Error::other(e.to_string()).into()),
            }),
            Scope::Workspace => {
                Err(std::io::Error::other("the bridge has no workspace scope").into())
            }
        }
    }

    fn secret_file(&self, key: &str) -> sapphire_framework_settings::Result<PathBuf> {
        match key {
            "embed.remote.key" => Ok(self.dir.embedding_key()),
            other => Err(sapphire_framework_settings::Error::UnknownKey(
                other.to_owned(),
            )),
        }
    }

    fn validate(&self, staged: &Staged) -> Result<(), Vec<FieldError>> {
        let mut errors = Vec::new();
        let device = self.dir.config_toml();
        let workgroup = match Workgroup::open(&self.dir) {
            Ok(wg) => wg.map(|wg| wg.config_toml()),
            Err(e) => return Err(vec![failure(&self.dir.root, e)]),
        };
        let mut check = |path: &std::path::Path, device_file: bool| {
            let text = match staged.read(path) {
                Ok(Some(text)) => text,
                Ok(None) => return,
                Err(e) => return errors.push(failure(path, e)),
            };
            let (net, embed) = if device_file {
                match parse_quiet::<DeviceConfig>(&text) {
                    Ok(c) => (c.net, c.embed),
                    Err(e) => return errors.push(failure(path, e)),
                }
            } else {
                match parse_quiet::<WorkgroupConfig>(&text) {
                    Ok(c) => (c.net, c.embed),
                    Err(e) => return errors.push(failure(path, e)),
                }
            };
            if let Err(e) = embed.models() {
                errors.push(failure(path, e));
            }
            // The error quotes the URL, a setting's value: say only what is wrong.
            if relays(&net, None).is_err() {
                errors.push(FieldError {
                    key: Some("net.relays".into()),
                    file: Some(path.to_path_buf()),
                    message: "holds an entry that is not a relay URL".into(),
                });
            }
        };
        check(&device, true);
        if let Some(path) = &workgroup {
            check(path, false);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn scope_note(&self, scope: Scope) -> Option<String> {
        (scope == Scope::Workgroup && matches!(Workgroup::open(&self.dir), Ok(None)))
            .then(|| NO_WORKGROUP.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sapphire_framework_settings::{EntryValue, Error, Op, Origin, apply, get};

    fn setup() -> (tempfile::TempDir, BridgeDir, Registry) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().to_path_buf()).unwrap();
        let reg = registry(&dir).unwrap();
        (tmp, dir, reg)
    }

    fn set(key: &str, value: Value) -> Op {
        Op::Set {
            key: key.into(),
            value,
        }
    }

    fn s(text: &str) -> Value {
        Value::String(text.into())
    }

    fn default_of(key: &str) -> Option<Value> {
        bridge_defs()
            .into_iter()
            .find(|d| d.key == key)
            .unwrap()
            .default
    }

    #[test]
    fn the_defaults_match_the_structs() {
        let net = NetConfig::default();
        assert_eq!(
            default_of("net.wake_on_sync"),
            Some(Value::Bool(net.wake_on_sync))
        );
        assert_eq!(
            default_of("net.discovery"),
            Some(Value::Bool(net.discovery))
        );
        assert_eq!(
            default_of("net.use_default_relays"),
            Some(Value::Bool(net.use_default_relays))
        );
        assert_eq!(
            default_of("net.relays"),
            Some(Value::List(
                net.relays.into_iter().map(Value::String).collect()
            ))
        );
        let local = sapphire_bridge_api::LocalModel::default();
        assert_eq!(
            default_of("embed.local.dimension"),
            Some(Value::Int(i64::from(local.dimension)))
        );
        assert_eq!(
            default_of("embed.local.max_tokens"),
            Some(Value::Int(local.max_tokens as i64))
        );
        // `embed.local.model` has no default, so an unset model reads as "not configured";
        // its only choice is the struct's default.
        assert_eq!(default_of("embed.local.model"), None);
        let def = bridge_defs()
            .into_iter()
            .find(|d| d.key == "embed.local.model")
            .unwrap();
        assert_eq!(
            def.kind,
            Kind::Enum {
                choices: vec![local.model]
            }
        );
    }

    #[test]
    fn every_key_is_a_path_of_the_structs() {
        let values: Vec<(&str, Value)> = vec![
            ("net.wake_on_sync", Value::Bool(false)),
            ("net.discovery", Value::Bool(false)),
            (
                "net.relays",
                Value::List(vec![s("https://relay.example.org")]),
            ),
            ("net.use_default_relays", Value::Bool(false)),
            ("embed.cache_dir", s("/var/cache/sapphire")),
            ("embed.local.enabled", Value::Bool(false)),
            ("embed.local.qtype", s("q8_0")),
            ("embed.remote.enabled", Value::Bool(true)),
            ("embed.local.model", s(LOCAL_MODEL)),
            ("embed.local.dimension", Value::Int(512)),
            ("embed.local.max_tokens", Value::Int(256)),
            ("gui.theme.preference", s("dark")),
            ("gui.theme.contrast", s("high")),
            ("gui.theme.accent", s("custom")),
            ("gui.theme.accent_hue", Value::Int(30)),
            ("gui.theme.accent_chroma", Value::Int(200)),
            ("gui.locale", s("ja")),
        ];
        // Every non-secret key is covered, so a new def without a row here fails.
        let remote: Vec<(&str, Value)> = vec![
            ("embed.remote.endpoint", s("https://api.example.org")),
            ("embed.remote.model", s("text-embedding-3-small")),
            ("embed.remote.dimension", Value::Int(1024)),
        ];
        let covered: Vec<&str> = values.iter().chain(&remote).map(|(k, _)| *k).collect();
        for def in bridge_defs() {
            assert!(
                def.kind == Kind::Secret || covered.contains(&def.key.as_str()),
                "{} is not covered",
                def.key
            );
        }
        // Without a workgroup, every key lands in the device file; with one, the workgroup
        // keys land in its own file. Both must validate.
        for with_workgroup in [false, true] {
            let (_tmp, dir, reg) = setup();
            if with_workgroup {
                Workgroup::create(&dir, "wg", "dev", "node").unwrap();
            }
            for (key, value) in &values {
                apply(&reg, None, vec![set(key, value.clone())])
                    .unwrap_or_else(|e| panic!("{key}: {e}"));
            }
            // The remote slot's fields go in one batch: alone they are a half slot.
            let ops = remote.iter().map(|(k, v)| set(k, v.clone())).collect();
            apply(&reg, None, ops).unwrap();
            for (key, value) in values.iter().chain(&remote) {
                assert_eq!(
                    get(&reg, key).unwrap().value,
                    EntryValue::Value(value.clone()),
                    "{key}"
                );
            }
        }
    }

    #[test]
    fn a_partial_remote_slot_is_rejected() {
        let (_tmp, _dir, reg) = setup();
        let err = apply(
            &reg,
            None,
            vec![set("embed.remote.endpoint", s("https://api.example.org"))],
        )
        .unwrap_err();
        assert!(matches!(&err, Error::Rejected(_)), "{err}");
        assert!(err.to_string().contains("`model`"), "{err}");
    }

    #[test]
    fn workgroup_keys_go_to_the_device_file_without_a_workgroup() {
        let (_tmp, dir, reg) = setup();
        apply(&reg, None, vec![set("embed.local.model", s(LOCAL_MODEL))]).unwrap();
        let text = std::fs::read_to_string(dir.config_toml()).unwrap();
        assert!(text.contains("[embed.local]"), "{text}");
        let entry = get(&reg, "embed.local.model").unwrap();
        assert_eq!(entry.origin, Origin::Scope(Scope::Workgroup));
        assert_eq!(entry.note.as_deref(), Some(NO_WORKGROUP));
    }

    #[test]
    fn with_a_workgroup_they_go_to_its_root() {
        let (_tmp, dir, reg) = setup();
        let wg = Workgroup::create(&dir, "wg", "dev", "node").unwrap();
        apply(&reg, None, vec![set("embed.local.model", s(LOCAL_MODEL))]).unwrap();
        let text = std::fs::read_to_string(wg.config_toml()).unwrap();
        assert!(text.contains("[embed.local]"), "{text}");
        assert!(!dir.config_toml().exists());
        let entry = get(&reg, "embed.local.model").unwrap();
        assert_eq!(entry.origin, Origin::Scope(Scope::Workgroup));
        assert_eq!(entry.note, None);
    }

    #[test]
    fn a_bad_relay_url_is_rejected() {
        let (_tmp, dir, reg) = setup();
        // The registry's own URL check would catch a typo first, so write the file by hand.
        std::fs::write(dir.config_toml(), "[net]\nrelays = [\"MARKER-42\"]\n").unwrap();
        let err = apply(&reg, None, vec![set("net.discovery", Value::Bool(false))]).unwrap_err();
        assert!(matches!(&err, Error::Rejected(_)), "{err}");
        assert!(err.to_string().contains("not a relay URL"), "{err}");
        assert!(!err.to_string().contains("MARKER-42"), "{err}");
    }

    #[test]
    fn a_broken_file_is_rejected_without_quoting_it() {
        let (_tmp, dir, reg) = setup();
        // Valid TOML, but not a `DeviceConfig`: the parse error must not quote the line.
        std::fs::write(dir.config_toml(), "[net]\nrelays = \"MARKER-42\"\n").unwrap();
        let err = apply(&reg, None, vec![set("net.discovery", Value::Bool(false))]).unwrap_err();
        assert!(matches!(&err, Error::Rejected(_)), "{err}");
        assert!(err.to_string().contains("line 2"), "{err}");
        assert!(!err.to_string().contains("MARKER-42"), "{err}");
    }

    #[test]
    fn a_batch_touching_only_the_workgroup_file_validates_against_the_device_file() {
        let (_tmp, dir, reg) = setup();
        let wg = Workgroup::create(&dir, "wg", "dev", "node").unwrap();
        std::fs::write(dir.config_toml(), "[net]\nrelays = [\"not a url\"]\n").unwrap();
        let err = apply(&reg, None, vec![set("embed.local.model", s(LOCAL_MODEL))]);
        // The device file is not staged, but it is still read and checked.
        assert!(matches!(err, Err(Error::Rejected(_))));
        assert!(!wg.config_toml().exists());
    }

    #[test]
    fn a_prefix_unset_on_a_host_without_a_workgroup_keeps_the_device_switch() {
        let (_tmp, dir, reg) = setup();
        std::fs::write(
            dir.config_toml(),
            "[embed.remote]\nenabled = false\nendpoint = \"https://api.example.org\"\n\
             model = \"m\"\ndimension = 8\n",
        )
        .unwrap();
        let unset = || {
            vec![Op::Unset {
                key: "embed.remote".into(),
            }]
        };
        // Keys span the device and the workgroup scope: a bare prefix is ambiguous.
        assert!(apply(&reg, None, unset()).is_err());
        apply(&reg, Some(Scope::Workgroup), unset()).unwrap();
        let text = std::fs::read_to_string(dir.config_toml()).unwrap();
        assert!(text.contains("enabled = false"), "{text}");
        for gone in ["endpoint", "model", "dimension"] {
            assert!(!text.contains(gone), "{gone} survived: {text}");
        }
    }

    #[test]
    fn gui_theme_keys_list_their_defaults() {
        let (_tmp, _dir, reg) = setup();
        let entries = sapphire_framework_settings::list(&reg, None, None).unwrap();
        for (key, value) in [
            (gui::THEME_PREFERENCE, s("system")),
            (gui::THEME_CONTRAST, s("medium")),
            (gui::THEME_ACCENT, s("default")),
            (gui::THEME_ACCENT_HUE, Value::Int(255)),
            (gui::THEME_ACCENT_CHROMA, Value::Int(150)),
        ] {
            let entry = entries.iter().find(|e| e.key == key).unwrap();
            assert_eq!(entry.value, EntryValue::Value(value), "{key}");
            assert_eq!(entry.origin, Origin::Default, "{key}");
        }
    }

    #[test]
    fn a_device_gui_value_overrides_the_workgroups() {
        let (_tmp, dir, reg) = setup();
        Workgroup::create(&dir, "wg", "dev", "node").unwrap();
        let contrast = |v| vec![set(gui::THEME_CONTRAST, s(v))];
        apply(&reg, Some(Scope::Workgroup), contrast("high")).unwrap();
        apply(&reg, Some(Scope::Device), contrast("low")).unwrap();
        let entry = get(&reg, gui::THEME_CONTRAST).unwrap();
        assert_eq!(entry.value, EntryValue::Value(s("low")));
        assert_eq!(entry.origin, Origin::Scope(Scope::Device));
        let unset = vec![Op::Unset {
            key: gui::THEME_CONTRAST.into(),
        }];
        apply(&reg, Some(Scope::Device), unset).unwrap();
        let entry = get(&reg, gui::THEME_CONTRAST).unwrap();
        assert_eq!(entry.value, EntryValue::Value(s("high")));
        assert_eq!(entry.origin, Origin::Scope(Scope::Workgroup));
    }

    #[test]
    fn gui_values_out_of_range_are_rejected() {
        let (_tmp, _dir, reg) = setup();
        for (key, value) in [
            (gui::THEME_ACCENT_HUE, Value::Int(361)),
            (gui::THEME_ACCENT_CHROMA, Value::Int(251)),
            (gui::THEME_CONTRAST, s("loud")),
            (gui::THEME_ACCENT, s("accent")),
            (gui::THEME_ACCENT, s("mauve")),
        ] {
            assert!(apply(&reg, None, vec![set(key, value)]).is_err(), "{key}");
        }
    }

    #[test]
    fn gui_locale_lists_system_by_default() {
        let (_tmp, _dir, reg) = setup();
        let entries = sapphire_framework_settings::list(&reg, None, None).unwrap();
        let entry = entries.iter().find(|e| e.key == gui::LOCALE).unwrap();
        assert_eq!(entry.value, EntryValue::Value(s("system")));
        assert_eq!(entry.origin, Origin::Default);
    }

    #[test]
    fn a_device_locale_overrides_the_workgroups() {
        let (_tmp, dir, reg) = setup();
        Workgroup::create(&dir, "wg", "dev", "node").unwrap();
        let locale = |v| vec![set(gui::LOCALE, s(v))];
        apply(&reg, Some(Scope::Workgroup), locale("ja")).unwrap();
        let entry = get(&reg, gui::LOCALE).unwrap();
        assert_eq!(entry.value, EntryValue::Value(s("ja")));
        assert_eq!(entry.origin, Origin::Scope(Scope::Workgroup));
        apply(&reg, Some(Scope::Device), locale("en-US")).unwrap();
        let entry = get(&reg, gui::LOCALE).unwrap();
        assert_eq!(entry.value, EntryValue::Value(s("en-US")));
        assert_eq!(entry.origin, Origin::Scope(Scope::Device));
        let unset = vec![Op::Unset {
            key: gui::LOCALE.into(),
        }];
        apply(&reg, Some(Scope::Device), unset).unwrap();
        let entry = get(&reg, gui::LOCALE).unwrap();
        assert_eq!(entry.value, EntryValue::Value(s("ja")));
        assert_eq!(entry.origin, Origin::Scope(Scope::Workgroup));
    }

    #[test]
    fn an_empty_locale_is_rejected() {
        let (_tmp, _dir, reg) = setup();
        assert!(apply(&reg, None, vec![set(gui::LOCALE, s(""))]).is_err());
    }

    #[test]
    fn a_gui_table_in_both_files_validates() {
        let (_tmp, dir, reg) = setup();
        let wg = Workgroup::create(&dir, "wg", "dev", "node").unwrap();
        let table = "[gui.theme]\ncontrast = \"high\"\n";
        std::fs::write(dir.config_toml(), table).unwrap();
        std::fs::write(wg.config_toml(), table).unwrap();
        apply(&reg, None, vec![set("net.discovery", Value::Bool(false))]).unwrap();
        apply(&reg, None, vec![set("embed.local.model", s(LOCAL_MODEL))]).unwrap();
    }
}
