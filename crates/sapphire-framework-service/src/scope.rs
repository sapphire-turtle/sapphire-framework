//! Which kind of unit to install, and which user it runs as.
//!
//! Every rule here turns on the effective uid, `SUDO_USER` and the platform; they arrive
//! bundled in an [`Environment`] so all combinations are testable on one machine, and so the
//! rules are functions of values rather than of the world they run in.

use crate::error::{Error, Result};
use crate::privilege::PrivilegeConfig;

/// Which kind of unit to install.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// A user unit, activated with `systemctl --user`.
    User,
    /// A system unit, activated with plain `systemctl`.
    System,
}

/// Which OS user a system unit's service runs as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunAs {
    /// No `User=` line: an app that starts as root and drops privileges itself, such as
    /// `sapphire-agent`. Privilege separation (spec §3) wins over any explicit user.
    Root,
    /// `User=$SUDO_USER`, or whatever `--run-as` names.
    InvokingUser,
}

/// Every environment fact the scope and target-user decisions need.
///
/// Read through a value rather than from the machine, so every combination is testable
/// without becoming root, without `sudo`, and without another operating system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Environment {
    /// The effective user id of the process doing the installing.
    pub euid: u32,
    /// `SUDO_USER`: the human behind a `sudo` invocation, if there was one.
    pub sudo_user: Option<String>,
    /// The operating system, which decides what a system-wide install may even be.
    pub os: Os,
}

/// The platforms an install can run on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    /// Linux, with systemd.
    Linux,
    /// macOS, with launchd.
    MacOs,
    /// Windows, with the Task Scheduler.
    Windows,
}

/// What an application wants installed.
///
/// Written by the application (`AppServer::service_spec` builds one); the rest of the crate
/// turns it into files and manager calls.
pub struct ServiceSpec {
    /// The application's name, as the manager and the paths name it: `sapphire-bridge`.
    pub app_name: &'static str,
    /// One line for the unit's `Description=` (or the platform's equivalent).
    pub description: String,
    /// Arguments handed to the executable, after the executable's own absolute path.
    pub args: Vec<String>,
    /// Which user a **system** unit runs as. A user unit always runs as its owner.
    pub system_run_as: RunAs,
    /// The app's privilege separation, when it has any; a system unit carries it in the
    /// environment instead of a `User=` line.
    pub privileges: Option<PrivilegeConfig>,
    /// Runs after activation, with the resolved context: for an app that wants to leave
    /// something in the target user's configuration.
    pub post_install: Option<PostInstall>,
}

impl std::fmt::Debug for ServiceSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `PostInstall` is a boxed closure; name it rather than trying to show it.
        f.debug_struct("ServiceSpec")
            .field("app_name", &self.app_name)
            .field("description", &self.description)
            .field("args", &self.args)
            .field("system_run_as", &self.system_run_as)
            .field("privileges", &self.privileges)
            .field("post_install", &self.post_install.as_ref().map(|_| "set"))
            .finish()
    }
}

/// A hook that runs after activation, with everything the install decided.
pub type PostInstall = Box<dyn Fn(&InstallContext) -> Result<()> + Send + Sync>;

/// Everything an install resolved, handed to [`PostInstall`].
#[derive(Clone, Debug)]
pub struct InstallContext {
    /// The kind of unit that was installed.
    pub scope: Scope,
    /// The user a system unit runs as, when it has one.
    pub target_user: Option<String>,
    /// Where the unit was written.
    pub unit_path: std::path::PathBuf,
    /// The absolute path of the running executable, the unit's `ExecStart` head.
    pub exe: std::path::PathBuf,
}

