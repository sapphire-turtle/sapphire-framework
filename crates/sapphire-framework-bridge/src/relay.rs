//! Relay configuration, merged from the host's `net.toml` and the workgroup's published one.
//!
//! Two files decide which relays a device uses, and neither overrides the other. The host's
//! `net.toml` is local preference; the workgroup's is what a self-hosted server announces to
//! its devices, synced with the rest of the workgroup root. They are **merged**: a device
//! keeps its own relays and gains the workgroup's, because a device that lost its own relay
//! when it joined a workgroup would be worse off than before. The public relays go off only
//! when both sides turn them off, so one side cannot silently strand the other.

use std::net::SocketAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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

// ── the embedded relay ─────────────────────────────────────────────────────

/// Whether this host runs a relay for its workgroup, and where.
///
/// Off by default — most hosts have neither a publicly reachable address nor a certificate —
/// and turned on by naming this section in `net.toml`:
///
/// ```toml
/// [embedded_relay]
/// bind = "0.0.0.0:80"
/// hostname = "relay.example.com"
///
/// [embedded_relay.tls]
/// cert_path = "/etc/letsencrypt/live/relay.example.com/fullchain.pem"
/// key_path = "/etc/letsencrypt/live/relay.example.com/privkey.pem"
/// https_bind = "0.0.0.0:443"
/// ```
///
/// The checks in [`EmbeddedRelay::start`] exist because each failure mode below is silent:
/// from the inside, a relay nobody can reach looks exactly like one that works.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct EmbeddedRelayConfig {
    /// The address the relay's HTTP services bind.
    ///
    /// Port `0` asks the operating system for a free port, which is what a test or a local
    /// run wants; the port actually bound is what [`EmbeddedRelay::url`] reports. On a real
    /// host this is normally port `80`, or, with TLS configured, the plain-HTTP socket that
    /// sits alongside the HTTPS one.
    ///
    /// This is also the address the start-up check inspects: on a real host it must not be a
    /// loopback address, because a relay nobody else can reach is worse than none — every
    /// device would try it and fail, with nothing in this host's log to say why.
    pub bind: SocketAddr,
    /// The name this relay is known by, and the name its certificate must be issued for.
    ///
    /// Required. It is reported verbatim by [`EmbeddedRelay::url`] rather than derived from
    /// `bind`: a relay is reached by name — that is what a device is told, and what a client
    /// verifies the certificate against — while the address it binds is a detail of this
    /// host. Confirming that the name resolves to `bind` would mean a DNS lookup at start-up,
    /// and would make `localhost` unusable in a test.
    pub hostname: String,
    /// The certificate to serve HTTPS with.
    ///
    /// Without one the relay serves plain HTTP from `bind`, which is what a relay behind a
    /// terminating proxy, or a local run, wants. With one the relay serves HTTPS as well, and
    /// `bind` keeps serving the plain HTTP the captive-portal probe needs.
    pub tls: TlsConfig,
    /// Allow `bind` to be a loopback address.
    ///
    /// Only for tests and local runs; see [`EmbeddedRelayConfig::bind`].
    pub allow_loopback: bool,
}

impl Default for EmbeddedRelayConfig {
    fn default() -> Self {
        EmbeddedRelayConfig {
            // Every interface, an ephemeral port: the address the default file would name is
            // the one that serves the world, and the port is chosen by the OS — which is
            // also what a test with a fixed port wants.
            bind: SocketAddr::from(([0, 0, 0, 0], 0)),
            hostname: String::new(),
            tls: TlsConfig::default(),
            allow_loopback: false,
        }
    }
}

/// Where a self-hosted relay's certificate and key are, and where it serves them.
///
/// Both files are PEM, the form every certificate authority hands out. The relay does not
/// generate a certificate: a relay serving one nobody trusts is a relay every device
/// rejects, so obtaining one is the operator's decision, not something to paper over.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct TlsConfig {
    /// The PEM certificate chain, leaf first.
    pub cert_path: Option<PathBuf>,
    /// The PEM private key for `cert_path`.
    ///
    /// Must be given whenever `cert_path` is, and never alone: half a certificate completes
    /// no handshake, and a relay that cannot complete one looks alive while serving nobody.
    pub key_path: Option<PathBuf>,
    /// The address the HTTPS server binds.
    ///
    /// Required once a certificate is given, and necessarily a different port from `bind`:
    /// the plain-HTTP captive-portal probe has to keep its own socket, so the two services
    /// cannot share one. Normally port `443`.
    pub https_bind: Option<SocketAddr>,
}

