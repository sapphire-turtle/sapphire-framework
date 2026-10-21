//! Host-local network configuration (`[net]` of the bridge's `config.toml`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// What this host does on the network.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct NetConfig {
    /// Start a stopped app server when a peer asks for one of its workspaces.
    pub wake_on_sync: bool,
    /// Use discovery services to find peers.
    pub discovery: bool,
    /// Relay URLs to use, in addition to the public ones while `use_default_relays` is on.
    pub relays: Vec<String>,
    /// Keep the public relays on alongside `relays`.
    ///
    /// Off only when this host wants the public relays gone entirely — and, once a
    /// workgroup is joined, not even then on its own: [`relays`](crate::relays) turns
    /// them off only when both sides say so.
    pub use_default_relays: bool,
    /// Run a relay for this workgroup from inside this process.
    ///
    /// Off unless `[net]` names it: a relay needs a publicly reachable address and a
    /// certificate, and most hosts have neither. The field is always present, so a `[net]`
    /// that names a relay still parses on a build without the `embedded-relay` feature — such
    /// a build simply starts none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedded_relay: Option<crate::relay::EmbeddedRelayConfig>,
}

impl Default for NetConfig {
    fn default() -> Self {
        NetConfig {
            wake_on_sync: true,
            discovery: true,
            relays: Vec::new(),
            use_default_relays: true,
            embedded_relay: None,
        }
    }
}

impl NetConfig {
    /// Read `[net]` of the device's `config.toml` at `path`, or the defaults if the file
    /// or the table is not there.
    ///
    /// Every field is optional: a `[net]` that sets only `wake_on_sync` leaves discovery on,
    /// the relay list empty, and the public relays on. A file that does not parse — in any
    /// table — is an error naming it, so a typo is reported when the bridge starts.
    pub fn load(path: &Path) -> Result<NetConfig> {
        Ok(crate::config_file::read_device(path)?.net)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_the_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let net = NetConfig::load(&tmp.path().join("config.toml")).unwrap();
        assert!(net.wake_on_sync);
        assert!(net.discovery);
        assert!(net.relays.is_empty());
        assert!(net.use_default_relays);
    }

    #[test]
    fn net_config_reads_the_net_table() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[net]\nwake_on_sync = false\n").unwrap();

        let net = NetConfig::load(&path).unwrap();
        assert!(!net.wake_on_sync);
        assert!(net.discovery);
        assert!(net.relays.is_empty());
        assert!(net.use_default_relays);
    }

    #[test]
    fn the_embed_table_beside_it_does_not_matter() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            "[net]\ndiscovery = false\n[embed.local]\nenabled = false\n",
        )
        .unwrap();
        assert!(!NetConfig::load(&path).unwrap().discovery);
    }

    #[test]
    fn relays_round_trip_through_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            "[net]\nwake_on_sync = false\ndiscovery = false\n\
             relays = [\"https://relay.example/\"]\nuse_default_relays = false\n",
        )
        .unwrap();

        let net = NetConfig::load(&path).unwrap();
        assert_eq!(net.relays, vec!["https://relay.example/".to_owned()]);
        assert!(!net.use_default_relays);
    }

    #[test]
    fn an_unreadable_file_names_itself_in_the_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "[net]\nwake_on_sync = \"yes\"\n").unwrap();

        let err = NetConfig::load(&path).unwrap_err();
        assert!(err.to_string().contains("config.toml"), "{err}");
    }
}
