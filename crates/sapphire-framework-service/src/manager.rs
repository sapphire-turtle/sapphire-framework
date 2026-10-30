//! The install, uninstall and status flows, and the manager they go through.
//!
//! Everything that touches the machine — writing a unit file, running a manager command,
//! removing one — goes through the [`ServiceManager`] trait. The real implementation writes
//! real files and runs real commands; [`RecordingManager`] records what it was asked and
//! answers from canned output, so the flows can be driven by tests that never touch the
//! host's own service manager.
//!
//! [`install`] renders the platform's file ([`crate::systemd`], [`crate::launchd`],
//! [`crate::windows`]), writes it through the manager and runs the activation commands. When
//! activation fails, the half-written unit is removed before the error returns: a unit file
//! that was written but never enabled is invisible to `status` and springs to life at the
//! next reboot. [`uninstall`] is idempotent — stopping, disabling or removing something that
//! is not there is not an error — and [`status`] reports whatever the manager said.
//!
//! Every install is user level: Linux goes through `systemctl --user` with the unit from
//! [`crate::systemd`], macOS through `launchctl` with the LaunchAgent from
//! [`crate::launchd`], Windows through `schtasks` with the task XML from [`crate::windows`].
//! The activation hint for a user unit on a machine that should run it without a login
//! ([`crate::systemd::linger_hint`]) is not carried here: an install returns an
//! [`InstallContext`], and the app CLI asks for the hint itself.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::error::{Error, Result};
use crate::launchd::{
    agent_path, label, legacy_agent_path, legacy_label, relabel, render_launch_agent,
};
use crate::scope::{Environment, InstallContext, Os, ServiceSpec};
use crate::systemd::{activation, linger_hint, render_unit, unit_path, words};
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

    /// Read a unit file; `None` when there is none.
    fn read_unit(&self, path: &Path) -> Result<Option<String>> {
        match std::fs::read_to_string(path) {
            Ok(body) => Ok(Some(body)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
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
    /// Every unit file read.
    pub read: Vec<PathBuf>,
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
    existing: Vec<(PathBuf, String)>,
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

    /// The same manager, answering a read of `path` with `body`, as if the file were there.
    pub fn with_unit(mut self, path: impl Into<PathBuf>, body: &str) -> Self {
        self.existing.push((path.into(), body.to_owned()));
        self
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

    fn read_unit(&self, path: &Path) -> Result<Option<String>> {
        self.calls
            .lock()
            .expect("the recording manager's lock is never held across a panic")
            .read
            .push(path.to_owned());
        Ok(self
            .existing
            .iter()
            .find(|(existing, _)| existing == path)
            .map(|(_, body)| body.clone()))
    }
}

/// Which kind of install a `service` invocation asks for.
#[derive(Clone, Debug, clap::Subcommand)]
pub enum ServiceCommand {
    /// Install the service and start it.
    Install,
    /// Stop, disable and remove the service.
    Uninstall,
    /// Report what the service manager says about the service.
    Status,
}

impl ServiceCommand {
    /// Carry out the command against `manager`, returning the process exit code.
    ///
    /// `env` is the machine this runs on — read by an application's CLI with
    /// [`Environment::detect`] — and `manager` is the real one there, while every test hands
    /// in a [`RecordingManager`] instead. What the user is told is printed here, so
    /// both the app servers' and the bridge's CLIs say the same thing about the same act.
    ///
    /// [`Environment::detect`]: crate::scope::Environment::detect
    pub fn run(
        &self,
        spec: &ServiceSpec,
        env: &Environment,
        manager: &dyn ServiceManager,
    ) -> Result<i32> {
        match self {
            ServiceCommand::Install => {
                let context = install(spec, env, manager)?;
                println!(
                    "installed the {} service ({})",
                    spec.app_name,
                    context.unit_path.display()
                );
                // A user unit dies with its login session; the hint to make it survive one
                // is Linux's own (`loginctl`), so it is only offered there.
                if env.os == Os::Linux
                    && let Some(hint) = linger_hint()
                {
                    println!("{hint}");
                }
                Ok(0)
            }
            ServiceCommand::Uninstall => {
                uninstall(spec, env, manager)?;
                println!("removed the {} service", spec.app_name);
                Ok(0)
            }
            ServiceCommand::Status => {
                let report = status(spec, env, manager)?;
                // Another tool's output, printed as it came: it is what the user asked for.
                print!("{report}");
                if !report.ends_with('\n') {
                    println!();
                }
                Ok(0)
            }
        }
    }
}

/// Install a service: write the platform's file, activate it, and run the spec's
/// post-install hook on what was installed.
///
/// The unit's `ExecStart` (or its platform's equivalent) is the absolute path of the
/// running executable — the app that called this — plus the spec's own arguments, because a
/// service manager starts one file and nothing else.
///
/// The hook runs last, with the [`InstallContext`] the install resolved, so what it sees is
/// what was installed. When the hook fails, the install reports the hook's own error and
/// that the service is installed and running — the files exist and the manager has started
/// the service, which is exactly why a failed hook should be able to wait for a fix rather
/// than undo an otherwise good install.
pub fn install(
    spec: &ServiceSpec,
    env: &Environment,
    manager: &dyn ServiceManager,
) -> Result<InstallContext> {
    retire_legacy_agent(spec, env, manager)?;
    let context = InstallContext {
        unit_path: install_path(spec, env)?,
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
        // copy left under the temporary directory has done its job.
        let _ = manager.remove_unit(&context.unit_path);
    }
    // The hook runs last, on an installed, running service, with what the install resolved.
    if let Some(hook) = spec.post_install.as_ref()
        && let Err(error) = hook(&context)
    {
        return Err(Error::Manager(format!(
            "{error}; the service is installed and running, so fix what the hook needs \
             and rerun the uninstall and install of your choice"
        )));
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
    env: &Environment,
    manager: &dyn ServiceManager,
) -> Result<()> {
    retire_legacy_agent(spec, env, manager)?;
    let path = install_path(spec, env)?;

    for command in uninstall_commands(spec, env) {
        // Idempotence: "not running" and "not enabled" are what we came for.
        let _ = manager.run(&command);
    }
    manager.remove_unit(&path)
}

/// Report what the service manager says about the service.
pub fn status(
    spec: &ServiceSpec,
    env: &Environment,
    manager: &dyn ServiceManager,
) -> Result<String> {
    // An agent under the legacy label is the service, renamed: it keeps running, under the
    // current label, unless one is installed there already.
    if let Some(legacy) = retire_legacy_agent(spec, env, manager)? {
        let path = install_path(spec, env)?;
        if manager.read_unit(&path)?.is_none() {
            manager.write_unit(&path, &relabel(&legacy, spec.app_name))?;
            manager.run(&bootstrap_command(&path, env))?;
        }
    }
    manager.run(&status_command(spec, env))
}

/// Take an agent installed under the [`legacy_label`] off launchd and remove its plist,
/// returning what the plist held; `None` off macOS or when there is none.
///
/// Left in place, it would start a second app server next to the one under the current
/// label.
fn retire_legacy_agent(
    spec: &ServiceSpec,
    env: &Environment,
    manager: &dyn ServiceManager,
) -> Result<Option<String>> {
    if env.os != Os::MacOs {
        return Ok(None);
    }
    let path = legacy_agent_path(spec.app_name, &home_dir(env)?);
    let Some(legacy) = manager.read_unit(&path)? else {
        return Ok(None);
    };
    // Not loaded (the user booted it out by hand) is what this wants anyway.
    let _ = manager.run(&words(&[
        "launchctl",
        "bootout",
        &format!("gui/{}", env.euid),
        &legacy_label(spec.app_name),
    ]));
    manager.remove_unit(&path)?;
    Ok(Some(legacy))
}

/// Where each platform keeps the file an install writes: the invoking user's own directory;
/// a Windows task's hand-over lives in the temporary directory.
fn install_path(spec: &ServiceSpec, env: &Environment) -> Result<PathBuf> {
    match env.os {
        Os::Linux => Ok(unit_path(spec.app_name, &home_dir(env)?)),
        Os::MacOs => Ok(agent_path(spec.app_name, &home_dir(env)?)),
        Os::Windows => Ok(std::env::temp_dir().join(format!("{}-task.xml", spec.app_name))),
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

/// The commands that activate what was just installed.
fn install_commands(
    spec: &ServiceSpec,
    context: &InstallContext,
    env: &Environment,
) -> Vec<Vec<String>> {
    match env.os {
        Os::Linux => activation(spec.app_name),
        // `gui/<uid>` is the agent session of the user doing the install; a LaunchAgent
        // belongs to them.
        Os::MacOs => vec![bootstrap_command(&context.unit_path, env)],
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

/// The command that loads a LaunchAgent into the agent session of the user doing the install.
fn bootstrap_command(path: &Path, env: &Environment) -> Vec<String> {
    words(&[
        "launchctl",
        "bootstrap",
        &format!("gui/{}", env.euid),
        &path.display().to_string(),
    ])
}

/// The commands that stop and disable an installed service.
fn uninstall_commands(spec: &ServiceSpec, env: &Environment) -> Vec<Vec<String>> {
    match env.os {
        Os::Linux => vec![
            words(&["systemctl", "--user", "stop", spec.app_name]),
            words(&["systemctl", "--user", "disable", spec.app_name]),
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
fn status_command(spec: &ServiceSpec, env: &Environment) -> Vec<String> {
    match env.os {
        // `--no-pager`: the answer is captured, not read off a terminal.
        Os::Linux => words(&["systemctl", "--user", "--no-pager", "status", spec.app_name]),
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
/// Taken from [`Environment::home`]. A user unit has nowhere to go without it, which is
/// worth saying rather than guessing a path.
fn home_dir(env: &Environment) -> Result<PathBuf> {
    env.home.clone().ok_or_else(|| {
        Error::Config(format!(
            "{} is not set; there is nowhere to write a user unit",
            env.os.home_variable()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ServiceSpec {
        ServiceSpec {
            app_name: "sapphire-agent",
            description: "Sapphire agent server".into(),
            args: vec!["server".into(), "run".into()],
            post_install: None,
        }
    }

    fn linux() -> Environment {
        Environment {
            euid: 1000,
            os: Os::Linux,
            home: Some(PathBuf::from("/home/user")),
        }
    }

    fn macos() -> Environment {
        Environment {
            euid: 1000,
            os: Os::MacOs,
            home: Some(PathBuf::from("/home/user")),
        }
    }

    fn windows() -> Environment {
        Environment {
            euid: 1000,
            os: Os::Windows,
            home: Some(PathBuf::from("/home/user")),
        }
    }

    #[test]
    fn installing_writes_a_unit_and_activates_it() {
        let manager = RecordingManager::default();
        install(&spec(), &linux(), &manager).unwrap();

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
    fn an_install_without_a_home_names_the_missing_variable() {
        for env in [linux(), macos()] {
            let env = Environment { home: None, ..env };
            let manager = RecordingManager::default();
            let err = install(&spec(), &env, &manager).unwrap_err().to_string();
            assert!(err.contains("HOME"), "{err}");
            assert!(
                manager.calls().units.is_empty(),
                "nothing is written without a home"
            );
        }
    }

    #[test]
    fn a_user_install_uses_the_user_flag() {
        // The one install kind always goes through the invoking user's own manager: every
        // command carries `--user`, so nothing here can reach the system's units.
        let manager = RecordingManager::default();
        install(&spec(), &linux(), &manager).unwrap();
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
    fn a_failing_activation_leaves_no_unit_behind() {
        let manager = RecordingManager::failing_on("enable");
        assert!(install(&spec(), &linux(), &manager).is_err());
        assert!(
            !manager.calls().removed.is_empty(),
            "a half-installed service is worse than none: the unit must be cleaned up"
        );
    }

    #[test]
    fn uninstalling_stops_disables_and_removes() {
        let manager = RecordingManager::default();
        uninstall(&spec(), &linux(), &manager).unwrap();

        let calls = manager.calls();
        let flat: Vec<String> = calls.commands.iter().flatten().cloned().collect();
        assert!(flat.contains(&"disable".to_owned()), "{flat:?}");
        assert_eq!(calls.removed.len(), 1);
    }

    #[test]
    fn uninstalling_something_that_is_not_installed_is_not_an_error() {
        let manager = RecordingManager::failing_on("disable");
        uninstall(&spec(), &linux(), &manager).expect("uninstall is idempotent");
    }

    #[test]
    fn status_reports_what_the_manager_said() {
        let manager = RecordingManager::returning("active");
        let text = status(&spec(), &linux(), &manager).unwrap();
        assert!(text.contains("active"), "{text}");
    }

    #[test]
    fn a_macos_install_bootstraps_a_launch_agent() {
        let manager = RecordingManager::default();
        install(&spec(), &macos(), &manager).unwrap();

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

    fn legacy_plist() -> PathBuf {
        PathBuf::from(
            "/home/user/Library/LaunchAgents/net.fireturtle.sapphire.sapphire-agent.plist",
        )
    }

    fn current_plist() -> PathBuf {
        PathBuf::from("/home/user/Library/LaunchAgents/net.sapphire-turtle.sapphire-agent.plist")
    }

    fn legacy_body() -> String {
        "<plist><dict><key>Label</key>\n  <string>net.fireturtle.sapphire.sapphire-agent</string>\n\
         <key>ProgramArguments</key><array><string>/opt/old/sapphire-agent</string></array>\
         </dict></plist>\n"
            .to_owned()
    }

    fn boots_out_legacy(calls: &Calls) -> bool {
        calls.commands.contains(&words(&[
            "launchctl",
            "bootout",
            "gui/1000",
            "net.fireturtle.sapphire.sapphire-agent",
        ]))
    }

    #[test]
    fn a_macos_install_retires_an_agent_under_the_legacy_label() {
        let manager = RecordingManager::new().with_unit(legacy_plist(), &legacy_body());
        install(&spec(), &macos(), &manager).unwrap();

        let calls = manager.calls();
        assert!(boots_out_legacy(&calls), "{:?}", calls.commands);
        assert_eq!(calls.removed, vec![legacy_plist()]);
        assert_eq!(calls.units.len(), 1);
        assert_eq!(calls.units[0].0, current_plist());
    }

    #[test]
    fn a_macos_uninstall_removes_both_labels() {
        let manager = RecordingManager::new().with_unit(legacy_plist(), &legacy_body());
        uninstall(&spec(), &macos(), &manager).unwrap();

        let calls = manager.calls();
        assert!(boots_out_legacy(&calls), "{:?}", calls.commands);
        assert_eq!(calls.removed, vec![legacy_plist(), current_plist()]);
    }

    #[test]
    fn a_macos_status_moves_a_legacy_agent_to_the_current_label() {
        let manager = RecordingManager::new().with_unit(legacy_plist(), &legacy_body());
        status(&spec(), &macos(), &manager).unwrap();

        let calls = manager.calls();
        assert!(boots_out_legacy(&calls), "{:?}", calls.commands);
        assert_eq!(calls.removed, vec![legacy_plist()]);
        assert_eq!(calls.units.len(), 1);
        let (path, body) = &calls.units[0];
        assert_eq!(path, &current_plist());
        assert!(
            body.contains("<string>net.sapphire-turtle.sapphire-agent</string>"),
            "{body}"
        );
        assert!(
            body.contains("/opt/old/sapphire-agent"),
            "the agent keeps its own program: {body}"
        );
        assert!(
            calls.commands.contains(&words(&[
                "launchctl",
                "bootstrap",
                "gui/1000",
                &current_plist().display().to_string(),
            ])),
            "{:?}",
            calls.commands
        );
    }

    #[test]
    fn a_macos_status_leaves_an_agent_already_under_the_current_label() {
        let manager = RecordingManager::new()
            .with_unit(legacy_plist(), &legacy_body())
            .with_unit(current_plist(), "<plist/>");
        status(&spec(), &macos(), &manager).unwrap();

        let calls = manager.calls();
        assert!(boots_out_legacy(&calls), "{:?}", calls.commands);
        assert!(calls.units.is_empty(), "{:?}", calls.units);
        assert!(
            !calls.commands.iter().flatten().any(|w| w == "bootstrap"),
            "{:?}",
            calls.commands
        );
    }

    #[test]
    fn without_a_legacy_agent_nothing_extra_runs() {
        let manager = RecordingManager::new();
        status(&spec(), &macos(), &manager).unwrap();
        let calls = manager.calls();
        assert_eq!(calls.commands.len(), 1, "{:?}", calls.commands);
        assert!(calls.removed.is_empty());

        // Off macOS, there is no legacy plist to look for.
        let manager = RecordingManager::new();
        status(&spec(), &linux(), &manager).unwrap();
        assert!(manager.calls().read.is_empty());
    }

    #[test]
    fn a_windows_install_registers_a_scheduled_task() {
        let manager = RecordingManager::default();
        install(&spec(), &windows(), &manager).unwrap();

        let flat: Vec<String> = manager.calls().commands.iter().flatten().cloned().collect();
        assert!(flat.contains(&"schtasks".to_owned()), "{flat:?}");
        assert!(flat.contains(&"/create".to_owned()), "{flat:?}");
    }

    #[test]
    fn no_test_touches_the_real_service_manager() {
        // Stated as a test so the intent is visible where it can be read: the real manager
        // is the only type that runs anything, and it is never constructed above.
        //
        // The needle is assembled from two pieces so that counting it does not count its
        // own text; the two remaining occurrences are the type's definition and the trait
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
        install(&spec, &linux(), &manager).unwrap();

        let order = order.lock().unwrap().clone();
        assert_eq!(
            order.last().map(String::as_str),
            Some("post_install"),
            "{order:?}"
        );
    }

    #[test]
    fn a_failing_post_install_fails_the_install_and_says_what_was_done() {
        let mut spec = spec();
        spec.post_install = Some(Box::new(|_| Err(Error::Config("no room".into()))));

        let err = install(&spec, &linux(), &RecordingManager::default()).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("no room"), "{message}");
        assert!(
            message.contains("installed"),
            "the service is installed and running; say so rather than leaving it ambiguous: \
            {message}"
        );
    }

    #[test]
    fn running_an_install_command_activates_the_service() {
        let manager = RecordingManager::default();
        let code = ServiceCommand::Install
            .run(&spec(), &linux(), &manager)
            .unwrap();

        assert_eq!(code, 0, "a successful install is a zero exit");
        assert_eq!(manager.calls().units.len(), 1);
    }

    #[test]
    fn running_an_uninstall_command_removes_the_unit() {
        let manager = RecordingManager::default();
        let code = ServiceCommand::Uninstall
            .run(&spec(), &linux(), &manager)
            .unwrap();

        assert_eq!(code, 0);
        assert_eq!(manager.calls().removed.len(), 1);
    }

    #[test]
    fn running_a_status_command_reports_what_the_manager_said() {
        let manager = RecordingManager::returning("active (running)");
        let code = ServiceCommand::Status
            .run(&spec(), &linux(), &manager)
            .unwrap();
        assert_eq!(code, 0, "a status that answered is a zero exit");
    }

    #[test]
    fn a_failing_install_through_the_command_is_an_error() {
        let manager = RecordingManager::failing_on("enable");
        let err = ServiceCommand::Install
            .run(&spec(), &linux(), &manager)
            .unwrap_err();
        assert!(err.to_string().contains("enable"), "{err}");
    }
}