/// A relay server running inside this process, for this host's workgroup.
///
/// Start one with [`EmbeddedRelay::start`], hand [`EmbeddedRelay::url`] to the endpoints that
/// should use it — through [`RelayConfig::urls`], the same path every other relay URL takes —
/// and stop it with [`EmbeddedRelay::stop`].
#[cfg(feature = "embedded-relay")]
#[derive(Debug)]
pub struct EmbeddedRelay {
    server: iroh_relay::server::Server,
    url: String,
}

#[cfg(feature = "embedded-relay")]
impl EmbeddedRelay {
    /// Start a relay for `config`.
    ///
    /// Refuses a configuration that could only produce a relay nobody can use: a loopback
    /// bind address without [`EmbeddedRelayConfig::allow_loopback`], a missing hostname, or
    /// half a certificate. Each is reported with its reason, because the failure a
    /// misconfigured relay otherwise causes is invisible — devices try it and cannot connect,
    /// while the relay itself looks healthy.
    pub async fn start(config: &EmbeddedRelayConfig) -> Result<EmbeddedRelay> {
        validate(config)?;

        let mut relay = iroh_relay::server::RelayConfig::new(config.bind);
        let tls = match (&config.tls.cert_path, &config.tls.key_path) {
            (Some(cert_path), Some(key_path)) => {
                Some(server_tls_config(config, cert_path, key_path)?)
            }
            // The other combinations are refused by `validate`.
            _ => None,
        };
        let secure = tls.is_some();
        relay.tls = tls;

        let mut server_config = iroh_relay::server::ServerConfig::default();
        server_config.relay = Some(relay);
        let server = iroh_relay::server::Server::spawn(server_config)
            .await
            .map_err(|e| Error::Config(format!("the embedded relay could not start: {e}")))?;

        // With TLS the URL names the hostname, because that is what a client verifies the
        // certificate against. Without it there is no name to verify, so the URL names the
        // address actually bound: a name that resolved elsewhere would send devices nowhere,
        // and there is nothing here to check that it does not.
        let url = if secure {
            let https = server.https_addr().ok_or_else(|| {
                Error::Config("the embedded relay bound no HTTPS address".to_owned())
            })?;
            if https.port() == 443 {
                format!("https://{}", config.hostname)
            } else {
                format!("https://{}:{}", config.hostname, https.port())
            }
        } else {
            let http = server.http_addr().ok_or_else(|| {
                Error::Config("the embedded relay bound no HTTP address".to_owned())
            })?;
            format!("http://{http}")
        };

        Ok(EmbeddedRelay { server, url })
    }

    /// The URL other devices should be given for this relay.
    ///
    /// Feed it into [`RelayConfig::urls`] like any other relay URL: the endpoints cannot tell
    /// an embedded relay from a remote one, and there is no second way to configure a relay.
    pub fn url(&self) -> String {
        self.url.clone()
    }

    /// Stop the relay and release its sockets.
    pub async fn stop(self) {
        if let Err(err) = self.server.shutdown().await {
            // The relay is going away either way and the caller has already decided to stop
            // it, so failing here would only report a shutdown it cannot act on.
            tracing::warn!("the embedded relay did not shut down cleanly: {err}");
        }
    }
}

/// Check `config` before anything is bound.
///
/// Every check here is about a relay that would run and never be used, which is the failure
/// worth an error message: from the inside, a relay nobody can reach is indistinguishable
/// from one that works.
#[cfg(feature = "embedded-relay")]
fn validate(config: &EmbeddedRelayConfig) -> Result<()> {
    if config.hostname.trim().is_empty() {
        return Err(Error::Config(
            "an embedded relay needs a hostname: it is the name devices are given, so without \
             one the relay cannot be reachable"
                .to_owned(),
        ));
    }
    if config.bind.ip().is_loopback() && !config.allow_loopback {
        return Err(Error::Config(format!(
            "the embedded relay's address {} is a loopback address, so the relay is not \
             reachable by any other device; set allow_loopback to run a relay for tests",
            config.bind.ip()
        )));
    }
    match (
        &config.tls.cert_path,
        &config.tls.key_path,
        config.tls.https_bind,
    ) {
        (None, None, _) => Ok(()),
        (Some(_), Some(_), Some(_)) => Ok(()),
        (Some(_), Some(_), None) => Err(Error::Config(
            "an embedded relay with a certificate needs tls.https_bind: the HTTPS server \
             cannot share the plain-HTTP socket"
                .to_owned(),
        )),
        (Some(_), None, _) | (None, Some(_), _) => Err(Error::Config(
            "an embedded relay's certificate needs both tls.cert_path and tls.key_path: a \
             certificate without its key completes no handshake, and a key without its \
             certificate has nothing to prove"
                .to_owned(),
        )),
    }
}

