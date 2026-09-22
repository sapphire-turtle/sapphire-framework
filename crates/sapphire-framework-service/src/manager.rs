//! The install, uninstall and status flows, and the manager they go through.
//!
//! Everything that touches the machine — writing a unit file, running a manager command,
//! removing one — goes through the [`ServiceManager`] trait. The real implementation writes
//! real files and runs real commands; [`RecordingManager`] records what it was asked and
//! answers from canned output, so the flows can be driven by tests that never touch the
//! host's own service manager.
//!
//! [`install`] is the scope decision table in motion: it resolves the scope and the target
//! user ([`crate::scope`]), renders the platform's file ([`crate::systemd`],
//! [`crate::launchd`], [`crate::windows`]), writes it through the manager and runs the
//! activation commands. When activation fails, the half-written unit is removed before the
//! error returns: a unit file that was written but never enabled is invisible to `status`
//! and springs to life at the next reboot. [`uninstall`] is idempotent — stopping,
//! disabling or removing something that is not there is not an error — and [`status`]
//! reports whatever the manager said.
//!
//! Off Linux the flows are user level only, because [`resolve_scope`] refuses a system
//! scope there before any of this runs: macOS goes through `launchctl` with the LaunchAgent
//! from [`crate::launchd`], Windows through `schtasks` with the task XML from
//! [`crate::windows`]. The activation hint for a user unit on a machine that should run it
//! without a login ([`crate::systemd::linger_hint`]) is not carried here: an install
//! returns an [`InstallContext`], and the app CLI asks for the hint itself.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::error::{Error, Result};
use crate::launchd::{agent_path, label, render_launch_agent};
use crate::scope::{
    Environment, InstallContext, Os, Scope, ServiceSpec, resolve_scope, resolve_target_user,
};
use crate::systemd::{activation, render_unit, unit_path};
use crate::windows::render_task;

/// The commands a service manager is driven with.
///
/// The one place that runs anything is the real implementation below; every flow takes a
/// `&dyn ServiceManager` so a test can hand it a [`RecordingManager`] instead and leave the
/// host's manager alone.
pub trait ServiceManager {
    /// Write a unit file, creating its directory if it is missing.
    fn write_unit(&self, path: &Path, body: &str) -> Result<()>;

    /// Run a manager command and return its standard output.
    ///
    /// The command's first element is the program, the rest its arguments; nothing here
    /// goes through a shell, so no word needs quoting.
    fn run(&self, command: &[String]) -> Result<String>;

    /// Remove a unit file. Removing one that is not there is not an error.
    fn remove_unit(&self, path: &Path) -> Result<()>;

    /// Hand a file the install wrote over to the user the service runs as, because a
    /// post-install hook about to write there runs as root.
    ///
    /// Only a Linux system install whose [`ServiceSpec`] named a user calls this; every
    /// other install needs nobody but its writer. The default does nothing: ownership
    /// changes are real work on a real machine, and each implementation owns how, whether
    /// and when its files change hands. Returning an error fails the install — the user the
    /// hook is about to write for would otherwise find files owned by root.
    fn chown_to_user(&self, path: &Path, user: &str) -> Result<()> {
        let _ = (path, user);
        Ok(())
    }
}

/// The real service manager: real files, real commands.
///
/// Only [`install`], [`uninstall`] and [`status`] construct it — the CLI in an app does —
/// and no test ever does.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemManager;

