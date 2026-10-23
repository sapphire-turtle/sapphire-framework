//! Carrying out one [`Command`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use sapphire_backend::protocol as proto;
use sapphire_bridge_api::settings::Process;
use sapphire_bridge_api::{
    ConfigEntry, ConfigFieldErrors, ConfigWriteResult, DevicePrioritySetParams, DeviceRetireParams,
    InviteParams, JoinParams, Scope, WorkgroupCreateParams,
};
use sapphire_bridge_client::{BridgeClient, ConfigFieldErrorsExt};
use sapphire_ipc::Client;

use super::conn::{Connections, drops_app, unanswered};
use super::types::{
    ClientConfig, Command, CommandError, CommandOutput, ConfigState, ServiceTarget,
};

/// How long a restart may take, waiting for the process to come back included.
const RESTART_LIMIT: Duration = Duration::from_secs(45);

/// How long a restarted app server has to answer again.
const APP_RESTART_WAIT: Duration = Duration::from_secs(30);

/// How often a restart looks whether the app server is back.
const APP_RESTART_POLL: Duration = Duration::from_millis(200);

/// How long `command` may take.
fn limit(cfg: &ClientConfig, command: &Command) -> Duration {
    match command {
        // An install may wait on an elevation prompt a person has to answer.
        Command::ServiceInstall(_) => cfg.install_timeout,
        // A join dials peers, so it may take far longer than any other command.
        Command::WorkgroupJoin { .. } => cfg.command_timeout * 3,
        // A restart waits for the process to come back.
        Command::Restart(_) => RESTART_LIMIT,
        _ => cfg.command_timeout,
    }
}

pub(crate) async fn execute(
    cfg: &ClientConfig,
    conns: &mut Connections,
    command: Command,
) -> Result<CommandOutput, CommandError> {
    let limit = limit(cfg, &command);
    match command {
        Command::ServiceInstall(target) => {
            let name = match target {
                ServiceTarget::Bridge => "sapphire-bridge service install",
                ServiceTarget::App => "the app's service install",
            };
            tokio::time::timeout(limit, install(cfg, target))
                .await
                .unwrap_or_else(|_| Err(unanswered(name, limit)))
                .map_err(CommandError::from)
        }
        Command::Restart(Process::App) => {
            match tokio::time::timeout(limit, restart_app(cfg, conns)).await {
                Ok(result) => result,
                Err(_) => {
                    conns.drop_app();
                    Err(unanswered(cfg.app.app_name, limit).into())
                }
            }
        }
        Command::WorkgroupCreate { .. }
        | Command::WorkgroupJoin { .. }
        | Command::DeviceInvite { .. }
        | Command::DeviceRetire { .. }
        | Command::DevicePrioritySet { .. }
        | Command::Embedding(_)
        | Command::ExternalDevice(_)
        | Command::ConfigLoad
        | Command::ConfigWrite { .. }
        | Command::Restart(Process::Bridge) => {
            match tokio::time::timeout(limit, bridge_command(cfg, conns, command)).await {
                Ok(Ok(Ok(out))) => Ok(out),
                // The app server answered, for the bridge: its connection is fine.
                Ok(Ok(Err(e))) if !drops_app(&e) => Err(bridge_failure(e)),
                Ok(Ok(Err(e))) => {
                    conns.drop_app();
                    Err(bridge_failure(e))
                }
                Ok(Err(e)) => {
                    conns.drop_app();
                    Err(e.into())
                }
                Err(_) => {
                    conns.drop_app();
                    Err(unanswered("sapphire-bridge", limit).into())
                }
            }
        }
        _ => {
            let result = tokio::time::timeout(limit, app_command(cfg, conns, command))
                .await
                .unwrap_or_else(|_| Err(unanswered(cfg.app.app_name, limit).into()));
            if result.is_err() {
                conns.drop_app();
            }
            result
        }
    }
}

