//! Schema and value types of the settings registry.
//!
//! A setting is addressed by its key (its TOML path in a scope's `config.toml`) and
//! described by a [`SettingDef`]: its [`Kind`], the [`Scope`]s it may live in, and
//! whether a change applies live or needs a restart. This module is types only, so the
//! bridge, the app servers and the CLIs can share it without sharing a store.

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Where a setting is stored. Ordered as declared: device, workgroup, workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// This machine only.
    Device,
    /// Shared by every member of a workgroup.
    Workgroup,
    /// One workspace.
    Workspace,
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Scope::Device => "device",
            Scope::Workgroup => "workgroup",
            Scope::Workspace => "workspace",
        })
    }
}

impl FromStr for Scope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "device" => Ok(Scope::Device),
            "workgroup" => Ok(Scope::Workgroup),
            "workspace" => Ok(Scope::Workspace),
            other => Err(format!(
                "unknown scope `{other}` (expected device, workgroup or workspace)"
            )),
        }
    }
}

/// The type of a setting's value, and so what it accepts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// `true`/`false`, also written `on`/`off`.
    Bool,
    /// An integer within `min..=max`.
    Int {
        /// Smallest accepted value.
        min: i64,
        /// Largest accepted value.
        max: i64,
    },
    /// One of a fixed set of strings.
    Enum {
        /// The accepted strings.
        choices: Vec<String>,
    },
    /// Free text.
    String {
        /// Reject the empty string.
        non_empty: bool,
    },
    /// An `http` or `https` URL.
    Url,
    /// A filesystem path.
    Path,
    /// A `host:port` socket address.
    SocketAddr,
    /// A list of items of one kind.
    List {
        /// The kind of every item.
        item: Box<Kind>,
    },
    /// A credential: device-scope, never read back, never taken from text on a command line.
    Secret,
}

/// When a changed setting takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Apply {
    /// Picked up by the running process.
    Live,
    /// Needs a restart.
    Restart,
}

/// A typed setting value.
///
/// Untagged on the wire, so a JSON `5` is `Int` and `[1,2]` is a list of `Int`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    /// A boolean.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A string (also URLs, paths, addresses, enum choices).
    String(String),
    /// A list.
    List(Vec<Value>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::String(s) => f.write_str(s),
            Value::List(items) => {
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{item}")?;
                }
                Ok(())
            }
        }
    }
}

/// Which process owns a setting: the one that stores, validates and applies it.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Process {
    /// The bridge.
    #[default]
    Bridge,
    /// The app server.
    App,
}

/// The registry's description of one setting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingDef {
    /// The TOML path, e.g. `embed.remote.endpoint`.
    pub key: String,
    /// What the value must look like.
    pub kind: Kind,
    /// The scopes it may be set in; the first is its home.
    pub scopes: Vec<Scope>,
    /// Whether a device may override a value set in a wider scope.
    pub overridable: bool,
    /// When a change takes effect.
    pub apply: Apply,
    /// The value in effect when no scope sets it.
    pub default: Option<Value>,
    /// The heading it is listed under.
    pub section: String,
    /// One-line help text.
    pub doc: String,
    /// The process that owns it; an older peer's definitions read as the bridge's.
    #[serde(default)]
    pub process: Process,
}

impl SettingDef {
    /// The scope the setting normally lives in.
    pub fn home(&self) -> Scope {
        self.scopes[0]
    }

    /// Whether the setting may be set in `scope`: its home, any other listed scope, or
    /// `Device` when it is overridable.
    pub fn allows(&self, scope: Scope) -> bool {
        scope == self.home()
            || (scope == Scope::Device && self.overridable)
            || self.scopes.contains(&scope)
    }
}

/// Whether a secret has a value; the value itself is never exposed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretState {
    /// A value is stored.
    Set,
    /// Nothing is stored.
    Unset,
}

/// What `get`/`list` report for a key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryValue {
    /// The effective value.
    Value(Value),
    /// A secret: only whether it is set.
    Secret(SecretState),
    /// No value and no default.
    Unset,
}

/// Where the effective value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// Set in this scope.
    Scope(Scope),
    /// The registry default.
    Default,
}

