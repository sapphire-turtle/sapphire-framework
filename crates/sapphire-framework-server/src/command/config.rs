//! `<app> config` (#314): this application's settings and the bridge's, by key.
//!
//! With the app server running, every command goes to it over the app connection
//! (`config.*`, which it answers merged with the bridge's), and a write reports which
//! process awaits a restart; `--restart` restarts them, the bridge first. With no server
//! running, the command reads and writes this application's files itself, with the same
//! checks; the bridge's keys then need `sapphire-bridge config`.
//!
//! See `docs/superpowers/specs/2026-10-22-app-settings-design.md`, "`<app> config`".

use std::sync::Arc;
use std::time::Duration;

use sapphire_backend::protocol as proto;
use sapphire_bridge_client::BridgeClient;
use sapphire_framework_settings::{Op, Process, Registry, Scope, SettingDef, apply, get, list};
use sapphire_ipc::{ClientInfo, Endpoint};
use sapphire_workspace::AppContext;

use crate::error::{Error, Result};
use crate::settings::AppSettings;

/// `config …`: the settings, by key.
#[derive(Debug, clap::Subcommand)]
pub enum AppConfigCommand {
    /// List settings and their values.
    List {
        /// Which scope's file to list.
        #[command(flatten)]
        scope: AppScopeFlags,
        /// Say where each value comes from.
        #[arg(long)]
        show_origin: bool,
    },
    /// Show one setting.
    Get {
        /// The setting's key.
        key: String,
    },
    /// Set settings: `key=value`..., or one secret key alone (prompted, or `--stdin`).
    Set {
        /// `key=value` pairs; a secret's key is given alone.
        #[arg(required = true)]
        entries: Vec<String>,
        /// Which scope's file to write (default: each key's own).
        #[command(flatten)]
        scope: AppScopeFlags,
        /// Read a secret from standard input even on a terminal.
        #[arg(long)]
        stdin: bool,
        /// Restart what the change waits for afterwards — the bridge, then this
        /// application's server — and wait until they are back.
        #[arg(long)]
        restart: bool,
    },
    /// Remove settings (or every setting under a prefix), going back to the defaults.
    Unset {
        /// The keys, or prefixes of several keys.
        #[arg(required = true)]
        keys: Vec<String>,
        /// Which scope's file to remove from (default: each key's own).
        #[command(flatten)]
        scope: AppScopeFlags,
        /// Restart what the change waits for afterwards — the bridge, then this
        /// application's server — and wait until they are back.
        #[arg(long)]
        restart: bool,
    },
}

/// The scope flags of an application's `config`: at most one of them.
#[derive(Debug, Clone, Copy, clap::Args)]
pub struct AppScopeFlags {
    /// This device's file.
    #[arg(long, conflicts_with_all = ["workgroup", "workspace"])]
    device: bool,
    /// The workgroup's file (the bridge's settings only).
    #[arg(long, conflicts_with = "workspace")]
    workgroup: bool,
    /// The selected workspace's file (this application's settings only).
    #[arg(long)]
    workspace: bool,
}

impl AppScopeFlags {
    /// The scope the flags name; `None` when none is given.
    pub fn scope(&self) -> Option<Scope> {
        if self.device {
            Some(Scope::Device)
        } else if self.workgroup {
            Some(Scope::Workgroup)
        } else if self.workspace {
            Some(Scope::Workspace)
        } else {
            None
        }
    }
}

/// What `list` prints under the server's keys when the bridge's are missing.
const BRIDGE_NOT_SHOWN: &str = "sapphire-bridge is not running; its settings are not shown";

/// How long `--restart` waits for the app server to answer again.
const RESTART_TIMEOUT: Duration = Duration::from_secs(30);

/// How often it looks.
const RESTART_POLL: Duration = Duration::from_millis(100);

/// `config …`: through the running server, or — when none runs — on this application's
/// files, with `owner`'s settings beside the framework's.
pub(super) async fn config<A: AppSettings + Clone>(
    ctx: &'static AppContext,
    owner: A,
    command: AppConfigCommand,
    version: &str,
) -> Result<i32> {
    let app = ctx.app_name;
    let Some(client) = super::try_connect_app(app, version).await? else {
        // Only `list` speaks of the bridge, and only to say whether it runs.
        let bridge_running = matches!(command, AppConfigCommand::List { .. })
            && sapphire_ipc::probe(&Endpoint::for_bridge()?)
                .await
                .unwrap_or(false);
        let printed = offline(ctx, Arc::new(owner), command, bridge_running)?;
        for line in printed.out {
            println!("{line}");
        }
        for line in printed.err {
            eprintln!("{line}");
        }
        return Ok(printed.code);
    };
    online(Arc::new(client), app, version, command).await
}