/// Run a bridge command through the app server: `Err` when the app server cannot be
/// reached, and otherwise what the relayed call came to.
async fn bridge_command(
    cfg: &ClientConfig,
    conns: &mut Connections,
    command: Command,
) -> Result<sapphire_ipc::Result<CommandOutput>, String> {
    let app = cfg.app.app_name;
    conns
        .app(cfg)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("the {app} server is not running"))?;
    let (Some(c), Some(server)) = (conns.bridge(cfg), conns.app_client()) else {
        return Err(format!("the {app} server is not running"));
    };
    let out = match command {
        Command::WorkgroupCreate { name, device_name } => c
            .workgroup_create(WorkgroupCreateParams { name, device_name })
            .await
            .map(|_| CommandOutput::Done),
        Command::WorkgroupJoin {
            ticket,
            device_name,
        } => c
            .join(JoinParams {
                ticket,
                device_name,
            })
            .await
            .map(|_| CommandOutput::Done),
        Command::DeviceInvite { name, ttl_secs } => c
            .invite(InviteParams {
                name,
                ttl: ttl_secs,
                workgroup: None,
            })
            .await
            .map(|r| CommandOutput::Ticket(r.ticket)),
        Command::DeviceRetire { selector } => c
            .device_retire(DeviceRetireParams { selector })
            .await
            .map(|_| CommandOutput::Done),
        Command::DevicePrioritySet { selector, priority } => c
            .device_priority_set(DevicePrioritySetParams { selector, priority })
            .await
            .map(|_| CommandOutput::Done),
        Command::ExternalDevice(request) => {
            c.external_device_request(request)
                .await
                .map(|outcome| match outcome {
                    sapphire_bridge_api::ExternalDeviceOutcome::WithToken(r) => {
                        CommandOutput::Token(r.token.expose().to_owned())
                    }
                    _ => CommandOutput::Done,
                })
        }
        Command::Embedding(request) => c.embed_request(request).await.map(|_| CommandOutput::Done),
        Command::ConfigLoad => config_load(&c, &server).await.map(CommandOutput::Config),
        Command::ConfigWrite { scope, set, unset } => config_write(&c, scope, set, unset)
            .await
            .map(CommandOutput::Written),
        // Relayed, it answers once the app server is connected to the new bridge.
        Command::Restart(Process::Bridge) => c.restart().await.map(|_| CommandOutput::Done),
        _ => unreachable!("execute routes only bridge commands here"),
    };
    Ok(out)
}

/// The settings: the schema, the effective values, and each scope's file — the workspace's
/// only when the server has one selected.
///
/// The relayed calls go out together: with no bridge running, each may wait a while for
/// one, and those waits overlap rather than add up.
async fn config_load(c: &BridgeClient, server: &Client) -> sapphire_ipc::Result<ConfigState> {
    let current: proto::WorkspaceCurrentResult = server
        .call(proto::WORKSPACE_CURRENT, serde_json::json!({}))
        .await?;
    let list = |scope| async move { c.config_list(scope).await.map(|r| r.entries) };
    let workspace = async {
        match current.workspace {
            Some(_) => list(Some(Scope::Workspace)).await.map(Some),
            None => Ok(None),
        }
    };
    let (schema, effective, device, workgroup, workspace) = tokio::try_join!(
        c.config_schema(),
        list(None),
        list(Some(Scope::Device)),
        list(Some(Scope::Workgroup)),
        workspace,
    )?;
    let mut by_scope = BTreeMap::from([(Scope::Device, device), (Scope::Workgroup, workgroup)]);
    if let Some(workspace) = workspace {
        by_scope.insert(Scope::Workspace, workspace);
    }
    Ok(ConfigState {
        schema: schema.settings,
        effective,
        by_scope,
    })
}