/// One setting as reported to a client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// The setting's key.
    pub key: String,
    /// Its effective value.
    pub value: EntryValue,
    /// Where that value came from.
    pub origin: Origin,
    /// A restart-only change has been written but is not yet in effect.
    pub pending_restart: bool,
    /// Extra remark for the user (e.g. an unknown key kept in the file).
    pub note: Option<String>,
}

impl Kind {
    /// Parse user text into a value of this kind, then [`check`](Kind::check) it.
    pub fn parse(&self, text: &str) -> Result<Value, String> {
        let value = match self {
            Kind::Bool => match text {
                "true" | "on" => Value::Bool(true),
                "false" | "off" => Value::Bool(false),
                _ => return Err("not a boolean (expected one of true, false, on, off)".into()),
            },
            Kind::Int { .. } => Value::Int(
                text.trim()
                    .parse::<i64>()
                    .map_err(|_| "not an integer".to_string())?,
            ),
            Kind::Enum { .. } | Kind::String { .. } | Kind::Url | Kind::Path | Kind::SocketAddr => {
                Value::String(text.to_string())
            }
            Kind::List { item } => Value::List(
                split_list(text)
                    .iter()
                    .map(|part| item.parse(part))
                    .collect::<Result<_, _>>()?,
            ),
            Kind::Secret => {
                return Err(
                    "a secret cannot be given as text; read it with --stdin or a prompt".into(),
                );
            }
        };
        self.check(&value)?;
        Ok(value)
    }

    /// Check an already-typed value against this kind.
    pub fn check(&self, value: &Value) -> Result<(), String> {
        match (self, value) {
            (Kind::Bool, Value::Bool(_)) => Ok(()),
            (Kind::Int { min, max }, Value::Int(i)) => {
                if (*min..=*max).contains(i) {
                    Ok(())
                } else {
                    Err(format!("out of range (expected {min}..={max})"))
                }
            }
            (Kind::Enum { choices }, Value::String(s)) => {
                if choices.contains(s) {
                    Ok(())
                } else {
                    Err(format!("not one of the choices ({})", choices.join(", ")))
                }
            }
            (Kind::String { non_empty }, Value::String(s)) => {
                if *non_empty && s.is_empty() {
                    Err("must not be empty".into())
                } else {
                    Ok(())
                }
            }
            (Kind::Url, Value::String(s)) => {
                let rest = s
                    .strip_prefix("http://")
                    .or_else(|| s.strip_prefix("https://"));
                match rest {
                    Some(rest) if !rest.is_empty() => Ok(()),
                    _ => Err("not an http or https URL".into()),
                }
            }
            (Kind::Path, Value::String(s)) => {
                if std::path::Path::new(s).as_os_str().is_empty() {
                    Err("path must not be empty".into())
                } else {
                    Ok(())
                }
            }
            (Kind::SocketAddr, Value::String(s)) => s
                .parse::<SocketAddr>()
                .map(|_| ())
                .map_err(|_| "not a host:port socket address".to_string()),
            (Kind::List { item }, Value::List(items)) => {
                items.iter().try_for_each(|v| item.check(v))
            }
            (Kind::Secret, Value::String(_)) => Ok(()),
            // A secret's value is never shown, not even a wrong one.
            (Kind::Secret, _) => Err("a secret must be a string".into()),
            // Error text never carries the value: it reaches logs, the GUI and CLI output.
            (kind, _) => Err(format!("has the wrong type (expected {})", kind.name())),
        }
    }

    /// A short name for this kind, for error text.
    fn name(&self) -> &'static str {
        match self {
            Kind::Bool => "bool",
            Kind::Int { .. } => "integer",
            Kind::Enum { .. } => "choice",
            Kind::String { .. } => "string",
            Kind::Url => "URL",
            Kind::Path => "path",
            Kind::SocketAddr => "socket address",
            Kind::List { .. } => "list",
            Kind::Secret => "secret",
        }
    }
}

/// Split list text: a comma-separated string or a TOML-style `["a", "b"]` array.
fn split_list(text: &str) -> Vec<String> {
    let text = text.trim();
    let text = text
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(text);
    if text.trim().is_empty() {
        return Vec::new();
    }
    text.split(',')
        .map(|part| {
            let part = part.trim();
            part.strip_prefix('"')
                .and_then(|p| p.strip_suffix('"'))
                .unwrap_or(part)
                .to_string()
        })
        .collect()
}