/// The rustls server configuration for `cert_path` and `key_path`.
///
/// The crypto provider is iroh-relay's own, so the relay serves TLS with the provider the
/// rest of the iroh stack uses rather than one this crate picked independently.
#[cfg(feature = "embedded-relay")]
fn server_tls_config(
    config: &EmbeddedRelayConfig,
    cert_path: &PathBuf,
    key_path: &PathBuf,
) -> Result<iroh_relay::server::TlsConfig> {
    use rustls::pki_types::pem::PemObject;

    let certs = rustls::pki_types::CertificateDer::pem_file_iter(cert_path)
        .map_err(|e| Error::Config(format!("{}: {e}", cert_path.display())))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Config(format!("{}: {e}", cert_path.display())))?;
    let key = rustls::pki_types::PrivateKeyDer::from_pem_file(key_path)
        .map_err(|e| Error::Config(format!("{}: {e}", key_path.display())))?;

    let server_config =
        rustls::ServerConfig::builder_with_provider(iroh_relay::tls::default_provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| Error::Config(format!("the relay's TLS versions are unusable: {e}")))?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| Error::Config(format!("the relay's certificate is unusable: {e}")))?;

    let https_bind = config
        .tls
        .https_bind
        .ok_or_else(|| Error::Config("an embedded relay needs tls.https_bind".to_owned()))?;
    Ok(iroh_relay::server::TlsConfig::new(
        https_bind,
        iroh_relay::server::CertConfig::Manual { server_config },
    ))
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

#[cfg(test)]
mod embedded_relay_tests {
    use super::*;

    #[test]
    fn no_embedded_relay_is_the_default() {
        // A relay must be asked for, never inherited: it is off unless `net.toml` names it.
        assert!(NetConfig::default().embedded_relay.is_none());
    }

    #[test]
    fn an_embedded_relay_round_trips_through_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("net.toml");
        std::fs::write(
            &path,
            "relays = [\"https://relay.example/\"]\n\
             [embedded_relay]\n\
             bind = \"0.0.0.0:80\"\n\
             hostname = \"relay.example.com\"\n",
        )
        .unwrap();

        let net = NetConfig::load(&path).unwrap();
        let relay = net.embedded_relay.expect("the relay was configured");
        assert_eq!(relay.bind, "0.0.0.0:80".parse().unwrap());
        assert_eq!(relay.hostname, "relay.example.com");
        assert!(!relay.allow_loopback, "a real host does not allow loopback");
        assert!(relay.tls.cert_path.is_none());
        assert!(relay.tls.https_bind.is_none());
    }

    #[test]
    fn a_certificate_and_key_round_trip_through_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("net.toml");
        std::fs::write(
            &path,
            "[embedded_relay]\n\
             bind = \"0.0.0.0:80\"\n\
             hostname = \"relay.example.com\"\n\
             [embedded_relay.tls]\n\
             cert_path = \"/etc/relay/cert.pem\"\n\
             key_path = \"/etc/relay/key.pem\"\n\
             https_bind = \"0.0.0.0:443\"\n",
        )
        .unwrap();

        let net = NetConfig::load(&path).unwrap();
        let tls = net.embedded_relay.unwrap().tls;
        assert_eq!(
            tls.cert_path.unwrap().to_str().unwrap(),
            "/etc/relay/cert.pem"
        );
        assert_eq!(
            tls.key_path.unwrap().to_str().unwrap(),
            "/etc/relay/key.pem"
        );
        assert_eq!(tls.https_bind.unwrap(), "0.0.0.0:443".parse().unwrap());
    }

    /// A `net.toml` with no relay must survive being written back out, which is what
    /// `Workgroup::publish_net` does when a workgroup announces its configuration. TOML has
    /// no way to spell `None`, so an unset relay has to be left out of the output.
    #[test]
    fn an_unset_relay_is_left_out_of_the_written_file() {
        let text = toml::to_string_pretty(&NetConfig::default()).unwrap();
        assert!(!text.contains("embedded_relay"), "{text}");
        let back: NetConfig = toml::from_str(&text).unwrap();
        assert!(back.embedded_relay.is_none());
    }

    #[test]
    fn a_configured_relay_survives_being_written_back_out() {
        let net = NetConfig {
            embedded_relay: Some(EmbeddedRelayConfig {
                bind: "0.0.0.0:80".parse().unwrap(),
                hostname: "relay.example.com".to_owned(),
                ..EmbeddedRelayConfig::default()
            }),
            ..NetConfig::default()
        };
        let text = toml::to_string_pretty(&net).unwrap();
        let back: NetConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.embedded_relay, net.embedded_relay);
    }
}
