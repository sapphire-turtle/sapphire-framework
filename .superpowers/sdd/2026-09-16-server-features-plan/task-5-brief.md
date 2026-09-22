### Task 5: `status.json`

**Files:**
- Create: `crates/sapphire-framework-bridge/src/status.rs`
- Modify: `crates/sapphire-framework-bridge/src/{lib.rs,command.rs}`
- Test: inline `#[cfg(test)] mod tests` in `status.rs`

**Interfaces:**
- Produces:
  - `StatusFile { version, pid, started_at, node_id, workgroup: Option<WorkgroupStatus>, peers: Vec<PeerStatus>, routes: Vec<RouteStatus>, relays: Vec<String> }`
  - `PeerStatus { device_id, name, node_id, connected, last_seen: Option<DateTime<Utc>>, last_error: Option<String> }`
  - `StatusWriter::start(path: PathBuf, source: Arc<dyn StatusSource>) -> StatusWriter`
  - `STATUS_INTERVAL: Duration = 5s`

**Written atomically**, through a temporary file and a rename. A reader that caught a
half-written `status.json` would report nonsense at exactly the moment someone is trying to
find out what is wrong.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(StatusFile);

    impl StatusSource for Fixed {
        fn snapshot(&self) -> StatusFile {
            self.0.clone()
        }
    }

    fn sample() -> StatusFile {
        StatusFile {
            version: "0.0.0".into(),
            pid: std::process::id(),
            started_at: chrono::Utc::now(),
            node_id: "abc".into(),
            workgroup: None,
            peers: vec![],
            routes: vec![],
            relays: vec![],
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_file_appears_promptly_and_parses() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("status.json");
        let writer = StatusWriter::start(path.clone(), Arc::new(Fixed(sample())));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !path.exists() {
            assert!(std::time::Instant::now() < deadline, "no status.json was written");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let parsed: StatusFile = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.node_id, "abc");
        writer.stop();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_reader_never_sees_a_half_written_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("status.json");
        let mut big = sample();
        big.peers = (0..2000)
            .map(|n| PeerStatus {
                device_id: grain_id::GrainId::random(),
                name: format!("device-{n}"),
                node_id: "x".repeat(64),
                connected: true,
                last_seen: Some(chrono::Utc::now()),
                last_error: None,
            })
            .collect();
        let writer = StatusWriter::start(path.clone(), Arc::new(Fixed(big)));

        // Read repeatedly while it is being rewritten; every read must parse.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut reads = 0;
        while std::time::Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&path) {
                serde_json::from_str::<StatusFile>(&text)
                    .expect("a partially written status.json reached a reader");
                reads += 1;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(reads > 10, "the test did not actually read anything");
        writer.stop();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stopping_the_writer_leaves_the_last_snapshot_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("status.json");
        let writer = StatusWriter::start(path.clone(), Arc::new(Fixed(sample())));
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        writer.stop();
        assert!(path.exists(), "the last status is useful after a clean stop");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_temporary_files_are_left_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("status.json");
        let writer = StatusWriter::start(path.clone(), Arc::new(Fixed(sample())));
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        writer.stop();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let leftovers: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n != "status.json")
            .collect();
        assert!(leftovers.is_empty(), "left behind {leftovers:?}");
    }
}
```

- [ ] **Step 2–5: Implement, verify, commit**

`bridge status` prefers a live control-plane call and falls back to reading `status.json` when
no bridge answers — which is how you find out what the bridge was doing before it stopped.

```bash
cargo test -p sapphire-framework-bridge --all-features status
git commit -m "feat(bridge): publish status.json atomically"
```

---