/// Write `set` and `unset` to `scope`: both checked first, so a batch the second would
/// reject writes nothing, then written in that order.
async fn config_write(
    c: &BridgeClient,
    scope: Scope,
    set: Vec<ConfigEntry>,
    unset: Vec<String>,
) -> sapphire_ipc::Result<ConfigWriteResult> {
    let scope = Some(scope);
    if !set.is_empty() {
        c.config_check_set(scope, set.clone()).await?;
    }
    if !unset.is_empty() {
        c.config_check_unset(scope, unset.clone()).await?;
    }
    let mut written = ConfigWriteResult::default();
    if !set.is_empty() {
        written = c.config_set(scope, set).await?;
    }
    if !unset.is_empty() {
        let removed = c.config_unset(scope, unset).await?;
        for key in removed.changed {
            if !written.changed.contains(&key) {
                written.changed.push(key);
            }
        }
        // Every key awaiting a restart, as of the later write.
        written.pending_restart = removed.pending_restart;
    }
    Ok(written)
}

/// Ask the app server to restart, wait for the connection that asked to close, and
/// reconnect until the new server answers `server.info`.
async fn restart_app(
    cfg: &ClientConfig,
    conns: &mut Connections,
) -> Result<CommandOutput, CommandError> {
    let app = cfg.app.app_name;
    let connected = conns.app(cfg).await.map(|c| c.is_some());
    let client: Arc<Client> = match (connected, conns.app_client()) {
        (Ok(true), Some(client)) => client,
        (Ok(_), _) => return Err(format!("the {app} server is not running").into()),
        (Err(e), _) => {
            conns.drop_app();
            return Err(failure(e));
        }
    };
    let asked: sapphire_ipc::Result<proto::Ack> = client
        .call(proto::SERVER_RESTART, serde_json::json!({}))
        .await;
    if let Err(e) = asked {
        // A refusal is an answer: the connection is fine.
        if !matches!(e, sapphire_ipc::Error::Rpc(_)) {
            conns.drop_app();
        }
        return Err(failure(e));
    }
    let came_back = async {
        client.closed().await;
        conns.drop_app();
        loop {
            if answers(cfg, conns).await {
                return;
            }
            tokio::time::sleep(APP_RESTART_POLL).await;
        }
    };
    if tokio::time::timeout(APP_RESTART_WAIT, came_back)
        .await
        .is_err()
    {
        conns.drop_app();
        return Err(format!(
            "the {app} server did not come back within {} s",
            APP_RESTART_WAIT.as_secs()
        )
        .into());
    }
    Ok(CommandOutput::Done)
}

/// Whether the app server answers `server.info` on the cached connection, making one if
/// needed; a failed attempt drops it, so the next one connects afresh.
async fn answers(cfg: &ClientConfig, conns: &mut Connections) -> bool {
    let answered = match conns.app(cfg).await {
        Ok(Some(c)) => c
            .call::<_, proto::StatusReport>(proto::SERVER_INFO, serde_json::json!({}))
            .await
            .is_ok(),
        _ => false,
    };
    if !answered {
        conns.drop_app();
    }
    answered
}

