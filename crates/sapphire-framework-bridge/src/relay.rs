//! Relay configuration, merged from the host's `net.toml` and the workgroup's published one.
//!
//! Two files decide which relays a device uses, and neither overrides the other. The host's
//! `net.toml` is local preference; the workgroup's is what a self-hosted server announces to
//! its devices, synced with the rest of the workgroup root. They are **merged**: a device
//! keeps its own relays and gains the workgroup's, because a device that lost its own relay
//! when it joined a workgroup would be worse off than before. The public relays go off only
//! when both sides turn them off, so one side cannot silently strand the other.

use crate::error::{Error, Result};
use crate::net::NetConfig;
use crate::workgroup::Workgroup;

/// The relays this host's endpoint should use.
///
/// The endpoint reads this through [`relay_mode`](crate::iroh::relay_mode) — which lives
/// behind the `node` feature — so this type stays free of iroh and a build without the
/// transport can still compute and validate its configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelayConfig {
    /// The custom relay URLs, deduplicated: the host's own first, then the workgroup's.
    pub urls: Vec<String>,
    /// Whether the public relays stay on in addition to `urls`.
    pub use_default: bool,
}

/// Merge the host's relay configuration with the workgroup's published one.
///
/// The host's `net.toml` is local preference; the workgroup's is what a self-hosted server
/// announces. A device without a workgroup, or whose workgroup has published nothing yet,
/// simply uses its own configuration — the defaults keep the public relays on.
///
/// Every URL on either side is validated here rather than where the endpoint is built, so a
/// typo in either file is a configuration error at startup, not a silent lack of
/// connectivity after it.
pub fn relays(host: &NetConfig, workgroup: Option<&Workgroup>) -> Result<RelayConfig> {
    let announced = match workgroup {
        Some(workgroup) => workgroup.published_net()?,
        None => NetConfig::default(),
    };
    for url in host.relays.iter().chain(&announced.relays) {
        if let Err(err) = url::Url::parse(url) {
            return Err(Error::Config(format!("{url}: not a relay URL: {err}")));
        }
    }
    let mut urls: Vec<String> = Vec::with_capacity(host.relays.len() + announced.relays.len());
    for url in host.relays.iter().chain(&announced.relays) {
        if !urls.contains(url) {
            urls.push(url.clone());
        }
    }
    Ok(RelayConfig {
        urls,
        // One side alone must not be able to strand the other: the public relays go off
        // only when both sides turned them off.
        use_default: host.use_default_relays || announced.use_default_relays,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::BridgeDir;

    const NODE_A: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

    fn host_with(urls: &[&str], use_default: bool) -> NetConfig {
        NetConfig {
            relays: urls.iter().map(|u| (*u).to_owned()).collect(),
            use_default_relays: use_default,
            ..NetConfig::default()
        }
    }

    /// A workgroup whose root publishes `net` under `root/net.toml`.
    ///
    /// The returned temporaries must stay alive for the workgroup to stay on disk.
    fn workgroup_with_net(net: &NetConfig) -> (tempfile::TempDir, BridgeDir, Workgroup) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        wg.publish_net(net).unwrap();
        (tmp, dir, wg)
    }

    #[test]
    fn with_no_configuration_the_public_relays_are_used() {
        let config = relays(&NetConfig::default(), None).unwrap();
        assert!(config.use_default);
        assert!(config.urls.is_empty());
    }

    #[test]
    fn a_hosts_own_relay_is_used() {
        let config = relays(&host_with(&["https://relay.example"], true), None).unwrap();
        assert_eq!(config.urls, vec!["https://relay.example".to_owned()]);
        assert!(config.use_default);
    }

    #[test]
    fn the_two_sources_are_merged_not_overridden() {
        let (_tmp, _dir, wg) = workgroup_with_net(&host_with(&["https://group.example"], true));
        let config = relays(&host_with(&["https://mine.example"], true), Some(&wg)).unwrap();

        let mut urls = config.urls.clone();
        urls.sort();
        assert_eq!(
            urls,
            vec![
                "https://group.example".to_owned(),
                "https://mine.example".to_owned()
            ],
            "a device must not lose its own relay by joining a workgroup"
        );
    }

    #[test]
    fn a_duplicate_relay_appears_once() {
        let (_tmp, _dir, wg) = workgroup_with_net(&host_with(&["https://same.example"], true));
        let config = relays(&host_with(&["https://same.example"], true), Some(&wg)).unwrap();
        assert_eq!(config.urls.len(), 1);
    }

    #[test]
    fn the_public_relays_go_off_only_when_both_sides_say_so() {
        let (_tmp, _dir, wg) = workgroup_with_net(&host_with(&[], false));

        // Host still wants them.
        assert!(
            relays(&host_with(&[], true), Some(&wg))
                .unwrap()
                .use_default
        );
        // Both say no.
        assert!(
            !relays(&host_with(&[], false), Some(&wg))
                .unwrap()
                .use_default
        );
    }

    #[test]
    fn a_workgroup_without_a_published_net_file_contributes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let wg = Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        let config = relays(&host_with(&["https://mine.example"], true), Some(&wg)).unwrap();
        assert_eq!(config.urls, vec!["https://mine.example".to_owned()]);
    }

    #[test]
    fn a_relay_url_that_is_not_a_url_is_refused() {
        let err = relays(&host_with(&["not a url"], true), None).unwrap_err();
        assert!(err.to_string().contains("not a url"), "{err}");
    }

    #[test]
    fn a_bad_url_published_by_the_workgroup_is_refused_too() {
        let (_tmp, _dir, wg) = workgroup_with_net(&host_with(&["not a url either"], true));

        let err = relays(&host_with(&[], true), Some(&wg)).unwrap_err();
        assert!(err.to_string().contains("not a relay URL"), "{err}");
        assert!(err.to_string().contains("not a url either"), "{err}");
    }

    #[test]
    fn a_good_url_on_both_sides_is_accepted() {
        let (_tmp, _dir, wg) = workgroup_with_net(&host_with(&["https://group.example"], true));
        let config = relays(&host_with(&["https://group.example"], true), Some(&wg)).unwrap();
        assert_eq!(config.urls, vec!["https://group.example".to_owned()]);
        assert!(config.use_default);
    }
}