// ── online ──────────────────────────────────────────────────────────────────

/// A write's result, or — when the batch was rejected and its keys named — `None` after
/// printing the error and one `{key}: {message}` line per key.
fn rejected_batch<T>(result: sapphire_ipc::Result<T>) -> Option<sapphire_ipc::Result<T>> {
    use sapphire_bridge_client::ConfigFieldErrorsExt;
    if let Err(err @ sapphire_ipc::Error::Rpc(rpc)) = &result
        && let Some(fields) = sapphire_bridge_api::ConfigFieldErrors::from_rpc(rpc)
    {
        eprintln!("{err}");
        for line in fields.lines() {
            eprintln!("{line}");
        }
        return None;
    }
    Some(result)
}

/// `config …` over `client`, the app server's connection.
async fn online(
    client: Arc<sapphire_ipc::Client>,
    app: &str,
    version: &str,
    command: AppConfigCommand,
) -> Result<i32> {
    let bridge = BridgeClient::relayed(Arc::clone(&client), sapphire_ipc::runtime_dir()?);
    let (written, restart, schema) = match command {
        AppConfigCommand::List { scope, show_origin } => {
            let entries = bridge.config_list(scope.scope()).await?.entries;
            for line in sapphire_bridge_api::describe_entries(&entries, show_origin) {
                println!("{line}");
            }
            // The workspace scope is the server's alone: nothing of the bridge's is missing.
            if scope.scope() != Some(Scope::Workspace)
                && !has_bridge(&bridge.config_schema().await?.settings)
            {
                println!("{BRIDGE_NOT_SHOWN}");
            }
            return Ok(0);
        }
        AppConfigCommand::Get { key } => {
            let lines = match bridge.config_get(&key).await {
                Ok(entry) => sapphire_bridge_api::describe_entries(&[entry], false),
                Err(sapphire_ipc::Error::NotRunning(_)) => {
                    eprintln!(
                        "unknown setting {key} (or sapphire-bridge, which may know it, is not running)"
                    );
                    return Ok(1);
                }
                // On stderr: `$(<app> config get k)` must not take it for the value.
                Err(err) => {
                    eprintln!("{err}");
                    return Ok(1);
                }
            };
            for line in lines {
                println!("{line}");
            }
            return Ok(0);
        }
        AppConfigCommand::Set {
            entries,
            scope,
            stdin,
            restart,
        } => {
            let schema = bridge.config_schema().await?.settings;
            // Checked here, not left to the server: `parse_entries` needs each key's kind to
            // read `key=value`, so a key missing from the schema would fail as unknown before
            // the server could say that the bridge, which may know it, is not running.
            if !has_bridge(&schema)
                && let Some(key) = entries
                    .iter()
                    .map(|e| e.split_once('=').map_or(e.as_str(), |(key, _)| key))
                    .find(|key| !schema.iter().any(|d| d.key == *key))
            {
                return Err(Error::Config(format!(
                    "unknown setting {key} (or sapphire-bridge, which may know it, is not running)"
                )));
            }
            let entries = sapphire_bridge_api::parse_entries(&schema, &entries, stdin)
                .map_err(Error::Config)?;
            (
                {
                    let result = bridge.config_set(scope.scope(), entries).await;
                    let Some(result) = rejected_batch(result) else {
                        return Ok(1);
                    };
                    result?
                },
                restart,
                Some(schema),
            )
        }
        AppConfigCommand::Unset {
            keys,
            scope,
            restart,
        } => (
            {
                let result = bridge.config_unset(scope.scope(), keys).await;
                let Some(result) = rejected_batch(result) else {
                    return Ok(1);
                };
                result?
            },
            restart,
            None,
        ),
    };

    // Which process each pending key waits on, through the schema; a key it does not list
    // is the bridge's, as an older bridge's definitions are.
    let schema = match schema {
        Some(schema) => schema,
        None => bridge.config_schema().await?.settings,
    };
    let waits_on = |process: Process| {
        written.pending_restart.iter().any(|key| {
            schema
                .iter()
                .find(|d| d.key == *key)
                .map_or(Process::Bridge, |d| d.process)
                == process
        })
    };
    let (bridge_waits, app_waits) = (waits_on(Process::Bridge), waits_on(Process::App));
    if !restart {
        if bridge_waits {
            println!("restart required: sapphire-bridge");
        }
        if app_waits {
            println!("restart required: {app} server");
        }
        return Ok(0);
    }
    if !bridge_waits && !app_waits {
        println!("nothing awaits a restart");
        return Ok(0);
    }
    // The bridge first: the app server's restart would only reconnect to the old one.
    if bridge_waits {
        // Relayed, the call returns once the new bridge answers.
        bridge.restart().await?;
        println!("sapphire-bridge restarted");
    }
    if app_waits {
        restart_app_server(&client, app, version).await?;
        println!("the {app} server restarted");
    }
    Ok(0)
}

