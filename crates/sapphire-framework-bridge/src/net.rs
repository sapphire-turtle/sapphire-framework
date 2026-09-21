//! Host-local network configuration (`net.toml`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

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
}

impl Default for NetConfig {
    fn default() -> Self {
        NetConfig {
            wake_on_sync: true,
            discovery: true,
            relays: Vec::new(),
            use_default_relays: true,
        }
    }
}

impl NetConfig {
    /// Read the file, or the defaults if it is not there.
    ///
    /// Every field is optional in the file: a `net.toml` that sets only `wake_on_sync`
    /// leaves discovery on, the relay list empty, and the public relays on.
    pub fn load(path: &Path) -> Result<NetConfig> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(NetConfig::default()),
            Err(e) => Err(Error::Io(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_the_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let net = NetConfig::load(&tmp.path().join("net.toml")).unwrap();
        assert!(net.wake_on_sync);
        assert!(net.discovery);
        assert!(net.relays.is_empty());
        assert!(net.use_default_relays);
    }

    #[test]
    fn a_partial_file_keeps_the_other_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("net.toml");
        std::fs::write(&path, "wake_on_sync = false\ndiscovery = false\n").unwrap();

        let net = NetConfig::load(&path).unwrap();
        assert!(!net.wake_on_sync);
        assert!(!net.discovery);
        assert!(net.relays.is_empty());
        assert!(net.use_default_relays);
    }

    #[test]
    fn relays_round_trip_through_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("net.toml");
        std::fs::write(
            &path,
            "wake_on_sync = false\ndiscovery = false\nrelays = [\"https://relay.example/\"]\n\
             use_default_relays = false\n",
        )
        .unwrap();

        let net = NetConfig::load(&path).unwrap();
        assert_eq!(net.relays, vec!["https://relay.example/".to_owned()]);
        assert!(!net.use_default_relays);
    }

    #[test]
    fn an_unreadable_file_names_itself_in_the_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("net.toml");
        std::fs::write(&path, "wake_on_sync = \"yes\"\n").unwrap();

        let err = NetConfig::load(&path).unwrap_err();
        assert!(err.to_string().contains("net.toml"), "{err}");
    }
}
