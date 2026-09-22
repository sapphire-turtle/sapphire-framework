//! Rendering and activating systemd units, for [`Scope::User`] and [`Scope::System`].
//!
//! The unit file is the contract between this crate and the machine: what it says is what
//! the service manager does. It turns the scope decision's three answers — which unit file
//! to write, which user the service runs as, and what starts it — into systemd's own words.
//! See the process-architecture spec, §3.2 and §9 step 10.

use std::path::{Path, PathBuf};

use crate::scope::{InstallContext, Scope, ServiceSpec};

/// The unit file a scope installs to.
///
/// A user unit lives under the invoking user's own `~/.config`, so it is theirs to remove;
/// a system unit lives in `/etc/systemd/system`, where the manager looks for system-wide
/// units.
pub fn unit_path(app_name: &str, scope: Scope, home: &Path) -> PathBuf {
    match scope {
        Scope::User => home
            .join(".config/systemd/user")
            .join(format!("{app_name}.service")),
        Scope::System => PathBuf::from("/etc/systemd/system").join(format!("{app_name}.service")),
    }
}

/// The commands that activate an installed unit.
///
/// `daemon-reload` first, so the manager sees the file just written; then `enable --now`,
/// which both registers the unit for future boots and starts it now.
pub fn activation(app_name: &str, scope: Scope) -> Vec<Vec<String>> {
    let scope_flag: &[&str] = match scope {
        Scope::User => &["--user"],
        Scope::System => &[],
    };
    let command = |tail: &[&str]| -> Vec<String> {
        std::iter::once("systemctl")
            .chain(scope_flag.iter().copied())
            .chain(tail.iter().copied())
            .map(str::to_owned)
            .collect()
    };
    vec![
        command(&["daemon-reload"]),
        command(&["enable", "--now", app_name]),
    ]
}

/// The advice an install should print for a machine meant to run the service without a login.
///
/// A user unit dies with its session; `loginctl enable-linger` lets it survive one, and
/// needs root unless the invoker is the user being lingered. A system unit runs regardless
/// of logins, so a system install has no hint.
pub fn linger_hint(scope: Scope, user: Option<&str>) -> Option<String> {
    match scope {
        Scope::System => None,
        Scope::User => match user {
            Some(user) => Some(format!(
                "to run without a login, run: sudo loginctl enable-linger {user}"
            )),
            None => Some("to run without a login, run: loginctl enable-linger".to_owned()),
        },
    }
}

/// Turn a [`ServiceSpec`] and a resolved [`InstallContext`] into a systemd unit file.
///
/// The scope decides the file's shape: a user unit runs as its owner with no network
/// ordering, a system unit waits for the network and either carries a `User=` line (a named
/// user) or none at all — an app that drops privileges itself carries its two users in the
/// environment instead, because a `User=` line would stop it becoming them.
pub fn render_unit(spec: &ServiceSpec, ctx: &InstallContext) -> String {
    let mut unit = String::new();
    unit.push_str("[Unit]\n");
    unit.push_str(&format!("Description={}\n", spec.description));
    if ctx.scope == Scope::System {
        // A system unit may come up before the network does; a user unit starts after the
        // session is up already.
        unit.push_str("After=network-online.target\n");
        unit.push_str("Wants=network-online.target\n");
    }
    unit.push_str("\n[Service]\nType=simple\n");
    match (ctx.scope, &ctx.target_user) {
        (Scope::User, _) => {} // A user unit runs as its owner; systemd needs no User= line.
        (Scope::System, Some(user)) => unit.push_str(&format!("User={user}\n")),
        (Scope::System, None) => {
            unit.push_str(
                "# No User=: this service starts as root in order to become two different users, \
                 and\n# drops privileges itself. See the process-architecture spec, section 3.\n",
            );
            if let Some(privileges) = &spec.privileges {
                unit.push_str("Environment=SAPPHIRE_RUN_AS=");
                unit.push_str(&privileges.run_as.to_string());
                unit.push('\n');
                if let Some(helper) = &privileges.helper {
                    unit.push_str("Environment=SAPPHIRE_HELPER_AS=");
                    unit.push_str(&helper.user.to_string());
                    unit.push('\n');
                }
            }
        }
    }
    unit.push_str(&format!(
        "ExecStart={} {}\n",
        ctx.exe.display(),
        spec.args.join(" ")
    ));
    unit.push_str("Restart=on-failure\n");
    unit.push_str("RestartSec=5\n");
    unit.push_str("\n[Install]\n");
    unit.push_str(match ctx.scope {
        Scope::User => "WantedBy=default.target\n",
        Scope::System => "WantedBy=multi-user.target\n",
    });
    unit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(words: &[&[&str]]) -> Vec<Vec<String>> {
        words
            .iter()
            .map(|c| c.iter().map(|w| (*w).to_owned()).collect())
            .collect()
    }

    #[test]
    fn a_user_unit_lives_under_the_users_own_config() {
        let path = unit_path("sapphire-bridge", Scope::User, Path::new("/home/alice"));
        assert_eq!(
            path,
            PathBuf::from("/home/alice/.config/systemd/user/sapphire-bridge.service")
        );
    }

    #[test]
    fn a_system_unit_lives_in_etc() {
        let path = unit_path("sapphire-bridge", Scope::System, Path::new("/home/alice"));
        assert_eq!(
            path,
            PathBuf::from("/etc/systemd/system/sapphire-bridge.service")
        );
    }

    #[test]
    fn a_user_unit_activates_through_the_user_manager() {
        assert_eq!(
            activation("sapphire-bridge", Scope::User),
            commands(&[
                &["systemctl", "--user", "daemon-reload"],
                &["systemctl", "--user", "enable", "--now", "sapphire-bridge"],
            ])
        );
    }

    #[test]
    fn a_system_unit_activates_without_the_user_flag() {
        assert_eq!(
            activation("sapphire-bridge", Scope::System),
            commands(&[
                &["systemctl", "daemon-reload"],
                &["systemctl", "enable", "--now", "sapphire-bridge"],
            ])
        );
    }

    #[test]
    fn a_user_install_hints_at_linger() {
        let hint = linger_hint(Scope::User, None).unwrap();
        assert!(hint.contains("loginctl enable-linger"), "{hint}");
    }

    #[test]
    fn a_linger_hint_for_a_named_user_uses_sudo() {
        let hint = linger_hint(Scope::User, Some("alice")).unwrap();
        assert!(hint.contains("sudo loginctl enable-linger alice"), "{hint}");
    }

    #[test]
    fn a_system_install_has_no_linger_hint() {
        assert_eq!(linger_hint(Scope::System, Some("alice")), None);
    }
}