/// Whether `schema` holds any of the bridge's settings: it does whenever the bridge
/// answered the server.
fn has_bridge(schema: &[SettingDef]) -> bool {
    schema.iter().any(|d| d.process == Process::Bridge)
}

/// Ask the app server `client` is connected to to restart, and wait until it is back.
///
/// "Back" is two facts in order: the connection that asked has closed — the old server
/// stopped serving it — and a new connection to the endpoint answers `server.info`.
async fn restart_app_server(client: &sapphire_ipc::Client, app: &str, version: &str) -> Result<()> {
    let _: proto::Ack = client
        .call(proto::SERVER_RESTART, serde_json::json!({}))
        .await?;
    let endpoint = Endpoint::for_app(app)?;
    let came_back = async {
        client.closed().await;
        loop {
            if answers(&endpoint, app, version).await {
                return;
            }
            tokio::time::sleep(RESTART_POLL).await;
        }
    };
    tokio::time::timeout(RESTART_TIMEOUT, came_back)
        .await
        .map_err(|_| {
            Error::Config(format!(
                "the {app} server did not come back within {} s",
                RESTART_TIMEOUT.as_secs()
            ))
        })
}

/// Whether the server at `endpoint` answers `server.info` on a new connection.
async fn answers(endpoint: &Endpoint, app: &str, version: &str) -> bool {
    let info = ClientInfo {
        kind: "cli".to_owned(),
        version: version.to_owned(),
        api: proto::API_VERSION,
        app_api: None,
        pid: std::process::id(),
    };
    let Ok(Some((client, _))) = sapphire_ipc::connect_or_absent(endpoint, app, info).await else {
        return false;
    };
    client
        .call::<_, proto::StatusReport>(proto::SERVER_INFO, serde_json::json!({}))
        .await
        .is_ok()
}

// ── offline ─────────────────────────────────────────────────────────────────

/// What an offline `config` command prints, and its exit code.
#[derive(Debug)]
struct Printed {
    /// The exit code.
    code: i32,
    /// Lines for stdout: the answer.
    out: Vec<String>,
    /// Lines for stderr: why there is no answer.
    err: Vec<String>,
}

impl Printed {
    fn ok(out: Vec<String>) -> Printed {
        Printed {
            code: 0,
            out,
            err: Vec::new(),
        }
    }

    fn failed(err: String) -> Printed {
        Printed {
            code: 1,
            out: Vec::new(),
            err: vec![err],
        }
    }
}

