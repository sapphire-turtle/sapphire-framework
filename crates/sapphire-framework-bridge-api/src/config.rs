//! The `config.*` methods: the registry's settings, read and written by key.
//!
//! The types themselves ([`Scope`], [`Value`], [`Entry`], [`SettingDef`]) are the settings
//! API's; this module only wraps them in parameters and results.

use std::fmt;

use crate::settings::{Entry, Scope, SettingDef, Value};
use serde::{Deserialize, Serialize};

/// Parameters of `config.list`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigListParams {
    /// Only what this scope's file holds; `None` lists every key's effective value.
    #[serde(default)]
    pub scope: Option<Scope>,
}

/// Parameters of `config.get`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigGetParams {
    /// The key.
    pub key: String,
}

/// One key and the value to give it.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigEntry {
    /// The key.
    pub key: String,
    /// The new value.
    pub value: Value,
}

/// Parameters of `config.set`: a batch, applied whole or not at all.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigSetParams {
    /// The scope to write; `None` writes each key to its home scope.
    #[serde(default)]
    pub scope: Option<Scope>,
    /// The changes.
    pub entries: Vec<ConfigEntry>,
}

// The values may be secrets, and this crate cannot tell which keys are: so no value is
// ever printed.
impl fmt::Debug for ConfigEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigEntry")
            .field("key", &self.key)
            .field("value", &format_args!("<secret>"))
            .finish()
    }
}

impl fmt::Debug for ConfigSetParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConfigSetParams")
            .field("scope", &self.scope)
            .field("entries", &self.entries)
            .finish()
    }
}

/// Parameters of `config.unset`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigUnsetParams {
    /// The scope to remove from; `None` removes each key from its home scope.
    #[serde(default)]
    pub scope: Option<Scope>,
    /// The keys, or prefixes of several keys.
    pub keys: Vec<String>,
}

/// Result of `config.schema`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigSchemaResult {
    /// Every setting the bridge knows.
    pub settings: Vec<SettingDef>,
}

/// Result of `config.list`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigListResult {
    /// The settings, in schema order.
    pub entries: Vec<Entry>,
}

/// Result of `config.set` and `config.unset`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigWriteResult {
    /// The keys the batch touched.
    pub changed: Vec<String>,
    /// Every key written but not yet in effect, from this batch or an earlier one.
    pub pending_restart: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_set_params_never_print_values() {
        let params = ConfigSetParams {
            scope: None,
            entries: vec![ConfigEntry {
                key: "embed.remote.key".into(),
                value: Value::String("sk-hunter2".into()),
            }],
        };
        let shown = format!("{params:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("<secret>") && shown.contains("embed.remote.key"));
    }

    #[test]
    fn set_params_round_trip() {
        let params = ConfigSetParams {
            scope: Some(Scope::Device),
            entries: vec![ConfigEntry {
                key: "net.discovery".into(),
                value: Value::Bool(false),
            }],
        };
        let json = serde_json::to_string(&params).unwrap();
        assert_eq!(
            serde_json::from_str::<ConfigSetParams>(&json).unwrap(),
            params
        );
    }
}
