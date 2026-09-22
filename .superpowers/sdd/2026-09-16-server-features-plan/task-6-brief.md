### Task 6: The log, and `bridge log`

**Files:**
- Create: `crates/sapphire-framework-bridge/src/logging.rs`
- Modify: `crates/sapphire-framework-bridge/src/command.rs`, `apps/sapphire-bridge/src/main.rs`
- Test: inline `#[cfg(test)] mod tests` in `logging.rs`

**Interfaces:**
- Produces:
  - `LOG_FILE: &str = "node.log"`, `LOG_MAX_BYTES: u64 = 10 * 1024 * 1024`, `LOG_KEEP: usize = 3`
  - `fn install(dir: &BridgeDir) -> Result<LogGuard>` — a `tracing` layer writing the bridge's
    own targets to `<bridge dir>/logs/node.log`, in addition to whatever the process already
    prints
  - `BridgeCommand::Log { follow: bool, lines: usize }`

This is the `bridge log` the bridge plan deferred. One writer — the single-instance lock
guarantees it — so the file is a continuous record across restarts rather than an interleaving.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::BridgeDir;

    #[test]
    fn installing_creates_the_log_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let guard = install(&dir).unwrap();
        tracing::info!(target: "sapphire_framework_bridge", "hello from the test");
        drop(guard);

        let text = std::fs::read_to_string(dir.log_dir().join(LOG_FILE)).unwrap();
        assert!(text.contains("hello from the test"), "{text}");
    }

    #[test]
    fn the_log_rotates_at_its_size_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        let guard = install_with_limit(&dir, 4096).unwrap();
        for n in 0..2000 {
            tracing::info!(target: "sapphire_framework_bridge", "line {n} padded {}", "x".repeat(64));
        }
        drop(guard);

        let files: Vec<String> = std::fs::read_dir(dir.log_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(files.len() > 1, "the log never rotated: {files:?}");
        assert!(files.len() <= LOG_KEEP + 1, "too many kept: {files:?}");
    }

    #[test]
    fn a_restart_continues_the_same_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();

        let guard = install(&dir).unwrap();
        tracing::info!(target: "sapphire_framework_bridge", "first run");
        drop(guard);

        let guard = install(&dir).unwrap();
        tracing::info!(target: "sapphire_framework_bridge", "second run");
        drop(guard);

        let text = std::fs::read_to_string(dir.log_dir().join(LOG_FILE)).unwrap();
        assert!(text.contains("first run") && text.contains("second run"), "{text}");
    }

    #[test]
    fn reading_the_tail_of_a_missing_log_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        assert!(tail(&dir, 20).unwrap().is_empty());
    }

    #[test]
    fn the_tail_returns_the_last_lines_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = BridgeDir::at(tmp.path().join("bridge")).unwrap();
        std::fs::create_dir_all(dir.log_dir()).unwrap();
        std::fs::write(
            dir.log_dir().join(LOG_FILE),
            (0..100).map(|n| format!("line {n}\n")).collect::<String>(),
        )
        .unwrap();

        let lines = tail(&dir, 3).unwrap();
        assert_eq!(lines, vec!["line 97", "line 98", "line 99"]);
    }
}
```

`install_with_limit` is `install` with the rotation threshold as a parameter; make `install`
call it with `LOG_MAX_BYTES` so the test exercises the real path.

- [ ] **Step 2–5: Implement, verify, commit**

Add `tracing-appender = "0.2"`. `bridge log` prints the tail, and with `--follow` keeps
printing as the file grows.

```bash
cargo test -p sapphire-framework-bridge --all-features logging
git commit -m "feat(bridge): write a log any process can read, and add bridge log"
```

---

