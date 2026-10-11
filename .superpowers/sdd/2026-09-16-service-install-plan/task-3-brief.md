### Task 3: macOS and Windows

**Files:**
- Create: `crates/sapphire-framework-service/src/{launchd.rs,windows.rs}`
- Create: `crates/sapphire-framework-service/tests/golden/{launchagent.plist,task.xml}`
- Test: `tests/golden.rs`

**Interfaces:**
- `fn render_launch_agent(spec: &ServiceSpec, ctx: &InstallContext) -> String`
- `fn render_task(spec: &ServiceSpec, ctx: &InstallContext) -> String`
- `fn agent_path(app_name: &str, home: &Path) -> PathBuf` — `~/Library/LaunchAgents/<label>.plist`
- The label is `net.fireturtle.sapphire.<app>`, matching the repository owner in
  `Cargo.toml`'s `repository` — a LaunchAgent label is a global namespace and a generic one
  would collide.

**User level only on both.** LaunchDaemons and real Windows services can be added when someone
needs them; an install run with administrator rights fails with the Linux-only message rather
than quietly making a user-level thing.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_launch_agent() {
    check(
        "launchagent.plist",
        &render_launch_agent(&spec(RunAs::InvokingUser, false), &ctx(Scope::User, None)),
    );
}

#[test]
fn a_launch_agent_label_is_namespaced() {
    let rendered = render_launch_agent(&spec(RunAs::InvokingUser, false), &ctx(Scope::User, None));
    assert!(
        rendered.contains("net.fireturtle.sapphire.sapphire-agent"),
        "a LaunchAgent label is a global namespace: {rendered}"
    );
}

#[test]
fn a_scheduled_task() {
    check(
        "task.xml",
        &render_task(&spec(RunAs::InvokingUser, false), &ctx(Scope::User, None)),
    );
}

#[test]
fn a_scheduled_task_runs_at_logon() {
    let rendered = render_task(&spec(RunAs::InvokingUser, false), &ctx(Scope::User, None));
    assert!(rendered.contains("LogonTrigger"), "{rendered}");
}

#[test]
fn an_argument_with_a_space_survives_the_xml() {
    let mut with_space = spec(RunAs::InvokingUser, false);
    with_space.args = vec!["server".into(), "--note".into(), "a b".into()];
    let rendered = render_task(&with_space, &ctx(Scope::User, None));
    assert!(rendered.contains("\"a b\""), "{rendered}");
}

#[test]
fn an_ampersand_in_a_description_is_escaped() {
    let mut awkward = spec(RunAs::InvokingUser, false);
    awkward.description = "Notes & ledger".into();
    let rendered = render_task(&awkward, &ctx(Scope::User, None));
    assert!(rendered.contains("Notes &amp; ledger"), "{rendered}");
    assert!(!rendered.contains("Notes & ledger"), "unescaped XML: {rendered}");
}
```

The last two are the ones that break in the field: a path with a space, and an app description
with an ampersand, both produce a file the platform rejects with a message that says nothing
useful.

- [ ] **Step 2–4: Write the golden files, implement, verify, commit**

```bash
cargo test -p sapphire-framework-service --test golden
git commit -m "feat(service): render a LaunchAgent and a scheduled task"
```

---

