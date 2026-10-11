### Task 1: The crate, the spec type, and scope detection

**Files:**
- Create: `crates/sapphire-framework-service/{Cargo.toml,src/lib.rs,src/error.rs,src/scope.rs}`
- Modify: `Cargo.toml` (workspace `members`), facade
- Test: inline `#[cfg(test)] mod tests` in `scope.rs`

**Interfaces:**
- Produces:
  - `RunAs::{Root, InvokingUser}`
  - `ServiceSpec { app_name: &'static str, description: String, args: Vec<String>, system_run_as: RunAs, privileges: Option<PrivilegeConfig>, post_install: Option<PostInstall> }`
  - `type PostInstall = Box<dyn Fn(&InstallContext) -> Result<()> + Send + Sync>`
  - `InstallContext { scope: Scope, target_user: Option<String>, unit_path: PathBuf, exe: PathBuf }`
  - `Scope::{User, System}`
  - `Environment { euid: u32, sudo_user: Option<String>, os: Os }` — every environment fact
    the decisions need, injected so the tests can vary it
  - `fn resolve_scope(env: &Environment, requested: Option<Scope>) -> Result<Scope>`
  - `fn resolve_target_user(env: &Environment, scope: Scope, spec: &ServiceSpec, override_user: Option<&str>) -> Result<Option<String>>`
  - `Error::{Io, Unsupported, MissingUser, Manager, Config}`

**Why `Environment` is a struct and not a set of calls:** every rule in the table above turns
on the effective uid, `SUDO_USER` and the platform. Reading them through a value makes all
eight combinations testable on one machine; reading them directly would make the table
untestable and it would rot.

- [ ] **Step 1: Create the manifest**

`crates/sapphire-framework-service/Cargo.toml`:

```toml
[package]
name = "sapphire-framework-service"
version.workspace = true
edition.workspace = true
description = "Register a sapphire-framework application with the OS service manager"
license.workspace = true
repository.workspace = true
keywords = ["service", "systemd", "launchd", "daemon"]
categories = ["command-line-utilities"]

[dependencies]
sapphire-server = { package = "sapphire-framework-server", version = "0.14.0", path = "../sapphire-framework-server", default-features = false }
clap.workspace = true
thiserror.workspace = true
tracing.workspace = true

[dev-dependencies]
tempfile = "3"
```

Root `Cargo.toml`: add `"crates/sapphire-framework-service",`. Facade: feature `service`.

> `-service` depends on `-server` only for `PrivilegeConfig`. If that drags in tokio and the
> workspace stack, move `PrivilegeConfig`, `UserSpec` and `HelperSpec` into their own module
> with no other dependencies, or into `-service` itself and re-export from `-server`. Decide
> when you see the dependency graph; do not leave `-service` pulling in redb to describe a
> unit file.

- [ ] **Step 2: Write the failing tests**

`crates/sapphire-framework-service/src/scope.rs`, at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn linux(euid: u32, sudo_user: Option<&str>) -> Environment {
        Environment {
            euid,
            sudo_user: sudo_user.map(str::to_owned),
            os: Os::Linux,
        }
    }

    fn spec(run_as: RunAs) -> ServiceSpec {
        ServiceSpec {
            app_name: "sapphire-bridge",
            description: "test".into(),
            args: vec!["run".into()],
            system_run_as: run_as,
            privileges: None,
            post_install: None,
        }
    }

    #[test]
    fn a_regular_user_gets_a_user_unit() {
        assert_eq!(resolve_scope(&linux(1000, None), None).unwrap(), Scope::User);
    }

    #[test]
    fn root_gets_a_system_unit() {
        assert_eq!(resolve_scope(&linux(0, None), None).unwrap(), Scope::System);
    }

    #[test]
    fn sudo_gets_a_system_unit() {
        assert_eq!(resolve_scope(&linux(0, Some("alice")), None).unwrap(), Scope::System);
    }

    #[test]
    fn the_scope_can_be_asked_for_explicitly() {
        assert_eq!(
            resolve_scope(&linux(0, Some("alice")), Some(Scope::User)).unwrap(),
            Scope::User
        );
    }

    #[test]
    fn a_regular_user_cannot_ask_for_a_system_unit() {
        let err = resolve_scope(&linux(1000, None), Some(Scope::System)).unwrap_err();
        assert!(err.to_string().contains("root"), "{err}");
    }

    #[test]
    fn a_system_unit_off_linux_is_refused() {
        let env = Environment { euid: 0, sudo_user: None, os: Os::MacOs };
        let err = resolve_scope(&env, None).unwrap_err();
        assert!(
            err.to_string().contains("supported on Linux only"),
            "the message must say what is possible instead: {err}"
        );
    }

    #[test]
    fn a_user_unit_needs_no_target_user() {
        let user = resolve_target_user(
            &linux(1000, None),
            Scope::User,
            &spec(RunAs::InvokingUser),
            None,
        )
        .unwrap();
        assert!(user.is_none(), "a user unit already runs as the right person");
    }

    #[test]
    fn a_system_unit_for_an_app_that_drops_its_own_privileges_has_no_user_line() {
        let user =
            resolve_target_user(&linux(0, Some("alice")), Scope::System, &spec(RunAs::Root), None)
                .unwrap();
        assert!(
            user.is_none(),
            "RunAs::Root means the app becomes someone else itself"
        );
    }

    #[test]
    fn a_system_unit_runs_as_the_invoking_user() {
        let user = resolve_target_user(
            &linux(0, Some("alice")),
            Scope::System,
            &spec(RunAs::InvokingUser),
            None,
        )
        .unwrap();
        assert_eq!(user.as_deref(), Some("alice"));
    }

    #[test]
    fn an_explicit_run_as_wins() {
        let user = resolve_target_user(
            &linux(0, Some("alice")),
            Scope::System,
            &spec(RunAs::InvokingUser),
            Some("bob"),
        )
        .unwrap();
        assert_eq!(user.as_deref(), Some("bob"));
    }

    #[test]
    fn root_without_sudo_user_is_refused_with_an_explanation() {
        let err =
            resolve_target_user(&linux(0, None), Scope::System, &spec(RunAs::InvokingUser), None)
                .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("--run-as"), "the message must say how to fix it: {message}");
        assert!(
            message.contains("root"),
            "and why it matters: files would be owned by root: {message}"
        );
    }
}
```

`root_without_sudo_user_is_refused_with_an_explanation` is the one that earns its keep. A
`sapphire-bridge` installed as a root system unit would put the bridge directory under `/root`
and create synced files owned by root — recoverable, but only after someone works out what
happened. Failing at install time with a sentence naming `--run-as` costs nothing.

- [ ] **Step 3: Run the tests to verify they fail, implement, verify, commit**

```bash
cargo test -p sapphire-framework-service scope
git commit -m "feat(service): decide the scope and the target user"
```

---