/// The keys of the GUI's settings: stored by the bridge, read by the apps' windows.
pub mod gui {
    /// Light, dark, or the system's.
    pub const THEME_PREFERENCE: &str = "gui.theme.preference";
    /// How far apart text and background are.
    pub const THEME_CONTRAST: &str = "gui.theme.contrast";
    /// The accent: one of [`ACCENT_CHOICES`].
    pub const THEME_ACCENT: &str = "gui.theme.accent";
    /// A custom accent's hue, in whole degrees.
    pub const THEME_ACCENT_HUE: &str = "gui.theme.accent_hue";
    /// A custom accent's chroma, in thousandths.
    pub const THEME_ACCENT_CHROMA: &str = "gui.theme.accent_chroma";
    /// The accent's values: `default`, one of the theme's colours by name, or `custom`, whose
    /// hue and chroma are [`THEME_ACCENT_HUE`] and [`THEME_ACCENT_CHROMA`].
    pub const ACCENT_CHOICES: [&str; 10] = [
        "default", "neutral", "red", "orange", "yellow", "green", "cyan", "blue", "purple",
        "custom",
    ];
    /// The language of the apps' windows: `system`, or a language tag such as `ja` or
    /// `en-US`.
    pub const LOCALE: &str = "gui.locale";
    /// Every theme key.
    pub const THEME_KEYS: [&str; 5] = [
        THEME_PREFERENCE,
        THEME_CONTRAST,
        THEME_ACCENT,
        THEME_ACCENT_HUE,
        THEME_ACCENT_CHROMA,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_typed_secret_is_not_echoed() {
        let value = Value::List(vec![Value::String("hunter2-sekrit".into())]);
        let err = Kind::Secret.check(&value).unwrap_err();
        assert!(!err.contains("hunter2"), "{err}");
        let err = Kind::Secret.check(&Value::Int(424242)).unwrap_err();
        assert!(!err.contains("424242"), "{err}");
    }

    #[test]
    fn a_rejected_value_is_not_echoed() {
        let list = Kind::List {
            item: Box::new(Kind::Url),
        };
        let parsed = [
            (Kind::Bool, "MARKER-42"),
            (Kind::Int { min: 1, max: 10 }, "MARKER-42"),
            (Kind::Int { min: 1, max: 10 }, "4242424"),
            (
                Kind::Enum {
                    choices: vec!["a".into()],
                },
                "MARKER-42",
            ),
            (Kind::Url, "MARKER-42"),
            (Kind::SocketAddr, "MARKER-42"),
            (list, "MARKER-42"),
        ];
        for (kind, text) in parsed {
            let err = kind.parse(text).unwrap_err();
            assert!(!err.contains(text), "{kind:?}: {err}");
        }
        let checked = [
            (Kind::Int { min: 1, max: 10 }, Value::Int(4242424)),
            (Kind::Bool, Value::String("MARKER-42".into())),
            (
                Kind::Int { min: 1, max: 10 },
                Value::String("MARKER-42".into()),
            ),
            (
                Kind::Enum {
                    choices: vec!["a".into()],
                },
                Value::List(vec![Value::String("MARKER-42".into())]),
            ),
        ];
        for (kind, value) in checked {
            let err = kind.check(&value).unwrap_err();
            assert!(!err.contains("MARKER-42"), "{kind:?}: {err}");
            assert!(!err.contains("4242424"), "{kind:?}: {err}");
            assert!(
                !err.contains("choices"),
                "names the kind, not its Debug: {err}"
            );
        }
    }

    fn def(key: &str, home: Scope) -> SettingDef {
        SettingDef {
            key: key.into(),
            kind: Kind::Bool,
            scopes: vec![home],
            overridable: false,
            apply: Apply::Live,
            default: None,
            section: "test".into(),
            doc: "test".into(),
            process: Process::Bridge,
        }
    }

    #[test]
    fn an_older_def_reads_as_the_bridges() {
        let json = r#"{"key":"a.b","kind":"bool","scopes":["device"],"overridable":false,
            "apply":"live","default":null,"section":"s","doc":"d"}"#;
        let def: SettingDef = serde_json::from_str(json).unwrap();
        assert_eq!(def.process, Process::Bridge);
        assert_eq!(serde_json::to_value(Process::App).unwrap(), "app");
    }