impl ServiceManager for SystemManager {
    fn write_unit(&self, path: &Path, body: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            // A user unit's directory (`~/.config/systemd/user`) may not exist yet on a
            // machine that never had one.
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, body)?;
        Ok(())
    }

    fn run(&self, command: &[String]) -> Result<String> {
        let Some((program, args)) = command.split_first() else {
            return Err(Error::Manager("an empty command".to_owned()));
        };
        let output = std::process::Command::new(program).args(args).output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(Error::Manager(format!(
                "{program} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }

    fn remove_unit(&self, path: &Path) -> Result<()> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            // An uninstall of something that was never installed has nothing to remove.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn chown_to_user(&self, path: &Path, user: &str) -> Result<()> {
        hand_over_to_user(path, user)
    }
}

/// Hand one file over to `user`, on Linux only.
///
/// Ownership is a Unix idea; the flows never call this off Linux, and a cfg here keeps the
/// crate compiling where the call has no counterpart. The call runs while root, so the
/// lookup goes through the C library's `getpwnam`; linking against libc anyway matches the
/// crate's other Linux-only peers. A user who does not exist fails the install: a hook
/// about to write into their directories has nowhere it belongs.
#[cfg(target_os = "linux")]
fn hand_over_to_user(path: &Path, user: &str) -> Result<()> {
    let name = std::ffi::CString::new(user)
        .map_err(|_| Error::Config(format!("the user {user:?} contains a NUL")))?;
    // SAFETY: `name` is NUL-terminated and alive for the call, and `passwd` may stay
    // uninitialised while the result is `NULL`.
    let passwd = unsafe { libc::getpwnam(name.as_ptr()) };
    if passwd.is_null() {
        return Err(Error::Config(format!(
            "no such user: {user}; the unit names {user} as the one the service runs as"
        )));
    }
    // SAFETY: `passwd` is valid while the result is used; `path` is NUL-free because a path
    // from this crate never holds a NUL, and it is alive for the call.
    let uid = unsafe { (*passwd).pw_uid };
    let gid = unsafe { (*passwd).pw_gid };
    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|_| Error::Config("the unit path contains a NUL".to_owned()))?;
    // SAFETY: `path` is NUL-terminated and alive for the call.
    if unsafe { libc::chown(path.as_ptr(), uid, gid) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

/// Hand one file over to `user`, on Linux only — nothing to do elsewhere.
#[cfg(not(target_os = "linux"))]
fn hand_over_to_user(_path: &Path, _user: &str) -> Result<()> {
    Ok(())
}

/// What a [`RecordingManager`] was asked to do, in order.
#[derive(Clone, Debug, Default)]
pub struct Calls {
    /// Every unit file written, with the body it was written with.
    pub units: Vec<(PathBuf, String)>,
    /// Every command run, each as its program and arguments.
    pub commands: Vec<Vec<String>>,
    /// Every unit file removed.
    pub removed: Vec<PathBuf>,
}

/// A service manager that records instead of acting.
///
/// It answers from canned output and never touches the machine, which is what lets the
/// install flows be tested on a laptop without enrolling it in a service. [`failing_on`]
/// makes one command fail, so the failure paths — a half-written unit, an uninstall of
/// something absent — are testable too.
///
/// [`failing_on`]: RecordingManager::failing_on
#[derive(Debug, Default)]
pub struct RecordingManager {
    calls: Mutex<Calls>,
    fail_on: Option<String>,
    output: String,
    order: Option<Arc<Mutex<Vec<String>>>>,
}

impl RecordingManager {
    /// A manager that succeeds at everything and answers with empty output.
    pub fn new() -> Self {
        Self::default()
    }

    /// A manager whose commands containing `needle` fail.
    ///
    /// The needle is matched against the command's own words, so `failing_on("enable")`
    /// fails the `enable --now` command alone and leaves `daemon-reload` working.
    pub fn failing_on(needle: &str) -> Self {
        Self {
            fail_on: Some(needle.to_owned()),
            ..Self::default()
        }
    }

    /// A manager that answers every command with `text`.
    pub fn returning(text: &str) -> Self {
        Self {
            output: text.to_owned(),
            ..Self::default()
        }
    }

    /// A manager that also appends to `order` as it goes: one entry per unit file written
    /// and per command run, so a test can compare the install flow's own steps against
    /// steps a post-install hook records into the same list.
    pub fn ordered(order: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            order: Some(order),
            ..Self::default()
        }
    }

    /// Everything recorded so far.
    pub fn calls(&self) -> Calls {
        self.calls
            .lock()
            .expect("the recording manager's lock is never held across a panic")
            .clone()
    }

    /// Whether a command is the one asked to fail.
    fn fails(&self, command: &[String]) -> bool {
        match &self.fail_on {
            Some(needle) => command.iter().any(|word| word == needle),
            None => false,
        }
    }
}

impl ServiceManager for RecordingManager {
    fn write_unit(&self, path: &Path, body: &str) -> Result<()> {
        self.calls
            .lock()
            .expect("the recording manager's lock is never held across a panic")
            .units
            .push((path.to_owned(), body.to_owned()));
        if let Some(order) = &self.order {
            order
                .lock()
                .expect("the shared order list's lock is never held across a panic")
                .push(format!("write_unit {}", path.display()));
        }
        Ok(())
    }

    fn run(&self, command: &[String]) -> Result<String> {
        // Record first: a command that failed was still run, and a test asking what was
        // attempted wants to see it.
        self.calls
            .lock()
            .expect("the recording manager's lock is never held across a panic")
            .commands
            .push(command.to_vec());
        if let Some(order) = &self.order {
            order
                .lock()
                .expect("the shared order list's lock is never held across a panic")
                .push(format!("run {}", command.join(" ")));
        }
        if self.fails(command) {
            return Err(Error::Manager(format!("refused: {}", command.join(" "))));
        }
        Ok(self.output.clone())
    }

    fn remove_unit(&self, path: &Path) -> Result<()> {
        self.calls
            .lock()
            .expect("the recording manager's lock is never held across a panic")
            .removed
            .push(path.to_owned());
        Ok(())
    }
}

/// Which kind of install a `service` invocation asks for.
#[derive(Clone, Debug, clap::Subcommand)]
pub enum ServiceCommand {
    /// Install the service and start it.
    Install(InstallArgs),
    /// Stop, disable and remove the service.
    Uninstall,
    /// Report what the service manager says about the service.
    Status,
}

/// The flags an install accepts.
///
/// The scope flags are also what an uninstall or a status uses to decide which unit it
/// means, so all three take the same struct: an install run as a regular user and an
/// uninstall run the same way address the same user unit.
#[derive(Clone, Debug, Default, clap::Args)]
pub struct InstallArgs {
    /// Install a user unit, in the invoking user's own configuration.
    #[arg(long)]
    pub user: bool,

    /// Install a system unit, in `/etc/systemd/system`.
    #[arg(long)]
    pub system: bool,

    /// The user a system unit runs as, when it is not the invoking one.
    #[arg(long, value_name = "USER")]
    pub run_as: Option<String>,

    /// Leave the application's own post-install hook to the user.
    #[arg(long)]
    pub keep_helper: bool,
}

impl InstallArgs {
    /// The scope the flags ask for, if they ask for one.
    ///
    /// Asking for both is refused here rather than left to the caller: they name two
    /// different units, and quietly picking one would install something nobody asked for.
    fn requested_scope(&self) -> Result<Option<Scope>> {
        match (self.user, self.system) {
            (true, true) => Err(Error::Config(
                "--user and --system name two different units; pass at most one".to_owned(),
            )),
            (true, false) => Ok(Some(Scope::User)),
            (false, true) => Ok(Some(Scope::System)),
            (false, false) => Ok(None),
        }
    }
}

/// Install a service: decide the scope and the target user, write the platform's file,
/// activate it, and run the spec's post-install hook on what was installed.
///
/// The unit's `ExecStart` (or its platform's equivalent) is the absolute path of the
/// running executable — the app that called this — plus the spec's own arguments, because a
/// service manager starts one file and nothing else.
///
/// The hook runs last, with the [`InstallContext`] the install resolved, so what it sees is
/// what was installed, not what was asked for. It may skip out of it: an install with
/// `--keep-helper` never runs the hook, leaving the file the application's configuration
/// names to whoever wrote that configuration. When the hook fails, the install reports the
/// hook's own error and that the service is installed and running — the files exist and the
/// manager has started the service, which is exactly why a failed hook should be able to
/// wait for a fix rather than undo an otherwise good install.
///
/// On a Linux system install that resolved to a named user, the unit file is handed to that
/// user before the hook runs: files the hook writes into their directories are theirs, not
/// root's. An app that drops privileges itself ([`RunAs::Root`] or privilege separation)
/// resolves to no user, so nothing changes hands.
pub fn install(
    spec: &ServiceSpec,
    args: &InstallArgs,
    env: &Environment,
    manager: &dyn ServiceManager,
) -> Result<InstallContext> {
    let scope = resolve_scope(env, args.requested_scope()?)?;
    let target_user = resolve_target_user(env, scope, spec, args.run_as.as_deref())?;
    let context = InstallContext {
        scope,
        target_user,
        unit_path: install_path(spec, scope, env)?,
        exe: std::env::current_exe()?,
    };

    let body = render_install(spec, &context, env.os);
    manager.write_unit(&context.unit_path, &body)?;

    for command in install_commands(spec, &context, env) {
        if let Err(error) = manager.run(&command) {
            if let Err(cleanup) = manager.remove_unit(&context.unit_path) {
                return Err(Error::Manager(format!(
                    "{error}; the partially written unit at {} could not be removed either \
                     ({cleanup})",
                    context.unit_path.display()
                )));
            }
            return Err(error);
        }
    }

    if env.os == Os::Windows {
        // The XML was a hand-over: `/create` copies the task into the scheduler, and the
        // copy left under the temporary directory has done its job. Failing an install that
        // succeeded over a leftover hand-over file would be the wrong trade.
        let _ = manager.remove_unit(&context.unit_path);
    }

    // The hook runs on an installed, running service, with the scope and the user the
    // install resolved. `--keep-helper` leaves the file the application's configuration
    // names to whoever wrote that configuration.
    if let Some(hook) = spec.post_install.as_ref().filter(|_| !args.keep_helper) {
        // A hook that writes into the target user's directories writes over files root
        // owns; handing the unit file over first is what makes those writes the user's.
        // Three conditions, two lines: the flat form hides them.
        #[allow(clippy::collapsible_if)]
        if let Some(user) = &context.target_user {
            if env.os == Os::Linux && scope == Scope::System {
                manager.chown_to_user(&context.unit_path, user)?;
            }
        }
        if let Err(error) = hook(&context) {
            return Err(Error::Manager(format!(
                "{error}; the service is installed and running, so fix what the hook needs \
                 and rerun the uninstall and install of your choice"
            )));
        }
    }

    Ok(context)
}

/// Uninstall a service: stop and disable it, then remove its file.
///
/// Idempotent by design: a service that was never installed, or was already removed, is not
/// an error. Stopping or disabling something the manager does not know about is exactly the
/// state an uninstall wants to reach, so those failures say nothing worth reporting; a
/// failure to remove the file does, and is returned.
pub fn uninstall(
    spec: &ServiceSpec,
    args: &InstallArgs,
    env: &Environment,
    manager: &dyn ServiceManager,
) -> Result<()> {
    let scope = resolve_scope(env, args.requested_scope()?)?;
    let path = install_path(spec, scope, env)?;

    for command in uninstall_commands(spec, scope, env) {
        // Idempotence: "not running" and "not enabled" are what we came for.
        let _ = manager.run(&command);
    }
    manager.remove_unit(&path)
}

/// Report what the service manager says about the service.
pub fn status(
    spec: &ServiceSpec,
    args: &InstallArgs,
    env: &Environment,
    manager: &dyn ServiceManager,
) -> Result<String> {
    let scope = resolve_scope(env, args.requested_scope()?)?;
    manager.run(&status_command(spec, scope, env))
}

/// Where a platform keeps the file an install writes.
///
/// Windows is the odd one out: a scheduled task is registered from XML and the scheduler
/// keeps its own copy, so the file named here is a hand-over on its way into the
/// registration, not the task itself.
///
/// The home directory is read only where it is used: a system unit lives in `/etc` whatever
/// `HOME` says, and a scheduled task's hand-over lives in the temporary directory, so
/// neither should fail for want of a variable it never consults.
fn install_path(spec: &ServiceSpec, scope: Scope, env: &Environment) -> Result<PathBuf> {
    match (env.os, scope) {
        // A system unit lives in `/etc` whatever `HOME` says.
        (Os::Linux, Scope::System) => {
            Ok(unit_path(spec.app_name, scope, Path::new("/nonexistent")))
        }
        (Os::Linux, Scope::User) => Ok(unit_path(spec.app_name, scope, &home_dir(env)?)),
        // macOS is user level only, so it always needs the home directory.
        (Os::MacOs, _) => Ok(agent_path(spec.app_name, &home_dir(env)?)),
        (Os::Windows, _) => Ok(std::env::temp_dir().join(format!("{}-task.xml", spec.app_name))),
    }
}

/// Render the platform's file for a resolved install.
fn render_install(spec: &ServiceSpec, context: &InstallContext, os: Os) -> String {
    match os {
        Os::Linux => render_unit(spec, context),
        Os::MacOs => render_launch_agent(spec, context),
        Os::Windows => render_task(spec, context),
    }
}

/// A command line built from its words.
fn words(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

/// A `systemctl` command: a user unit goes through the invoking user's own manager.
fn systemctl_command(scope: Scope, tail: &[&str]) -> Vec<String> {
    let scope_flag: &[&str] = match scope {
        Scope::User => &["--user"],
        Scope::System => &[],
    };
    std::iter::once("systemctl")
        .chain(scope_flag.iter().copied())
        .chain(tail.iter().copied())
        .map(str::to_owned)
        .collect()
}

/// The commands that activate what was just installed.
fn install_commands(
    spec: &ServiceSpec,
    context: &InstallContext,
    env: &Environment,
) -> Vec<Vec<String>> {
    match env.os {
        Os::Linux => activation(spec.app_name, context.scope),
        // `gui/<uid>` is the agent session of the user doing the install; a LaunchAgent
        // belongs to them.
        Os::MacOs => vec![words(&[
            "launchctl",
            "bootstrap",
            &format!("gui/{}", env.euid),
            &context.unit_path.display().to_string(),
        ])],
        // The task name is the app name — the same key an uninstall and a status address.
        Os::Windows => vec![words(&[
            "schtasks",
            "/create",
            "/tn",
            spec.app_name,
            "/xml",
            &context.unit_path.display().to_string(),
            "/f",
        ])],
    }
}

/// The commands that stop and disable an installed service.
fn uninstall_commands(spec: &ServiceSpec, scope: Scope, env: &Environment) -> Vec<Vec<String>> {
    match env.os {
        Os::Linux => vec![
            systemctl_command(scope, &["stop", spec.app_name]),
            systemctl_command(scope, &["disable", spec.app_name]),
        ],
        Os::MacOs => vec![words(&[
            "launchctl",
            "bootout",
            &format!("gui/{}", env.euid),
            &label(spec.app_name),
        ])],
        Os::Windows => vec![words(&["schtasks", "/delete", "/tn", spec.app_name, "/f"])],
    }
}

/// The command that asks the manager about a service.
fn status_command(spec: &ServiceSpec, scope: Scope, env: &Environment) -> Vec<String> {
    match env.os {
        // `--no-pager`: the answer is captured, not read off a terminal.
        Os::Linux => systemctl_command(scope, &["--no-pager", "status", spec.app_name]),
        Os::MacOs => words(&[
            "launchctl",
            "print",
            &format!("gui/{}/{}", env.euid, label(spec.app_name)),
        ]),
        Os::Windows => words(&["schtasks", "/query", "/tn", spec.app_name]),
    }
}

/// The invoking user's home directory, where a user-level install writes.
///
/// Read from the environment for the platform in hand, so a Windows install looks at
/// `USERPROFILE` and everything else at `HOME`. A user unit has nowhere to go without it,
/// which is worth saying rather than guessing a path.
fn home_dir(env: &Environment) -> Result<PathBuf> {
    let variable = match env.os {
        Os::Windows => "USERPROFILE",
        Os::Linux | Os::MacOs => "HOME",
    };
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| {
            Error::Config(format!(
                "{variable} is not set; there is nowhere to write a user unit"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::RunAs;
    use std::sync::atomic::{AtomicBool, Ordering};

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

    fn linux_user() -> Environment {
        Environment {
            euid: 1000,
            sudo_user: None,
            os: Os::Linux,
        }
    }

    fn linux_root() -> Environment {
        Environment {
            euid: 0,
            sudo_user: Some("alice".into()),
            os: Os::Linux,
        }
    }

    /// The brief's privilege helper, against this crate's own `privilege` types: they moved
    /// here from `-server` in Task 1, so `-server` is not part of a unit file's vocabulary.
    fn privileges_for(run_as: &str, helper: &str) -> crate::privilege::PrivilegeConfig {
        crate::privilege::PrivilegeConfig {
            run_as: run_as.parse().unwrap(),
            helper: Some(crate::privilege::HelperSpec {
                user: helper.parse().unwrap(),
                program: std::path::PathBuf::from("/usr/lib/sapphire-agent/tool-broker"),
                args: vec![],
            }),
        }
    }

    #[test]
    fn installing_writes_a_unit_and_activates_it() {
        let manager = RecordingManager::default();
        install(&spec(), &InstallArgs::default(), &linux_user(), &manager).unwrap();

        let calls = manager.calls();
        assert_eq!(calls.units.len(), 1);
        assert!(
            calls.units[0].0.ends_with("sapphire-agent.service"),
            "{:?}",
            calls.units[0].0
        );
        assert!(
            calls
                .commands
                .iter()
                .any(|c| c.contains(&"enable".to_owned())),
            "{:?}",
            calls.commands
        );
    }

    #[test]
    fn a_user_install_uses_the_user_flag() {
        let manager = RecordingManager::default();
        install(&spec(), &InstallArgs::default(), &linux_user(), &manager).unwrap();
        assert!(
            manager
                .calls()
                .commands
                .iter()
                .all(|c| c.contains(&"--user".to_owned())),
            "{:?}",
            manager.calls().commands
        );
    }

    #[test]
    fn a_system_install_does_not() {
        let manager = RecordingManager::default();
        install(&spec(), &InstallArgs::default(), &linux_root(), &manager).unwrap();
        assert!(
            manager
                .calls()
                .commands
                .iter()
                .all(|c| !c.contains(&"--user".to_owned())),
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
        let args = InstallArgs {
            user: true,
            system: true,
            ..InstallArgs::default()
        };
        let err = install(&spec(), &args, &linux_root(), &RecordingManager::default()).unwrap_err();
        assert!(err.to_string().contains("--user"), "{err}");
    }

    #[test]
    fn a_macos_install_bootstraps_a_launch_agent() {
        let env = Environment {
            euid: 1000,
            sudo_user: None,
            os: Os::MacOs,
        };
        let manager = RecordingManager::default();
        install(&spec(), &InstallArgs::default(), &env, &manager).unwrap();

        let calls = manager.calls();
        assert_eq!(calls.units.len(), 1);
        assert!(
            calls.units[0].0.to_string_lossy().ends_with(".plist"),
            "a LaunchAgent is a plist: {:?}",
            calls.units[0].0
        );
        let flat: Vec<String> = calls.commands.iter().flatten().cloned().collect();
        assert!(flat.contains(&"launchctl".to_owned()), "{flat:?}");
        assert!(flat.contains(&"bootstrap".to_owned()), "{flat:?}");
    }

    #[test]
    fn a_windows_install_registers_a_scheduled_task() {
        let env = Environment {
            euid: 1000,
            sudo_user: None,
            os: Os::Windows,
        };
        let manager = RecordingManager::default();
        install(&spec(), &InstallArgs::default(), &env, &manager).unwrap();

        let flat: Vec<String> = manager.calls().commands.iter().flatten().cloned().collect();
        assert!(flat.contains(&"schtasks".to_owned()), "{flat:?}");
        assert!(flat.contains(&"/create".to_owned()), "{flat:?}");
    }

    #[test]
    fn a_system_install_reads_no_home_directory() {
        // A system unit lives in `/etc` whatever `HOME` says, so an install must not fail
        // for want of a variable it never uses.
        let path = install_path(&spec(), Scope::System, &linux_root()).unwrap();
        assert_eq!(
            path,
            std::path::PathBuf::from("/etc/systemd/system/sapphire-agent.service")
        );
    }

    #[test]
    fn no_test_touches_the_real_service_manager() {
        // Stated as a test so the intent is visible where it can be read: the real manager
        // is the only type that runs anything, and it is never constructed above.
        //
        // The needle is assembled from two pieces so that counting it does not count its
        // own text; the two remaining occurrences are the type's definition and thethe trait
        // implementation that follows it.
        let source = include_str!("manager.rs");
        let constructions = source.matches(concat!("System", "Manager")).count();
        assert!(
            constructions <= 2,
            "the real manager appears {constructions} times; tests must use RecordingManager"
        );
    }

    #[test]
    fn post_install_runs_after_activation() {
        let order = Arc::new(Mutex::new(Vec::<String>::new()));
        let recorded = Arc::clone(&order);
        let mut spec = spec();
        spec.post_install = Some(Box::new(move |_| {
            recorded.lock().unwrap().push("post_install".to_owned());
            Ok(())
        }));

        let manager = RecordingManager::ordered(Arc::clone(&order));
        install(&spec, &InstallArgs::default(), &linux_user(), &manager).unwrap();

        let order = order.lock().unwrap().clone();
        assert_eq!(
            order.last().map(String::as_str),
            Some("post_install"),
            "{order:?}"
        );
    }

    #[test]
    fn post_install_sees_the_resolved_target_user() {
        let seen = Arc::new(Mutex::new(Option::<String>::None));
        let recorded = Arc::clone(&seen);
        let mut spec = spec();
        spec.post_install = Some(Box::new(move |ctx| {
            *recorded.lock().unwrap() = ctx.target_user.clone();
            Ok(())
        }));

        install(
            &spec,
            &InstallArgs::default(),
            &linux_root(),
            &RecordingManager::default(),
        )
        .unwrap();
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

        let args = InstallArgs {
            keep_helper: true,
            ..InstallArgs::default()
        };
        install(&spec, &args, &linux_user(), &RecordingManager::default()).unwrap();
        assert!(!ran.load(Ordering::Relaxed));
    }

    #[test]
    fn a_failing_post_install_fails_the_install_and_says_what_was_done() {
        let mut spec = spec();
        spec.post_install = Some(Box::new(|_| Err(Error::Config("no room".into()))));

        let err = install(
            &spec,
            &InstallArgs::default(),
            &linux_user(),
            &RecordingManager::default(),
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("no room"), "{message}");
        assert!(
            message.contains("installed"),
            "the service is installed and running; say so rather than leaving it ambiguous: \
            {message}"
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
}
