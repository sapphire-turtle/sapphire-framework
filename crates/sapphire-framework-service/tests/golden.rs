//! Generated unit files, compared against copies checked into the repository.
//!
//! When one of these fails, read the diff before regenerating: a unit file is the contract
//! between this crate and the machine, and a change to it is a change of behaviour.

use std::path::{Path, PathBuf};

use sapphire_framework_service::{
    HelperSpec, InstallContext, PrivilegeConfig, RunAs, Scope, ServiceSpec, render_unit,
};

fn golden(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
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
        privileges: privileges.then(|| PrivilegeConfig {
            run_as: "alice".parse().unwrap(),
            helper: Some(HelperSpec {
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
    let rendered = render_unit(
        &spec(RunAs::InvokingUser, false),
        &ctx(Scope::System, Some("alice")),
    );
    assert!(
        rendered.contains("After=network-online.target"),
        "{rendered}"
    );
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
