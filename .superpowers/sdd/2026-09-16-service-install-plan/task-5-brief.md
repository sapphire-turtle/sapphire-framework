### Task 5: `post_install` and privilege separation

**Files:**
- Modify: `crates/sapphire-framework-service/src/{lib.rs,manager.rs}`
- Test: inline tests in `manager.rs`

**Interfaces:**
- `post_install` runs **after** activation, with the resolved `InstallContext`
- Files it writes into another user's directories are **chowned to that user**
- `InstallArgs::keep_helper` skips it

**What uses it:** an app that wants to leave something in the target user's configuration. The
sync spec's example — `sapphire-sync` writing `embedded_node = false` — is gone with
`embedded_node` itself, but the hook is not: `sapphire-agent` uses it to write the app's
`run_as` and `helper_as` into its config file so the server reads the same values the unit
names.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn post_install_runs_after_activation() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&order);
    let mut spec = spec();
    spec.post_install = Some(Box::new(move |_| {
        recorded.lock().unwrap().push("post_install");
        Ok(())
    }));

    let manager = RecordingManager::ordered(Arc::clone(&order));
    install(&spec, &InstallArgs::default(), &linux_user(), &manager).unwrap();

    let order = order.lock().unwrap().clone();
    assert_eq!(order.last().map(String::as_str), Some("post_install"), "{order:?}");
}

#[test]
fn post_install_sees_the_resolved_target_user() {
    let seen = Arc::new(Mutex::new(None));
    let recorded = Arc::clone(&seen);
    let mut spec = spec();
    spec.post_install = Some(Box::new(move |ctx| {
        *recorded.lock().unwrap() = ctx.target_user.clone();
        Ok(())
    }));

    install(&spec, &InstallArgs::default(), &linux_root(), &RecordingManager::default()).unwrap();
    assert_eq!(seen.lock().unwrap().as_deref(), Some("alice"));
}

#[test]
fn keep_helper_skips_post_install() {
    let ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ran);
    let mut spec = spec();
    spec.post_install = Some(Box::new(move |_| {
        flag.store(true, Ordering::Relaxed);
        Ok(())
    }));

    let args = InstallArgs { keep_helper: true, ..InstallArgs::default() };
    install(&spec, &args, &linux_user(), &RecordingManager::default()).unwrap();
    assert!(!ran.load(Ordering::Relaxed));
}

#[test]
fn a_failing_post_install_fails_the_install_and_says_what_was_done() {
    let mut spec = spec();
    spec.post_install = Some(Box::new(|_| Err(Error::Config("no room".into()))));

    let err =
        install(&spec, &InstallArgs::default(), &linux_user(), &RecordingManager::default())
            .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("no room"), "{message}");
    assert!(
        message.contains("installed"),
        "the service is installed and running; say so rather than leaving it ambiguous: {message}"
    );
}

#[test]
fn a_privilege_separated_spec_installs_as_a_root_unit_whatever_run_as_says() {
    let mut spec = spec();
    spec.system_run_as = RunAs::InvokingUser;
    spec.privileges = Some(privileges_for("alice", "sapphire-agent-tools"));

    let manager = RecordingManager::default();
    install(&spec, &InstallArgs::default(), &linux_root(), &manager).unwrap();

    let body = &manager.calls().units[0].1;
    assert!(
        !body.contains("\nUser="),
        "an app that drops privileges itself must start as root: {body}"
    );
}
```

The last one closes a hole: a spec that asks for both `RunAs::InvokingUser` and privilege
separation is contradictory, and silently honouring `User=` would produce a service that
cannot do what it was configured to do. Privilege separation wins, and the doc comment says so.

- [ ] **Step 2–5: Implement, verify, commit**

```bash
cargo test -p sapphire-framework-service
git commit -m "feat(service): run post_install, and keep privilege separation coherent"
```

---