/// The error for a call that failed: its message, and the keys a rejected settings write
/// names.
fn failure(e: sapphire_ipc::Error) -> CommandError {
    let fields = match &e {
        sapphire_ipc::Error::Rpc(rpc) => ConfigFieldErrors::from_rpc(rpc)
            .map(|errors| errors.fields)
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    CommandError {
        message: e.to_string(),
        fields,
    }
}

/// The error for a relayed bridge call that failed.
fn bridge_failure(e: sapphire_ipc::Error) -> CommandError {
    match e {
        // The relay's answer that no bridge runs.
        sapphire_ipc::Error::NotRunning(_) => "sapphire-bridge is not running".into(),
        e => failure(e),
    }
}

async fn app_command(
    cfg: &ClientConfig,
    conns: &mut Connections,
    command: Command,
) -> Result<CommandOutput, CommandError> {
    let app = cfg.app.app_name;
    let c = conns
        .app(cfg)
        .await
        .map_err(failure)?
        .ok_or_else(|| format!("the {app} server is not running"))?;
    let r: sapphire_ipc::Result<()> = async {
        match command {
            Command::WorkspaceInit { dir, sync } => {
                let _: proto::WorkspaceInitResult = c
                    .call(proto::WORKSPACE_INIT, proto::WorkspaceInitParams { dir })
                    .await?;
                if sync {
                    let _: proto::SyncEnableResult =
                        c.call(proto::SYNC_ENABLE, serde_json::json!({})).await?;
                }
            }
            Command::WorkspaceSelect { dir } => {
                let _: proto::CurrentWorkspace = c
                    .call(
                        proto::WORKSPACE_SELECT,
                        proto::WorkspaceSelectParams { dir },
                    )
                    .await?;
            }
            Command::WorkspaceMove { dir } => {
                let _: proto::CurrentWorkspace = c
                    .call(proto::WORKSPACE_MOVE, proto::WorkspaceMoveParams { dir })
                    .await?;
            }
            Command::WorkspaceRemove { delete } => {
                let _: proto::Ack = c
                    .call(
                        proto::WORKSPACE_REMOVE,
                        proto::WorkspaceRemoveParams { delete },
                    )
                    .await?;
            }
            Command::SyncEnable => {
                let _: proto::SyncEnableResult =
                    c.call(proto::SYNC_ENABLE, serde_json::json!({})).await?;
            }
            Command::SyncDisable => {
                let _: proto::Ack = c.call(proto::SYNC_DISABLE, serde_json::json!({})).await?;
            }
            Command::WorkspaceMap { workspace_id, dir } => {
                // "Bring to this host…" names a folder inside the one picked, which does
                // not exist yet. The GUI runs on the same host as the server, so it makes it.
                std::fs::create_dir_all(&dir).map_err(|e| {
                    sapphire_ipc::Error::Io(std::io::Error::new(
                        e.kind(),
                        format!("could not create {}: {e}", dir.display()),
                    ))
                })?;
                // No `workspace.init` first: `sync.map` makes the marker, and a
                // `config.toml` written here would meet the workspace's own (#253).
                let _: proto::SyncEnableResult = c
                    .call(
                        proto::SYNC_MAP,
                        proto::SyncMapParams {
                            workspace: workspace_id.to_string(),
                            dir,
                        },
                    )
                    .await?;
            }
            _ => unreachable!("execute routes only app commands here"),
        }
        Ok(())
    }
    .await;
    r.map(|()| CommandOutput::Done).map_err(failure)
}

/// Run `<exe> service install`. The CLI's own verb registers the CLI binary, which is what
/// the service manager must start.
async fn install(cfg: &ClientConfig, target: ServiceTarget) -> Result<CommandOutput, String> {
    let exe = match target {
        ServiceTarget::Bridge => &cfg.service_exes.bridge,
        ServiceTarget::App => &cfg.service_exes.app,
    };
    let manual = format!("{} service install", exe.display());
    if !exe.is_file() {
        return Err(format!("{} was not found; run: {manual}", exe.display()));
    }
    let output = tokio::process::Command::new(exe)
        .args(["service", "install"])
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("{e}; run: {manual}"))?;
    if output.status.success() {
        Ok(CommandOutput::Done)
    } else {
        Err(install_failure(
            &String::from_utf8_lossy(&output.stderr),
            &String::from_utf8_lossy(&output.stdout),
            output.status.code(),
            &manual,
        ))
    }
}

