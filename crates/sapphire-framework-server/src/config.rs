//! The server's own settings: `<config dir>/server.toml`.
//!
//! What a server offers beyond its IPC endpoint is the host's choice, so it lives in the
//! application's config directory beside `workspace.toml`. Every setting is off or empty by
//! default, and a missing file is all defaults. The server reads the file when it starts.
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
//! # Serve Prometheus metrics at /metrics, behind an external device's token.
//! enabled = true
//!
//! [update]
//! # Do not ask GitHub for newer releases of this application. On by default, for an
//! # application that names where its releases are.
//! check = false
//! ```

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use sapphire_workspace::AppContext;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The file's name inside the application's config directory.
pub const SERVER_CONFIG_FILE: &str = "server.toml";

/// What `server.toml` holds.
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
/// The listener binds only when something is served over it. Every route on it is behind
/// an external device's bearer token (`sapphire-framework-keys`).
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
    /// `ctx`'s application's file; `None` while its config directory is unresolved.
    pub fn path_for_app(ctx: &AppContext) -> Option<PathBuf> {
        ctx.try_config_dir().map(|dir| dir.join(SERVER_CONFIG_FILE))
    }

    /// `ctx`'s application's settings. A context whose config directory was never resolved
    /// has no file to read, and gets the defaults — as it gets no log file.
    pub fn load_for_app(ctx: &AppContext) -> Result<ServerConfig> {
        match ServerConfig::path_for_app(ctx) {
            Some(path) => ServerConfig::load(&path),
            None => Ok(ServerConfig::default()),
        }
    }

    /// Read the file at `path`; a missing file is all defaults.
    ///
    /// A file that does not parse is an error rather than defaults: a host that asked for
    /// something and got nothing would not find out why.
    pub fn load(path: &Path) -> Result<ServerConfig> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                // `[mcp] http` moved under `[http]`. `deny_unknown_fields` alone would only
                // say that `mcp` is unknown, so say where the setting went. A text that is
                // not a table falls through to the parse below, which reports why.
                if let Ok(table) = text.parse::<toml::Table>()
                    && table.contains_key("mcp")
                {
                    return Err(Error::Config(format!(
                        "{}: [mcp] http moved to [http.mcp] enabled",
                        path.display()
                    )));
                }
                toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ServerConfig::default()),
            Err(e) => Err(Error::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_all_defaults_and_mcp_is_off() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = ServerConfig::load(&tmp.path().join(SERVER_CONFIG_FILE)).unwrap();
        assert_eq!(config, ServerConfig::default());
        assert!(!config.http.mcp.enabled);
        assert!(!config.http.metrics.enabled);
        assert_eq!(config.http.listen, None);
        assert!(config.update.check);
    }

    #[test]
    fn the_file_turns_the_update_check_off() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(SERVER_CONFIG_FILE);
        std::fs::write(&path, "[update]\ncheck = false\n").unwrap();
        assert!(!ServerConfig::load(&path).unwrap().update.check);
        std::fs::write(&path, "[update]\n").unwrap();
        assert!(ServerConfig::load(&path).unwrap().update.check);
    }

    #[test]
    fn the_file_turns_mcp_on_and_names_the_listener() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(SERVER_CONFIG_FILE);
        std::fs::write(
            &path,
            "[http]\nlisten = \"127.0.0.1:3173\"\nallowed_hosts = [\"box.example\"]\n\n[http.mcp]\nenabled = true\n",
        )
        .unwrap();
        let config = ServerConfig::load(&path).unwrap();
        assert!(config.http.mcp.enabled);
        assert_eq!(config.http.listen, Some("127.0.0.1:3173".parse().unwrap()));
        assert_eq!(config.http.allowed_hosts, vec!["box.example".to_owned()]);
    }

    #[test]
    fn a_misspelt_setting_is_an_error_not_a_default() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(SERVER_CONFIG_FILE);
        std::fs::write(&path, "[http.mcp]\nenabeld = true\n").unwrap();
        assert!(ServerConfig::load(&path).is_err());
    }

    #[test]
    fn the_file_turns_metrics_on_under_http() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(SERVER_CONFIG_FILE);
        std::fs::write(
            &path,
            "[http]\nlisten = \"127.0.0.1:3173\"\n\n[http.metrics]\nenabled = true\n",
        )
        .unwrap();
        let config = ServerConfig::load(&path).unwrap();
        assert!(config.http.metrics.enabled);
        assert!(!config.http.mcp.enabled);
    }

    #[test]
    fn an_old_mcp_table_says_where_the_setting_went() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(SERVER_CONFIG_FILE);
        std::fs::write(&path, "[mcp]\nhttp = true\n").unwrap();
        let err = ServerConfig::load(&path).unwrap_err().to_string();
        assert!(
            err.contains("[mcp] http moved to [http.mcp] enabled"),
            "{err}"
        );
    }
}
