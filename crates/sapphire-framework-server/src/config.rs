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
//! [mcp]
//! # Serve the application's MCP tools at /mcp, behind an external device's token.
//! http = true
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
    /// MCP over HTTP.
    #[serde(default)]
    pub mcp: McpConfig,
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
}

/// The `[mcp]` table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    /// Serve the application's MCP tools at `/mcp` on the HTTP listener. Off by default.
    #[serde(default)]
    pub http: bool,
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
        assert!(!config.mcp.http);
        assert_eq!(config.http.listen, None);
    }

    #[test]
    fn the_file_turns_mcp_on_and_names_the_listener() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(SERVER_CONFIG_FILE);
        std::fs::write(
            &path,
            "[http]\nlisten = \"127.0.0.1:3173\"\nallowed_hosts = [\"box.example\"]\n\n[mcp]\nhttp = true\n",
        )
        .unwrap();
        let config = ServerConfig::load(&path).unwrap();
        assert!(config.mcp.http);
        assert_eq!(config.http.listen, Some("127.0.0.1:3173".parse().unwrap()));
        assert_eq!(config.http.allowed_hosts, vec!["box.example".to_owned()]);
    }

    #[test]
    fn a_misspelt_setting_is_an_error_not_a_default() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(SERVER_CONFIG_FILE);
        std::fs::write(&path, "[mcp]\nhtpp = true\n").unwrap();
        assert!(ServerConfig::load(&path).is_err());
    }
}
