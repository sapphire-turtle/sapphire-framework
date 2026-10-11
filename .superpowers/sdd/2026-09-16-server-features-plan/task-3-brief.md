### Task 3: Relay configuration

**Files:**
- Create: `crates/sapphire-framework-bridge/src/relay.rs`
- Modify: `crates/sapphire-framework-bridge/src/{net.rs,iroh.rs,workgroup.rs}`
- Test: inline `#[cfg(test)] mod tests` in `relay.rs`

**Interfaces:**
- Produces:
  - `RelayConfig { urls: Vec<String>, use_default: bool }`
  - `fn relays(host: &NetConfig, workgroup: Option<&Workgroup>) -> Result<RelayConfig>` —
    merges the host's `net.toml` with the workgroup's published one
  - `Workgroup::published_net(&self) -> Result<NetConfig>` — reads `root/net.toml`
  - `Workgroup::publish_net(&self, net: &NetConfig) -> Result<()>`

**Two files, and which wins:** the host's `net.toml` is local preference; the workgroup's is
what a self-hosted server announces to its devices. They are **merged, not overridden** — a
device keeps its own relays and gains the workgroup's — because a device that lost its own
relay when it joined a workgroup would be worse off than before. `use_default` is `false` only
if **both** turn the public relays off, so one side cannot silently strand the other.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::NetConfig;

    fn host_with(urls: &[&str], use_default: bool) -> NetConfig {
        NetConfig {
            relays: urls.iter().map(|u| (*u).to_owned()).collect(),
            use_default_relays: use_default,
            ..NetConfig::default()
        }
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
            vec!["https://group.example".to_owned(), "https://mine.example".to_owned()],
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
        assert!(relays(&host_with(&[], true), Some(&wg)).unwrap().use_default);
        // Both say no.
        assert!(!relays(&host_with(&[], false), Some(&wg)).unwrap().use_default);
    }

    #[test]
    fn a_workgroup_without_a_published_net_file_contributes_nothing() {
        let (_tmp, dir) = crate::dir::tests_bridge_dir();
        let wg = crate::workgroup::Workgroup::create(&dir, "home", "laptop", NODE_A).unwrap();
        let config = relays(&host_with(&["https://mine.example"], true), Some(&wg)).unwrap();
        assert_eq!(config.urls, vec!["https://mine.example".to_owned()]);
    }

    #[test]
    fn a_relay_url_that_is_not_a_url_is_refused() {
        let err = relays(&host_with(&["not a url"], true), None).unwrap_err();
        assert!(err.to_string().contains("not a url"), "{err}");
    }
}
```

Write `workgroup_with_net` as a helper that creates a workgroup and calls `publish_net`.

- [ ] **Step 2–5: Implement, verify, commit**

`NetConfig` gains `use_default_relays: bool` (default `true`). `iroh.rs` feeds `RelayConfig`
into the endpoint builder. `relays` validates each URL with `url::Url::parse` — add `url = "2"`
— so a typo is caught at startup rather than as a silent lack of connectivity.

```bash
cargo test -p sapphire-framework-bridge --all-features relay
git commit -m "feat(bridge): merge relay configuration from the host and the workgroup"
```

---

