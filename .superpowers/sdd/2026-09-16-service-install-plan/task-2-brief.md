### Task 2: systemd units, against golden files

**Files:**
- Create: `crates/sapphire-framework-service/src/systemd.rs`
- Create: `crates/sapphire-framework-service/tests/{golden.rs,golden/*.service}`
- Test: `tests/golden.rs`

**Interfaces:**
- Produces:
  - `fn unit_path(app_name: &str, scope: Scope, home: &Path) -> PathBuf`
  - `fn render_unit(spec: &ServiceSpec, ctx: &InstallContext) -> String`
  - `fn activation(app_name: &str, scope: Scope) -> Vec<Vec<String>>` — the commands to run
  - `fn linger_hint(scope: Scope, user: Option<&str>) -> Option<String>`

- [ ] **Step 1: Write the failing tests**

`crates/sapphire-framework-service/tests/golden.rs`:

```rust
//! Generated unit files, compared against copies checked into the repository.
//!
//! When one of these fails, read the diff before regenerating: a unit file is the contract
//! between this crate and the machine, and a change to it is a change of behaviour.

use std::path::{Path, PathBuf};

use sapphire_framework_service::{
    InstallContext, RunAs, Scope, ServiceSpec, render_unit,
};

fn golden(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}; create it from the failure output", path.display()))
}

fn check(name: &str, rendered: &str) {
    let want = golden(name);
    assert_eq!(
        rendered.trim_end(),
        want.trim_end(),
        "\n--- generated ---\n{rendered}\n--- {name} ---\n{want}\n"
    );
}

fn spec(run_as: RunAs, privileges: bool) -> ServiceSpec {
    ServiceSpec {
        app_name: "sapphire-agent",
        description: "Sapphire agent server".into(),
        args: vec!["server".into(), "run".into()],
        system_run_as: run_as,
        privileges: privileges.then(|| sapphire_server::PrivilegeConfig {
            run_as: "alice".parse().unwrap(),
            helper: Some(sapphire_server::HelperSpec {
                user: "sapphire-agent-tools".parse().unwrap(),
                program: PathBuf::from("/usr/lib/sapphire-agent/tool-broker"),
                args: vec![],
            }),
        }),
        post_install: None,
    }
}

fn ctx(scope: Scope, target_user: Option<&str>) -> InstallContext {
    InstallContext {
        scope,
        target_user: target_user.map(str::to_owned),
        unit_path: PathBuf::from("/dev/null"),
        exe: PathBuf::from("/usr/bin/sapphire-agent"),
    }
}

#[test]
fn a_user_unit() {
    check(
        "user.service",
        &render_unit(&spec(RunAs::InvokingUser, false), &ctx(Scope::User, None)),
    );
}

#[test]
fn a_system_unit_running_as_a_named_user() {
    check(
        "system-user.service",
        &render_unit(
            &spec(RunAs::InvokingUser, false),
            &ctx(Scope::System, Some("alice")),
        ),
    );
}

#[test]
fn a_system_unit_that_drops_its_own_privileges() {
    check(
        "system-privsep.service",
        &render_unit(&spec(RunAs::Root, true), &ctx(Scope::System, None)),
    );
}

#[test]
fn a_user_unit_has_no_network_ordering() {
    let rendered = render_unit(&spec(RunAs::InvokingUser, false), &ctx(Scope::User, None));
    assert!(
        !rendered.contains("network-online.target"),
        "a user unit starts after the session is up already"
    );
}

#[test]
fn a_system_unit_waits_for_the_network() {
    let rendered =
        render_unit(&spec(RunAs::InvokingUser, false), &ctx(Scope::System, Some("alice")));
    assert!(rendered.contains("After=network-online.target"), "{rendered}");
}

#[test]
fn exec_start_is_absolute_and_carries_the_arguments() {
    let rendered = render_unit(&spec(RunAs::InvokingUser, false), &ctx(Scope::User, None));
    assert!(
        rendered.contains("ExecStart=/usr/bin/sapphire-agent server run"),
        "{rendered}"
    );
}

#[test]
fn a_privilege_separated_unit_has_no_user_line() {
    let rendered = render_unit(&spec(RunAs::Root, true), &ctx(Scope::System, None));
    assert!(
        !rendered.contains("\nUser="),
        "the app becomes someone else itself; a User= line would stop it being able to:\n{rendered}"
    );
}

#[test]
fn a_privilege_separated_unit_names_both_users() {
    let rendered = render_unit(&spec(RunAs::Root, true), &ctx(Scope::System, None));
    assert!(rendered.contains("alice"), "{rendered}");
    assert!(rendered.contains("sapphire-agent-tools"), "{rendered}");
}
```

`crates/sapphire-framework-service/tests/golden/user.service`:

```ini
[Unit]
Description=Sapphire agent server

[Service]
Type=simple
ExecStart=/usr/bin/sapphire-agent server run
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

`tests/golden/system-user.service`:

```ini
[Unit]
Description=Sapphire agent server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=alice
ExecStart=/usr/bin/sapphire-agent server run
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

`tests/golden/system-privsep.service`:

```ini
[Unit]
Description=Sapphire agent server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
# No User=: this service starts as root in order to become two different users, and
# drops privileges itself. See the process-architecture spec, section 3.
Environment=SAPPHIRE_RUN_AS=alice
Environment=SAPPHIRE_HELPER_AS=sapphire-agent-tools
ExecStart=/usr/bin/sapphire-agent server run
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sapphire-framework-service --test golden`
Expected: FAIL — `render_unit` does not exist.

- [ ] **Step 3: Implement, verify, commit**

`unit_path` is `~/.config/systemd/user/<app>.service` for `Scope::User` and
`/etc/systemd/system/<app>.service` for `Scope::System`. `activation` returns
`[["systemctl", "--user", "daemon-reload"], ["systemctl", "--user", "enable", "--now", "<app>"]]`
or the system equivalents. `linger_hint` returns the `loginctl enable-linger` sentence for a
user unit, and `None` for a system one.

```bash
cargo test -p sapphire-framework-service --test golden
git commit -m "feat(service): render systemd units"
```

---

