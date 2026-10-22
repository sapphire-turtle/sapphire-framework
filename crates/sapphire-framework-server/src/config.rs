//! The server's own settings: `http.*` and `update.check` in `<config dir>/config.toml`.
//!
//! What a server offers beyond its IPC endpoint is the host's choice, so it lives in the
//! application's config directory beside `workspace.toml`, with the application's own
//! device settings (#314). Every setting is off or empty by default, and a missing file is
//! all defaults. The server reads them when it starts, through [`Settings`]; the file used
//! to be `server.toml`, which the server moves over once.
//!
//! ```toml
//! [http]
//! # Where the HTTP listener binds. Overrides the application's default address.
//! listen = "127.0.0.1:3173"
//! # The Host names clients use beyond loopback (a LAN name, a Tailscale name).
//! allowed_hosts = ["box.tailnet.ts.net"]
//!
//! [http.mcp]
//! # Serve the application's MCP tools at /mcp, behind an external device's token.
//! enabled = true
//!
//! [http.metrics]
//! # Serve Prometheus metrics at /metrics, without a token (counts and states only).
//! enabled = true
//!
//! [update]
//! # Do not ask GitHub for newer releases of this application. On by default, for an
//! # application that names where its releases are.
//! check = false
//! ```

use std::net::SocketAddr;

use sapphire_framework_settings::Value;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::settings::Settings;

/// The device settings file's name inside the application's config directory.
pub const SERVER_CONFIG_FILE: &str = "config.toml";

/// Where the old `[mcp] http` went: `deny_unknown_fields` alone would only say that `mcp`
/// is unknown.
pub(crate) const MCP_MOVED: &str = "[mcp] http moved to [http.mcp] enabled";

/// The server's settings, as `config.toml` holds them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// The HTTP listener.
    #[serde(default)]
    pub http: HttpConfig,
    /// Looking for newer releases.
    #[serde(default)]
    pub update: UpdateConfig,
}

/// The `[http]` table: the listener every HTTP route shares.
///
/// The listener binds only when something is served over it. Every route on it but
/// `/metrics` is behind an external device's bearer token (`sapphire-framework-keys`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpConfig {
    /// Where to bind. `None` is the application's default address, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen: Option<SocketAddr>,
    /// The `Host` names clients reach the server by, beyond loopback.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    /// MCP over HTTP.
    #[serde(default, skip_serializing_if = "is_default")]
    pub mcp: HttpMcpConfig,
    /// Prometheus metrics over HTTP.
    #[serde(default, skip_serializing_if = "is_default")]
    pub metrics: HttpMetricsConfig,
}

/// The `[http.mcp]` table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpMcpConfig {
    /// Serve the application's MCP tools at `/mcp` on the HTTP listener. Off by default.
    #[serde(default)]
    pub enabled: bool,
}

/// The `[http.metrics]` table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpMetricsConfig {
    /// Serve Prometheus metrics at `/metrics` on the HTTP listener. Off by default.
    #[serde(default)]
    pub enabled: bool,
}

/// Serde's `skip_serializing_if` wants a function, and a table that is all defaults is
/// not worth writing out.
fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// The `[update]` table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {
    /// Ask the application's release page for newer releases, once at start and once a
    /// day. On by default; it does nothing for an application that names no release page.
    #[serde(default = "UpdateConfig::default_check")]
    pub check: bool,
}

impl UpdateConfig {
    fn default_check() -> bool {
        true
    }
}

impl Default for UpdateConfig {
    fn default() -> Self {
        UpdateConfig { check: true }
    }
}

impl ServerConfig {
    /// The framework's settings out of `settings`, as they were when it was read.
    pub fn from_settings(settings: &Settings) -> Result<ServerConfig> {
        let flag = |key: &str, default: bool| match settings.get(key) {
            None => Ok(default),
            Some(Value::Bool(b)) => Ok(b),
            Some(_) => Err(invalid(key)),
        };
        let listen = match settings.get("http.listen") {
            None => None,
            Some(Value::String(s)) => Some(s.parse().map_err(|_| invalid("http.listen"))?),
            Some(_) => return Err(invalid("http.listen")),
        };
        let allowed_hosts = match settings.get("http.allowed_hosts") {
            None => Vec::new(),
            Some(_) => settings
                .get_as::<Vec<String>>("http.allowed_hosts")
                .ok_or_else(|| invalid("http.allowed_hosts"))?,
        };
        Ok(ServerConfig {
            http: HttpConfig {
                listen,
                allowed_hosts,
                mcp: HttpMcpConfig {
                    enabled: flag("http.mcp.enabled", false)?,
                },
                metrics: HttpMetricsConfig {
                    enabled: flag("http.metrics.enabled", false)?,
                },
            },
            update: UpdateConfig {
                check: flag("update.check", true)?,
            },
        })
    }
}

/// A framework setting whose value is not one it takes; the value itself is not repeated.
fn invalid(key: &str) -> Error {
    Error::Config(format!(
        "`{key}` in {SERVER_CONFIG_FILE} does not hold a valid value"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// The server config a `config.toml` of `text` (none for `None`) reads as.
    fn from_text(text: Option<&str>) -> Result<ServerConfig> {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = sapphire_workspace::AppContext::new("configtest");
        ctx.set_config_dir(tmp.path().to_path_buf());
        if let Some(text) = text {
            std::fs::write(tmp.path().join(SERVER_CONFIG_FILE), text).unwrap();
        }
        ServerConfig::from_settings(&Settings::load(&ctx, Arc::new(()))?)
    }

    #[test]
    fn a_missing_file_is_all_defaults_and_mcp_is_off() {
        let config = from_text(None).unwrap();
        assert_eq!(config, ServerConfig::default());
        assert!(!config.http.mcp.enabled);
        assert!(!config.http.metrics.enabled);
        assert_eq!(config.http.listen, None);
        assert!(config.update.check);
    }

    #[test]
    fn the_file_turns_the_update_check_off() {
        assert!(
            !from_text(Some("[update]\ncheck = false\n"))
                .unwrap()
                .update
                .check
        );
        assert!(from_text(Some("[update]\n")).unwrap().update.check);
    }

    #[test]
    fn the_file_turns_mcp_on_and_names_the_listener() {
        let config = from_text(Some(
            "[http]\nlisten = \"127.0.0.1:3173\"\nallowed_hosts = [\"box.example\"]\n\n[http.mcp]\nenabled = true\n",
        ))
        .unwrap();
        assert!(config.http.mcp.enabled);
        assert_eq!(config.http.listen, Some("127.0.0.1:3173".parse().unwrap()));
        assert_eq!(config.http.allowed_hosts, vec!["box.example".to_owned()]);
    }

    #[test]
    fn a_misspelt_setting_is_an_error_not_a_default() {
        assert!(from_text(Some("[http.mcp]\nenabeld = true\n")).is_err());
        // A table of another name may be the application's own: the file is shared.
        assert!(from_text(Some("[htp]\nlisten = \"127.0.0.1:1\"\n")).is_ok());
    }

    #[test]
    fn the_file_turns_metrics_on_under_http() {
        let config = from_text(Some(
            "[http]\nlisten = \"127.0.0.1:3173\"\n\n[http.metrics]\nenabled = true\n",
        ))
        .unwrap();
        assert!(config.http.metrics.enabled);
        assert!(!config.http.mcp.enabled);
    }

    #[test]
    fn the_application_keeps_its_own_table() {
        let config = from_text(Some("[configtest]\nanything = 1\n")).unwrap();
        assert_eq!(config, ServerConfig::default());
    }
}
