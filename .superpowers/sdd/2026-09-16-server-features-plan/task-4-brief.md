### Task 4: The embedded relay

**Files:**
- Modify: `crates/sapphire-framework-bridge/src/relay.rs`, `Cargo.toml`
- Modify: `apps/sapphire-bridge/Cargo.toml`
- Test: `crates/sapphire-framework-bridge/tests/embedded_relay.rs`

**Interfaces (feature `embedded-relay`):**
- `EmbeddedRelay::start(config: &EmbeddedRelayConfig) -> Result<EmbeddedRelay>`
- `EmbeddedRelayConfig { bind: SocketAddr, hostname: String, tls: TlsConfig }`
- `EmbeddedRelay::url(&self) -> String`, `EmbeddedRelay::stop(self)`
- `NetConfig` gains `embedded_relay: Option<EmbeddedRelayConfig>`

**Off by default, and the error says why.** A relay needs a publicly reachable address and a
certificate. Turning it on without either produces a relay nobody can use, so the start-up
check is explicit: the address must not be loopback unless `allow_loopback` is set (which the
tests do), and a hostname is required.

**Read the API before writing this**: the embedded relay is iroh's, and its surface is not
pinned by anything in this repository. Check `https://docs.rs/iroh-relay` for the current
server type and its configuration, and shape the code to that rather than to the sketch here.

- [ ] **Step 1: Write the tests**

```rust
#![cfg(feature = "embedded-relay")]

use sapphire_framework_bridge::{EmbeddedRelay, EmbeddedRelayConfig};

fn loopback(port: u16) -> EmbeddedRelayConfig {
    EmbeddedRelayConfig {
        bind: format!("127.0.0.1:{port}").parse().unwrap(),
        hostname: "localhost".into(),
        allow_loopback: true,
        ..EmbeddedRelayConfig::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_starts_and_reports_its_url() {
    let relay = EmbeddedRelay::start(&loopback(0)).await.unwrap();
    assert!(relay.url().starts_with("http"), "{}", relay.url());
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loopback_address_is_refused_unless_it_is_asked_for() {
    let mut config = loopback(0);
    config.allow_loopback = false;
    let err = EmbeddedRelay::start(&config).await.unwrap_err();
    assert!(
        err.to_string().contains("reachable"),
        "the message must say what is wrong: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_hostname_is_refused() {
    let mut config = loopback(0);
    config.hostname = String::new();
    assert!(EmbeddedRelay::start(&config).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn two_endpoints_meet_through_the_embedded_relay() {
    // Direct connections off, so the relay is the only path. This is the test that says the
    // feature works rather than merely starts.
    let relay = EmbeddedRelay::start(&loopback(0)).await.unwrap();
    // … build two IrohTransports configured with only this relay and with direct
    // addresses suppressed, then open a stream between them …
    relay.stop().await;
}
```

Write the last one against whatever iroh 1.2 offers for suppressing direct paths; if it offers
nothing, mark it `#[ignore]` with a doc comment saying so rather than deleting it — an
untested relay is worth knowing about.

- [ ] **Step 2–4: Implement, verify, commit**

```bash
cargo test -p sapphire-framework-bridge --features embedded-relay --test embedded_relay
git commit -m "feat(bridge): optionally run a relay for a workgroup"
```

---

