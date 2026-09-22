### Task 6: Wiring it into the apps

**Files:**
- Modify: `crates/sapphire-framework-server/src/command.rs`
- Modify: `crates/sapphire-framework-bridge/src/command.rs`
- Modify: `apps/sapphire-bridge/{Cargo.toml,src/main.rs}`
- Modify: `docs/ARCHITECTURE.md`
- Test: inline tests in both `command.rs` files

**Interfaces:**
- `ServerCommand::Service(ServiceCommand)` — the `service install | uninstall | status` the
  app-server plan left for this step
- `BridgeCommand::Service(ServiceCommand)`
- `AppServer::service_spec(&self) -> ServiceSpec` — so an app does not assemble one by hand

- [ ] **Step 1: Write the failing tests**

```rust
// In -server:
#[test]
fn the_service_subcommands_parse() {
    for args in [
        vec!["app", "service", "install"],
        vec!["app", "service", "install", "--system"],
        vec!["app", "service", "install", "--run-as", "alice"],
        vec!["app", "service", "uninstall"],
        vec!["app", "service", "status"],
    ] {
        assert!(Probe::try_parse_from(&args).is_ok(), "{args:?}");
    }
}

#[test]
fn the_generated_spec_runs_the_server_not_the_cli() {
    let spec = AppServer::new(&CTX, "0.0.0").service_spec();
    assert_eq!(spec.args, vec!["server".to_owned(), "run".to_owned()]);
}

#[test]
fn the_generated_spec_carries_the_apps_privileges() {
    let privileges = privileges_for("alice", "tools");
    let spec = AppServer::new(&CTX, "0.0.0")
        .privileges(privileges.clone())
        .service_spec();
    assert!(spec.privileges.is_some());
}

// In -bridge:
#[test]
fn the_bridge_service_spec_runs_the_bridge() {
    let spec = bridge_service_spec("0.0.0");
    assert_eq!(spec.args, vec!["run".to_owned()]);
    assert!(
        matches!(spec.system_run_as, RunAs::InvokingUser),
        "a root bridge would put the bridge directory under /root"
    );
}
```

- [ ] **Step 2–4: Implement, verify**

`AppServer` gains `privileges(PrivilegeConfig)` as a stored field for this purpose — it does
not apply them, which stays `privilege::apply`'s job in `main` (step 5's deviation) — and
`service_spec` reads it.

- [ ] **Step 5: Note it in `ARCHITECTURE.md` and commit**

```markdown
| `sapphire-framework-service` | OS のサービスマネージャへの登録（systemd user/system・LaunchAgent・タスクスケジューラ） | ✅ |
```

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked
git add crates apps docs/ARCHITECTURE.md Cargo.lock
git commit -m "feat(service): add service install to the app servers and the bridge"
```

---

## What this plan does not cover

| | Left for |
|---|---|
| LaunchDaemons and real Windows services | when someone needs a service that runs without a login on those platforms |
| Verifying an install by actually starting the service | needs a machine to change; the privileged CI job of step 5 is the closest thing, and adding a service to it is a separate decision |
| Uninstalling a unit written by an older version whose path differs | there is no older version; when there is, `uninstall` gains a list of historical paths |
| Packaging (`.deb`, Homebrew, MSI) | out of scope for the framework |