    #[test]
    fn bools_accept_on_off_true_false() {
        for (t, v) in [
            ("true", true),
            ("on", true),
            ("false", false),
            ("off", false),
        ] {
            assert_eq!(Kind::Bool.parse(t).unwrap(), Value::Bool(v));
        }
        assert!(
            Kind::Bool
                .parse("yes")
                .unwrap_err()
                .contains("true, false, on, off")
        );
    }
    #[test]
    fn ints_name_their_range() {
        let k = Kind::Int { min: 64, max: 2048 };
        assert_eq!(k.parse("1024").unwrap(), Value::Int(1024));
        assert!(k.parse("32").unwrap_err().contains("64..=2048"));
    }
    #[test]
    fn enums_list_their_choices() {
        let k = Kind::Enum {
            choices: vec!["q8_0".into(), "f16".into()],
        };
        assert!(k.parse("q4").unwrap_err().contains("q8_0, f16"));
    }
    #[test]
    fn urls_are_http_only() {
        assert!(Kind::Url.parse("https://a.example").is_ok());
        assert!(Kind::Url.parse("ftp://a.example").is_err());
    }
    #[test]
    fn lists_take_commas_or_a_toml_array() {
        let k = Kind::List {
            item: Box::new(Kind::Url),
        };
        let want = Value::List(vec![
            Value::String("https://a".into()),
            Value::String("https://b".into()),
        ]);
        assert_eq!(k.parse("https://a,https://b").unwrap(), want);
        assert_eq!(k.parse(r#"["https://a", "https://b"]"#).unwrap(), want);
        assert_eq!(k.parse("").unwrap(), Value::List(vec![]));
    }
    #[test]
    fn secrets_are_never_parsed_from_text() {
        assert!(
            Kind::Secret
                .parse("sk-123")
                .unwrap_err()
                .contains("--stdin")
        );
    }
    #[test]
    fn device_is_allowed_only_when_overridable() {
        let mut d = def("embed.remote.model", Scope::Workgroup);
        assert!(!d.allows(Scope::Device));
        d.overridable = true;
        assert!(d.allows(Scope::Device));
    }
    #[test]
    fn values_round_trip_through_json() {
        let v = Value::List(vec![Value::Int(1), Value::Int(2)]);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "[1,2]");
        assert_eq!(serde_json::from_str::<Value>(&json).unwrap(), v);
        for v in [Value::Bool(true), Value::Int(-7), Value::String("x".into())] {
            let back: Value = serde_json::from_str(&serde_json::to_string(&v).unwrap()).unwrap();
            assert_eq!(back, v);
        }
    }
    #[test]
    fn scopes_display_and_parse() {
        for s in [Scope::Device, Scope::Workgroup, Scope::Workspace] {
            assert_eq!(s.to_string().parse::<Scope>().unwrap(), s);
        }
        assert_eq!(Scope::Device.to_string(), "device");
        assert!("nope".parse::<Scope>().is_err());
    }
    #[test]
    fn check_validates_typed_values() {
        let k = Kind::Int { min: 1, max: 3 };
        assert!(k.check(&Value::Int(2)).is_ok());
        assert!(k.check(&Value::Int(9)).is_err());
        assert!(k.check(&Value::Bool(true)).is_err());
        assert!(
            Kind::String { non_empty: true }
                .check(&Value::String(String::new()))
                .is_err()
        );
        assert!(Kind::SocketAddr.parse("127.0.0.1:80").is_ok());
        assert!(Kind::SocketAddr.parse("nope").is_err());
        assert!(Kind::Secret.check(&Value::String("x".into())).is_ok());
    }
    #[test]
    fn the_crate_stays_free_of_framework_crates() {
        let manifest = include_str!("../Cargo.toml");
        let deps = manifest.split("[dependencies]").nth(1).unwrap();
        assert!(!deps.contains("sapphire-framework-"));
    }
}