/// Decide which kind of unit to install.
///
/// A request wins when it is given; otherwise root gets a system unit and everyone else a
/// user one.
///
/// A regular user asking for a system unit is refused: they could write the file, but not
/// enable it, and a half-installed service is worse than a refused one. A system unit is
/// Linux only; elsewhere the refusal says what is possible instead.
pub fn resolve_scope(env: &Environment, requested: Option<Scope>) -> Result<Scope> {
    match requested {
        Some(Scope::User) => Ok(Scope::User),
        Some(Scope::System) => {
            if env.euid != 0 {
                Err(Error::Unsupported(
                    "a system unit needs root; install as a regular user with --user, or run \
                     the install under sudo"
                        .to_owned(),
                ))
            } else if env.os == Os::Linux {
                Ok(Scope::System)
            } else {
                Err(Error::Unsupported(
                    "system-wide installation is supported on Linux only; a user unit is the \
                     most this platform offers"
                        .to_owned(),
                ))
            }
        }
        None => {
            if env.euid == 0 {
                if env.os == Os::Linux {
                    Ok(Scope::System)
                } else {
                    Err(Error::Unsupported(
                        "system-wide installation is supported on Linux only; a user unit is \
                         the most this platform offers"
                            .to_owned(),
                    ))
                }
            } else {
                Ok(Scope::User)
            }
        }
    }
}

/// Decide which user a system unit's service runs as.
///
/// `override_user` (the CLI's `--run-as`) wins; then the spec's own [`RunAs`]; then
/// `SUDO_USER`. An app with [`RunAs::Root`] drops privileges itself, so the unit carries no
/// `User=` and privilege separation wins over any explicit user.
///
/// Fails with the fix in the message when nothing names a user: running `sapphire-bridge` as
/// root would put the bridge directory under `/root` and create synced files owned by root.
pub fn resolve_target_user(
    env: &Environment,
    scope: Scope,
    spec: &ServiceSpec,
    override_user: Option<&str>,
) -> Result<Option<String>> {
    if scope == Scope::User {
        return Ok(None);
    }

    let has_privileges = spec.privileges.is_some();
    let runs_as_root = matches!(spec.system_run_as, RunAs::Root) || has_privileges;
    if runs_as_root {
        // The app becomes someone else itself. An override cannot be honoured here, so it
        // is refused rather than silently dropped.
        if override_user.is_some() {
            return Err(Error::Config(
                "--run-as cannot be honoured: this app drops privileges itself, so a system \
                 unit runs as root and carries no User= line"
                    .to_owned(),
            ));
        }
        return Ok(None);
    }

    if let Some(user) = override_user {
        return Ok(Some(user.to_owned()));
    }

    let invoking = env.sudo_user.as_ref();
    if let Some(user) = invoking {
        return Ok(Some(user.clone()));
    }
    Err(Error::MissingUser(
        "this system unit would run as root; files it creates would be owned by root. \
         Pass --run-as <user>, or install as a regular user with --user"
            .to_owned(),
    ))
}

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
        assert_eq!(
            resolve_scope(&linux(1000, None), None).unwrap(),
            Scope::User
        );
    }

    #[test]
    fn root_gets_a_system_unit() {
        assert_eq!(resolve_scope(&linux(0, None), None).unwrap(), Scope::System);
    }

    #[test]
    fn sudo_gets_a_system_unit() {
        assert_eq!(
            resolve_scope(&linux(0, Some("alice")), None).unwrap(),
            Scope::System
        );
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
        let env = Environment {
            euid: 0,
            sudo_user: None,
            os: Os::MacOs,
        };
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
        assert!(
            user.is_none(),
            "a user unit already runs as the right person"
        );
    }

    #[test]
    fn a_system_unit_for_an_app_that_drops_its_own_privileges_has_no_user_line() {
        let user = resolve_target_user(
            &linux(0, Some("alice")),
            Scope::System,
            &spec(RunAs::Root),
            None,
        )
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
        let err = resolve_target_user(
            &linux(0, None),
            Scope::System,
            &spec(RunAs::InvokingUser),
            None,
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("--run-as"),
            "the message must say how to fix it: {message}"
        );
        assert!(
            message.contains("root"),
            "and why it matters: files would be owned by root: {message}"
        );
    }
}
