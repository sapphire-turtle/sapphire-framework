### Task 4: `ServiceManager` and `ServiceCommand`

**Files:**
- Create: `crates/sapphire-framework-service/src/manager.rs`
- Modify: `crates/sapphire-framework-service/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in `manager.rs`

**Interfaces:**
- Produces:
  - `trait ServiceManager { fn write_unit(&self, path: &Path, body: &str) -> Result<()>; fn run(&self, command: &[String]) -> Result<String>; fn remove_unit(&self, path: &Path) -> Result<()>; }`
  - `SystemManager` — the real one
  - `RecordingManager` — records calls and returns canned output
  - `ServiceCommand::{Install(InstallArgs), Uninstall, Status}` (`clap::Subcommand`)
  - `InstallArgs { user: bool, system: bool, run_as: Option<String>, keep_helper: bool }`
  - `fn install(spec: &ServiceSpec, args: &InstallArgs, env: &Environment, manager: &dyn ServiceManager) -> Result<InstallContext>`
  - `fn uninstall(...)`, `fn status(...)`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ServiceSpec {
        ServiceSpec {
            app_name: "sapphire-agent",
            description: "Sapphire agent server".into(),
            args: vec!["server".into(), "run".into()],
            system_run_as: RunAs::InvokingUser,
            privileges: None,
            post_install: None,
        }
    }

    fn privileges_for(run_as: &str, helper: &str) -> sapphire_server::PrivilegeConfig {
        sapphire_server::PrivilegeConfig {
            run_as: run_as.parse().unwrap(),
            helper: Some(sapphire_server::HelperSpec {
                user: helper.parse().unwrap(),
                program: std::path::PathBuf::from("/usr/lib/sapphire-agent/tool-broker"),
                args: vec![],
            }),
        }
    }

    fn linux_user() -> Environment {
        Environment { euid: 1000, sudo_user: None, os: Os::Linux }
    }

    fn linux_root() -> Environment {
        Environment { euid: 0, sudo_user: Some("alice".into()), os: Os::Linux }
    }

    #[test]
    fn installing_writes_a_unit_and_activates_it() {
        let manager = RecordingManager::default();
        install(&spec(), &InstallArgs::default(), &linux_user(), &manager).unwrap();

        let calls = manager.calls();
        assert_eq!(calls.units.len(), 1);
        assert!(calls.units[0].0.ends_with("sapphire-agent.service"), "{:?}", calls.units[0].0);
        assert!(
            calls.commands.iter().any(|c| c.contains(&"enable".to_owned())),
            "{:?}",
            calls.commands
        );
    }

    #[test]
    fn a_user_install_uses_the_user_flag() {
        let manager = RecordingManager::default();
        install(&spec(), &InstallArgs::default(), &linux_user(), &manager).unwrap();
        assert!(
            manager.calls().commands.iter().all(|c| c.contains(&"--user".to_owned())),
            "{:?}",
            manager.calls().commands
        );
    }

    #[test]
    fn a_system_install_does_not() {
        let manager = RecordingManager::default();
        install(&spec(), &InstallArgs::default(), &linux_root(), &manager).unwrap();
        assert!(
            manager.calls().commands.iter().all(|c| !c.contains(&"--user".to_owned())),
            "{:?}",
            manager.calls().commands
        );
    }

    #[test]
    fn a_failing_activation_leaves_no_unit_behind() {
        let manager = RecordingManager::failing_on("enable");
        assert!(install(&spec(), &InstallArgs::default(), &linux_user(), &manager).is_err());
        assert!(
            !manager.calls().removed.is_empty(),
            "a half-installed service is worse than none: the unit must be cleaned up"
        );
    }

    #[test]
    fn uninstalling_stops_disables_and_removes() {
        let manager = RecordingManager::default();
        uninstall(&spec(), &InstallArgs::default(), &linux_user(), &manager).unwrap();

        let calls = manager.calls();
        let flat: Vec<String> = calls.commands.iter().flatten().cloned().collect();
        assert!(flat.contains(&"disable".to_owned()), "{flat:?}");
        assert_eq!(calls.removed.len(), 1);
    }

    #[test]
    fn uninstalling_something_that_is_not_installed_is_not_an_error() {
        let manager = RecordingManager::failing_on("disable");
        uninstall(&spec(), &InstallArgs::default(), &linux_user(), &manager)
            .expect("uninstall is idempotent");
    }

    #[test]
    fn status_reports_what_the_manager_said() {
        let manager = RecordingManager::returning("active");
        let text = status(&spec(), &InstallArgs::default(), &linux_user(), &manager).unwrap();
        assert!(text.contains("active"), "{text}");
    }

    #[test]
    fn asking_for_both_scopes_is_refused() {
        let args = InstallArgs { user: true, system: true, ..InstallArgs::default() };
        let err = install(&spec(), &args, &linux_root(), &RecordingManager::default()).unwrap_err();
        assert!(err.to_string().contains("--user"), "{err}");
    }

    #[test]
    fn no_test_touches_the_real_service_manager() {
        // Stated as a test so the intent is visible where it can be read. `SystemManager` is
        // the only type that runs anything, and it is never constructed above.
        let source = include_str!("manager.rs");
        let constructions = source.matches("SystemManager").count();
        assert!(
            constructions <= 2,
            "SystemManager appears {constructions} times; tests must use RecordingManager"
        );
    }
}
```

`a_failing_activation_leaves_no_unit_behind` is the one to get right: a unit file written but
never enabled is invisible to `systemctl status` and springs to life at the next reboot.

- [ ] **Step 2–5: Implement, verify, commit**

```bash
cargo test -p sapphire-framework-service
git commit -m "feat(service): install, uninstall and report through a manager trait"
```

---

