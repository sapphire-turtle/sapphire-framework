//! Starting as root in order to become two different users, and then neither being root nor
//! able to become root again.
//!
//! See `docs/superpowers/specs/2026-09-16-process-architecture-design.md` §3, and
//! `sapphire-agent` issue #257 for what this is for: an agent's shell and generic file tools
//! keep their freedom while losing access to the workspace.
//!
//! Unix only. On any other platform a configuration that asks for this fails.
//!
//! The calling order is forced (spec §3.1): the helper is forked before the drop, because
//! becoming another user needs root, and the IPC socket is bound after `apply` returns,
//! because otherwise it would be created owned by root.
//!
//! ```rust,ignore
//! static CTX: AppContext = AppContext::new("sapphire-agent");
//!
//! fn main() -> anyhow::Result<()> {
//!     CTX.init(AppKind::Server);
//!     let runtime = sapphire_ipc::runtime_dir()?;
//!
//!     // Everything below this line runs as the human user.
//!     let privileges = privilege::apply(
//!         &config.privileges,
//!         &[&runtime, CTX.cache_dir(), CTX.data_dir(), CTX.config_dir()],
//!     )?;
//!
//!     let tools = privileges.helper.map(|h| ToolBroker::new(h.socket));
//!     // … build and run the AppServer …
//!     Ok(())
//! }
//! ```

// The spec types (`UserSpec`, `HelperSpec`, `PrivilegeConfig`) moved to
// `sapphire-framework-service::privilege` (see that module's docs for why) and are
// re-exported here — and from the crate root — so every existing path keeps working.
pub use sapphire_framework_service::privilege::{HelperSpec, PrivilegeConfig, UserSpec};

#[cfg(unix)]
mod drop;
#[cfg(unix)]
mod helper;
#[cfg(unix)]
mod users;

#[cfg(unix)]
pub use drop::{drop_to, hand_over, is_root};
#[cfg(unix)]
pub use helper::HelperHandle;
#[cfg(unix)]
pub use users::{ResolvedUser, current_uid, resolve};

use std::path::Path;

use crate::error::{Error, Result};

/// The result of applying a [`PrivilegeConfig`].
///
/// Defined on every platform so that calling code needs no `cfg`, even though on non-Unix
/// [`apply`] never returns one.
#[derive(Debug)]
pub struct Privileges {
    /// The user this process now is.
    #[cfg(unix)]
    pub run_as: ResolvedUser,
    /// The helper, if one was configured. Its socket is the application's to use.
    #[cfg(unix)]
    pub helper: Option<HelperHandle>,
}

/// Run the privilege-separation sequence of spec §3.1.
///
/// `dirs` are handed to `run_as` before the drop: pass every directory the application will
/// write to — the cache, data and config trees from `AppContext`, and
/// [`sapphire_ipc::runtime_dir`]. Directories that do not exist are skipped.
///
/// Call this **before** binding any socket and **before** connecting to the bridge. After it
/// returns, this process is an ordinary process of `run_as` and can do neither of those
/// things as root.
#[cfg(unix)]
pub fn apply(config: &PrivilegeConfig, dirs: &[&Path]) -> Result<Privileges> {
    let run_as = resolve(&config.run_as)?;

    if !drop::is_root() {
        // Without root there is no second identity to be had. Succeed only if the
        // configuration describes what is already true.
        if run_as.uid != current_uid() {
            return Err(Error::Privilege(format!(
                "cannot run as {} without root: this process is uid {}",
                config.run_as,
                current_uid()
            )));
        }
        if config.helper.is_some() {
            return Err(Error::Privilege(
                "a helper needs root: without it the helper would run with this server's own \
                 privileges, which separates nothing"
                    .to_owned(),
            ));
        }
        drop::hand_over(dirs, &run_as)?;
        return Ok(Privileges {
            run_as,
            helper: None,
        });
    }

    // 2. The directories become the user's, while we can still chown.
    drop::hand_over(dirs, &run_as)?;

    // 3-4. The helper, while we can still become someone else.
    let helper = match &config.helper {
        Some(spec) => {
            let user = resolve(&spec.user)?;
            if user.uid == run_as.uid {
                return Err(Error::Privilege(format!(
                    "the helper user and run_as are both {}; a helper with the same identity \
                     separates nothing",
                    user.name
                )));
            }
            Some(helper::spawn_helper(spec, &user)?)
        }
        None => None,
    };

    // 5. The drop, verified.
    drop::drop_to(&run_as)?;
    tracing::info!(
        user = %run_as.name,
        helper = ?helper.as_ref().map(|h| (&h.user.name, h.pid)),
        "dropped privileges"
    );

    Ok(Privileges { run_as, helper })
}

/// Privilege separation is a Unix facility.
///
/// A configuration that asks for it on another platform fails here rather than being
/// silently ignored: an application that believes it is separated and is not is worse off
/// than one that knows it cannot be.
#[cfg(not(unix))]
pub fn apply(_config: &PrivilegeConfig, _dirs: &[&Path]) -> Result<Privileges> {
    Err(Error::Privilege(
        "privilege separation is not available on this platform".to_owned(),
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn my_spec() -> UserSpec {
        UserSpec::Uid(current_uid())
    }

    #[test]
    fn a_non_root_process_may_run_as_itself() {
        if drop::is_root() {
            return; // the root job covers the privileged path
        }
        let tmp = tempfile::tempdir().unwrap();
        let config = PrivilegeConfig {
            run_as: my_spec(),
            helper: None,
        };
        let privileges = apply(&config, &[tmp.path()]).unwrap();
        assert_eq!(privileges.run_as.uid, current_uid());
        assert!(privileges.helper.is_none());
    }

    #[test]
    fn a_non_root_process_may_not_run_as_someone_else() {
        if drop::is_root() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        // uid 1 is `daemon` or `bin` on every Unix, and is never the test runner.
        let config = PrivilegeConfig {
            run_as: UserSpec::Uid(1),
            helper: None,
        };
        let err = apply(&config, &[tmp.path()]).unwrap_err();
        assert!(err.to_string().contains("without root"), "{err}");
    }

    #[test]
    fn a_non_root_process_may_not_ask_for_a_helper() {
        if drop::is_root() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let config = PrivilegeConfig {
            run_as: my_spec(),
            helper: Some(HelperSpec {
                user: my_spec(),
                program: "/bin/true".into(),
                args: vec![],
            }),
        };
        let err = apply(&config, &[tmp.path()]).unwrap_err();
        assert!(err.to_string().contains("helper"), "{err}");
    }

    #[test]
    fn running_as_root_is_refused_outright() {
        let tmp = tempfile::tempdir().unwrap();
        let config = PrivilegeConfig {
            run_as: UserSpec::Uid(0),
            helper: None,
        };
        let err = apply(&config, &[tmp.path()]).unwrap_err();
        assert!(err.to_string().contains("root"), "{err}");
    }

    #[test]
    fn a_configuration_round_trips_through_toml() {
        let text = r#"
run_as = "alice"

[helper]
user = "sapphire-agent-tools"
program = "/usr/lib/sapphire-agent/tool-broker"
args = ["--quiet"]
"#;
        let config: PrivilegeConfig = toml::from_str(text).unwrap();
        assert_eq!(config.run_as, UserSpec::Name("alice".into()));
        let helper = config.helper.unwrap();
        assert_eq!(helper.user, UserSpec::Name("sapphire-agent-tools".into()));
        assert_eq!(helper.args, vec!["--quiet".to_owned()]);
    }
}