/// Carry out `command` on `ctx`'s application's files, with `owner`'s settings beside the
/// framework's: what to print, and the exit code.
///
/// The files are opened as the server opens them — `server.toml` migrated first — but the
/// checks a starting server runs do not stop the command, so it can repair a file the
/// server refuses: `list` and `get` answer and add the checks' error as a warning, and a
/// write is accepted when the edited files pass the owners' checks. A file that does not
/// parse at all is an error naming it. `bridge_running` says which line `list` ends with.
fn offline(
    ctx: &'static AppContext,
    owner: Arc<dyn AppSettings>,
    command: AppConfigCommand,
    bridge_running: bool,
) -> Result<Printed> {
    let app = ctx.app_name;
    // Checked before anything is written: the user asked for a restart, and a write that
    // took effect with none would be a surprise.
    if let AppConfigCommand::Set { restart: true, .. }
    | AppConfigCommand::Unset { restart: true, .. } = command
    {
        return Ok(Printed::failed(format!(
            "the {app} server is not running; nothing to restart"
        )));
    }
    let (reg, checked) = crate::settings::open_unchecked(ctx, owner)?;
    let reg = &reg;
    // Read-only commands answer anyway, saying what a starting server would refuse.
    let warning = match (&command, checked) {
        (AppConfigCommand::List { .. } | AppConfigCommand::Get { .. }, Err(e)) => {
            Some(format!("warning: the {app} server would not start: {e}"))
        }
        _ => None,
    };
    let warned = |mut printed: Printed| {
        printed.err.splice(0..0, warning.clone());
        printed
    };
    let bridge_key = |key: &str| {
        Printed::failed(format!(
            "the {app} server is not running; start it, or use sapphire-bridge config for {key}"
        ))
    };
    let written = || {
        Ok(Printed::ok(vec![format!(
            "the {app} server is not running; it applies these settings when it starts"
        )]))
    };
    match command {
        AppConfigCommand::List { scope, show_origin } => {
            // The workgroup scope is the bridge's alone.
            let entries = match scope.scope() {
                Some(Scope::Workgroup) => Vec::new(),
                scope => list(reg, scope, None)?,
            };
            let mut out = sapphire_bridge_api::describe_entries(&entries, show_origin);
            // The bridge's settings reach this command through the server alone.
            if scope.scope() != Some(Scope::Workspace) {
                out.push(if bridge_running {
                    format!("start the {app} server to see sapphire-bridge's settings")
                } else {
                    BRIDGE_NOT_SHOWN.to_owned()
                });
            }
            Ok(warned(Printed::ok(out)))
        }
        AppConfigCommand::Get { key } => {
            if reg.def(&key).is_err() {
                return Ok(warned(bridge_key(&key)));
            }
            Ok(warned(match get(reg, &key) {
                Ok(entry) => Printed::ok(sapphire_bridge_api::describe_entries(&[entry], false)),
                Err(err) => Printed::failed(err.to_string()),
            }))
        }
        AppConfigCommand::Set {
            entries,
            scope,
            stdin,
            restart: _,
        } => {
            let keys = entries
                .iter()
                .map(|e| e.split_once('=').map_or(e.as_str(), |(key, _)| key));
            if let Some(key) = keys.into_iter().find(|key| !own(reg, key)) {
                return Ok(bridge_key(key));
            }
            let entries = sapphire_bridge_api::parse_entries(&reg.schema(), &entries, stdin)
                .map_err(Error::Config)?;
            let ops = entries
                .into_iter()
                .map(|e| Op::Set {
                    key: e.key,
                    value: e.value,
                })
                .collect();
            apply(reg, scope.scope(), ops)?;
            written()
        }
        AppConfigCommand::Unset { keys, scope, .. } => {
            if let Some(key) = keys.iter().find(|key| !own(reg, key)) {
                return Ok(bridge_key(key));
            }
            let ops = keys.into_iter().map(|key| Op::Unset { key }).collect();
            apply(reg, scope.scope(), ops)?;
            written()
        }
    }
}

/// Whether `key` is this server's: a key its registry knows, or a prefix of keys it knows.
fn own(reg: &Registry, key: &str) -> bool {
    reg.def(key).is_ok() || !reg.under(key).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Probe {
        #[command(subcommand)]
        command: AppConfigCommand,
    }

    fn parse(args: &[&str]) -> std::result::Result<AppConfigCommand, clap::Error> {
        Probe::try_parse_from(std::iter::once("config").chain(args.iter().copied()))
            .map(|p| p.command)
    }

    #[test]
    fn each_scope_flag_names_its_scope() {
        for (flag, scope) in [
            ("--device", Some(Scope::Device)),
            ("--workgroup", Some(Scope::Workgroup)),
            ("--workspace", Some(Scope::Workspace)),
        ] {
            let AppConfigCommand::List { scope: flags, .. } = parse(&["list", flag]).unwrap()
            else {
                panic!("not list");
            };
            assert_eq!(flags.scope(), scope, "{flag}");
        }
        let AppConfigCommand::List { scope, .. } = parse(&["list"]).unwrap() else {
            panic!("not list");
        };
        assert_eq!(scope.scope(), None);
    }

    #[test]
    fn the_scope_flags_exclude_each_other() {
        for pair in [
            ["--device", "--workgroup"],
            ["--device", "--workspace"],
            ["--workgroup", "--workspace"],
        ] {
            assert!(parse(&["list", pair[0], pair[1]]).is_err(), "{pair:?}");
        }
    }

    #[test]
    fn set_and_unset_take_restart() {
        assert!(matches!(
            parse(&["set", "a.b=1", "--restart", "--workspace"]).unwrap(),
            AppConfigCommand::Set { restart: true, .. }
        ));
        assert!(matches!(
            parse(&["unset", "a.b", "--restart"]).unwrap(),
            AppConfigCommand::Unset { restart: true, .. }
        ));
        assert!(parse(&["set"]).is_err(), "set needs an entry");
    }
}