/// The message for a failed `service install`: what it said on stderr, else on stdout,
/// else its exit status — never empty before the `; run:` hint.
fn install_failure(stderr: &str, stdout: &str, code: Option<i32>, manual: &str) -> String {
    let said = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|s| !s.is_empty())
        .map(str::to_owned);
    let what = said.unwrap_or_else(|| match code {
        Some(code) => format!("exit status {code}"),
        None => "it was stopped by a signal".to_owned(),
    });
    format!("{what}; run: {manual}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::conn::tests::{config, holds_app, over_a_refusing_relay};

    #[tokio::test]
    async fn a_bridge_error_in_a_command_keeps_the_app_connection() {
        let cfg = config();
        let mut conns = over_a_refusing_relay().await;

        let create = Command::WorkgroupCreate {
            name: "home".into(),
            device_name: "laptop".into(),
        };
        let result = execute(&cfg, &mut conns, create).await;
        assert!(
            matches!(&result, Err(e) if e.message.contains("the bridge said no")),
            "{result:?}"
        );
        assert!(
            holds_app(&conns),
            "the app server answered: its connection is fine"
        );
    }

    #[test]
    fn an_install_gets_its_own_limit_and_a_join_three_times_the_command_limit() {
        use std::time::Duration;
        let app = super::super::AppIdentity {
            app_name: "app",
            version: "0",
        };
        let dir = std::path::PathBuf::from("/run");
        let cfg = ClientConfig {
            app,
            endpoints: super::super::Endpoints {
                app: sapphire_ipc::Endpoint::in_dir("a", dir),
            },
            service_exes: super::super::ServiceExes {
                bridge: "/b".into(),
                app: "/a".into(),
            },
            refresh: Duration::from_secs(2),
            fetch_timeout: Duration::from_secs(5),
            command_timeout: Duration::from_secs(30),
            install_timeout: Duration::from_secs(300),
        };
        assert_eq!(
            limit(&cfg, &Command::ServiceInstall(ServiceTarget::App)),
            Duration::from_secs(300)
        );
        assert_eq!(
            limit(
                &cfg,
                &Command::WorkgroupJoin {
                    ticket: "t".into(),
                    device_name: None
                }
            ),
            Duration::from_secs(90)
        );
        assert_eq!(limit(&cfg, &Command::SyncEnable), Duration::from_secs(30));
        assert_eq!(limit(&cfg, &Command::ConfigLoad), Duration::from_secs(30));
        for process in [Process::App, Process::Bridge] {
            assert_eq!(
                limit(&cfg, &Command::Restart(process)),
                Duration::from_secs(45)
            );
        }
    }

    #[test]
    fn a_rejected_write_keeps_its_field_errors() {
        let fields = ConfigFieldErrors {
            fields: vec![sapphire_bridge_api::ConfigFieldError {
                key: "net.discovery".into(),
                message: "expected true or false".into(),
            }],
        };
        let rpc = sapphire_ipc::RpcError {
            data: Some(fields.to_data()),
            ..sapphire_ipc::RpcError::internal("invalid value for `net.discovery`")
        };
        let err = bridge_failure(sapphire_ipc::Error::Rpc(rpc));
        assert_eq!(err.fields, fields.fields);
        assert!(err.message.contains("net.discovery"), "{err:?}");

        let plain = bridge_failure(sapphire_ipc::Error::Rpc(sapphire_ipc::RpcError::internal(
            "no",
        )));
        assert!(plain.fields.is_empty(), "{plain:?}");
        assert_eq!(
            bridge_failure(sapphire_ipc::Error::NotRunning("x".into())),
            CommandError::from("sapphire-bridge is not running")
        );
    }

    #[test]
    fn an_install_failure_prefers_stderr_then_stdout_then_the_status() {
        let manual = "x service install";
        assert_eq!(
            install_failure(" denied \n", "ignored", Some(1), manual),
            "denied; run: x service install"
        );
        assert_eq!(
            install_failure("  ", "need admin\n", Some(1), manual),
            "need admin; run: x service install"
        );
        assert_eq!(
            install_failure("", "", Some(3), manual),
            "exit status 3; run: x service install"
        );
        let killed = install_failure("", "", None, manual);
        assert!(!killed.starts_with(';'), "{killed}");
        assert!(killed.ends_with("; run: x service install"), "{killed}");
    }
}
