//! `config.*` on the app server (#314): its own settings merged with the bridge's.
//!
//! The app server answers the five methods itself rather than relaying them. Its own keys
//! — the framework's and the application's, in [`Settings`] — come from its own files; the
//! rest are the bridge's, asked over the relay's connection. Clients do not change: the
//! methods keep their names and types.
//!
//! - `config.schema`: this server's definitions, then the bridge's.
//! - `config.list`: both sides' entries; the workgroup scope is the bridge's alone and the
//!   workspace scope this server's alone.
//! - `config.get`: answered by the key's owner.
//! - `config.set` / `config.unset`: the batch split by owner, this server's part checked,
//!   then the bridge's (with `dry_run`), and only then written: the bridge's first, then
//!   this server's.
//!
//! With no bridge running, reads answer this server's part alone, and a batch or a `get`
//! that needs the bridge fails. Values are never logged: only the method and the outcome.
//!
//! See `docs/superpowers/specs/2026-10-22-app-settings-design.md`, "`config.*` on the app
//! server".

use std::future::Future;
use std::sync::Arc;

use sapphire_bridge_api::{
    BRIDGE_NOT_RUNNING, CONFIG_GET, CONFIG_LIST, CONFIG_SCHEMA, CONFIG_SET, CONFIG_UNSET,
    ConfigEntry, ConfigGetParams, ConfigListParams, ConfigListResult, ConfigSchemaResult,
    ConfigSetParams, ConfigUnsetParams, ConfigWriteResult,
};
use sapphire_bridge_client::BridgeClient;
use sapphire_framework_settings::{Entry, Op, Registry, Scope};
use sapphire_ipc::{Router, RpcError};
use serde_json::Value;

use crate::relay::{self, Relay};
use crate::settings::{self, Settings};

/// What a batch with a bridge part says to a bridge that would write it rather than check
/// it.
const TOO_OLD: &str =
    "this sapphire-bridge is too old to check settings without writing them; update it";

/// What answers `config.*`.
struct Config {
    settings: Settings,
    relay: Arc<Relay>,
    /// Held across a whole batch, checks and both writes, so two batches never interleave.
    writing: tokio::sync::Mutex<()>,
}

/// `router` with `config.schema`, `config.list`, `config.get`, `config.set` and
/// `config.unset` answered from `settings` and the bridge `relay` reaches.
pub(crate) fn routes(router: Router, settings: Settings, relay: Arc<Relay>) -> Router {
    let config = Arc::new(Config {
        settings,
        relay,
        writing: tokio::sync::Mutex::new(()),
    });
    let router = answer(router, &config, CONFIG_SCHEMA, |config, _| async move {
        encode(config.schema().await?)
    });
    let router = answer(router, &config, CONFIG_LIST, |config, params| async move {
        let params: ConfigListParams = parse(params, CONFIG_LIST)?;
        encode(config.list(params.scope).await?)
    });
    let router = answer(router, &config, CONFIG_GET, |config, params| async move {
        let params: ConfigGetParams = parse(params, CONFIG_GET)?;
        encode(config.get(&params.key).await?)
    });
    let router = answer(router, &config, CONFIG_SET, |config, params| async move {
        let params: ConfigSetParams = parse(params, CONFIG_SET)?;
        encode(
            config
                .write(params.scope, Batch::Set(params.entries), params.dry_run)
                .await?,
        )
    });
    answer(router, &config, CONFIG_UNSET, |config, params| async move {
        let params: ConfigUnsetParams = parse(params, CONFIG_UNSET)?;
        encode(
            config
                .write(params.scope, Batch::Unset(params.keys), params.dry_run)
                .await?,
        )
    })
}

/// `router` with `method` answered by `f`, logging the method and the outcome only.
fn answer<F, Fut>(router: Router, config: &Arc<Config>, method: &'static str, f: F) -> Router
where
    F: Fn(Arc<Config>, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value, RpcError>> + Send + 'static,
{
    let config = Arc::clone(config);
    let f = Arc::new(f);
    router.method(method, move |ctx| {
        let result = f(Arc::clone(&config), ctx.params);
        async move {
            let result = result.await;
            match &result {
                Ok(_) => tracing::debug!(method, outcome = "ok", "answered a config call"),
                Err(e) => tracing::debug!(method, outcome = e.code, "answered a config call"),
            }
            result
        }
    })
}

/// A `config.set` or `config.unset` batch.
enum Batch {
    Set(Vec<ConfigEntry>),
    Unset(Vec<String>),
}

impl Batch {
    /// This server's part and the bridge's: a key this server's registry knows, or a prefix
    /// of keys it knows, is its own; anything else is the bridge's.
    fn split(self, reg: &Registry) -> (Batch, Batch) {
        let own = |key: &str| reg.def(key).is_ok() || !reg.under(key).is_empty();
        match self {
            Batch::Set(entries) => {
                let (mine, theirs) = entries.into_iter().partition(|e| own(&e.key));
                (Batch::Set(mine), Batch::Set(theirs))
            }
            Batch::Unset(keys) => {
                let (mine, theirs) = keys.into_iter().partition(|k| own(k));
                (Batch::Unset(mine), Batch::Unset(theirs))
            }
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Batch::Set(entries) => entries.is_empty(),
            Batch::Unset(keys) => keys.is_empty(),
        }
    }

    /// The keys, in order.
    fn keys(&self) -> Vec<&str> {
        match self {
            Batch::Set(entries) => entries.iter().map(|e| e.key.as_str()).collect(),
            Batch::Unset(keys) => keys.iter().map(String::as_str).collect(),
        }
    }

    /// The batch as the settings store's ops.
    fn ops(&self) -> Vec<Op> {
        match self {
            Batch::Set(entries) => entries
                .iter()
                .map(|e| Op::Set {
                    key: e.key.clone(),
                    value: e.value.clone(),
                })
                .collect(),
            Batch::Unset(keys) => keys
                .iter()
                .map(|key| Op::Unset { key: key.clone() })
                .collect(),
        }
    }

    /// Send the batch to the bridge: checked only, with `dry_run`.
    async fn send(
        &self,
        bridge: &BridgeClient,
        scope: Option<Scope>,
        dry_run: bool,
    ) -> Result<ConfigWriteResult, RpcError> {
        let result = match (self, dry_run) {
            (Batch::Set(entries), true) => bridge.config_check_set(scope, entries.clone()).await,
            (Batch::Set(entries), false) => bridge.config_set(scope, entries.clone()).await,
            (Batch::Unset(keys), true) => bridge.config_check_unset(scope, keys.clone()).await,
            (Batch::Unset(keys), false) => bridge.config_unset(scope, keys.clone()).await,
        };
        result.map_err(from_bridge)
    }
}

impl Config {
    /// `config.schema`: this server's definitions, then the bridge's when one answers.
    async fn schema(&self) -> Result<ConfigSchemaResult, RpcError> {
        let mut settings = self.settings.registry().schema();
        if let Some(theirs) = self
            .ask(|bridge| async move { bridge.config_schema().await })
            .await?
        {
            settings.extend(theirs.settings);
        }
        Ok(ConfigSchemaResult { settings })
    }

    /// `config.list`: this server's entries, then the bridge's when one answers; the
    /// workgroup scope is the bridge's alone, the workspace scope this server's alone.
    async fn list(&self, scope: Option<Scope>) -> Result<ConfigListResult, RpcError> {
        let mut entries = Vec::new();
        if scope != Some(Scope::Workgroup) {
            entries = self.own_list(scope).map_err(failed)?;
        }
        if scope != Some(Scope::Workspace)
            && let Some(theirs) = self
                .ask(|bridge| async move { bridge.config_list(scope).await })
                .await?
        {
            entries.extend(theirs.entries);
        }
        Ok(ConfigListResult { entries })
    }

    /// `config.get`: from this server's files when the key is its own, from the bridge
    /// otherwise.
    async fn get(&self, key: &str) -> Result<Entry, RpcError> {
        let reg = self.settings.registry();
        if reg.def(key).is_ok() {
            // Through `list`, which alone marks a pending restart.
            let entry = sapphire_framework_settings::get(reg, key).map_err(failed)?;
            return Ok(self
                .own_list(None)
                .map_err(failed)?
                .into_iter()
                .find(|e| e.key == key)
                .unwrap_or(entry));
        }
        let bridge = self.relay.bridge().await?;
        bridge.config_get(key).await.map_err(from_bridge)
    }

    /// `config.set` / `config.unset`: check this server's part, have the bridge check its
    /// part, then write the bridge's part and this server's. With `dry_run`, only check.
    async fn write(
        &self,
        scope: Option<Scope>,
        batch: Batch,
        dry_run: bool,
    ) -> Result<ConfigWriteResult, RpcError> {
        let _one = self.writing.lock().await;
        let reg = self.settings.registry();
        let (mine, theirs) = batch.split(reg);

        let mut changed = Vec::new();
        if !mine.is_empty() {
            changed = sapphire_framework_settings::check(reg, scope, mine.ops())
                .map_err(failed)?
                .keys;
        }

        let mut bridge_result = None;
        if !theirs.is_empty() {
            let bridge = match self.relay.bridge().await {
                Ok(bridge) => bridge,
                Err(e) if e.code == BRIDGE_NOT_RUNNING => return Err(no_bridge_for(&theirs)),
                Err(e) => return Err(e),
            };
            // A batch of the bridge's keys alone is the bridge's to check as it writes; one
            // with this server's keys too must not reach a bridge that would write it
            // before this server's part is known to pass.
            if !mine.is_empty() || dry_run {
                let status = bridge.status().await.map_err(from_bridge)?;
                if !status.config_dry_run {
                    return Err(RpcError::internal(TOO_OLD));
                }
                let checked = theirs.send(&bridge, scope, true).await?;
                if dry_run {
                    bridge_result = Some(checked);
                }
            }
            if !dry_run {
                bridge_result = Some(theirs.send(&bridge, scope, false).await?);
            }
        }

        if !dry_run && !mine.is_empty() {
            match sapphire_framework_settings::apply(reg, scope, mine.ops()) {
                Ok(applied) => changed = applied.keys,
                Err(e) if bridge_result.is_some() => {
                    return Err(RpcError::internal(format!(
                        "sapphire-bridge's settings were written, but this server's were not: {e}"
                    )));
                }
                Err(e) => return Err(failed(e)),
            }
            if self.settings.refresh().is_err() {
                // The error may quote the file; the write itself succeeded.
                tracing::warn!("could not read the settings back after a write");
            }
        }

        let mut pending_restart: Vec<String> = self
            .own_list(None)
            .map_err(failed)?
            .into_iter()
            .filter(|e| e.pending_restart)
            .map(|e| e.key)
            .collect();
        match bridge_result {
            Some(theirs) => {
                changed.extend(theirs.changed);
                pending_restart.extend(theirs.pending_restart);
            }
            // The bridge's own pending keys, from earlier batches, when it answers.
            None => {
                if let Ok(Some(theirs)) = self
                    .ask(|bridge| async move { bridge.config_list(None).await })
                    .await
                {
                    pending_restart.extend(
                        theirs
                            .entries
                            .into_iter()
                            .filter(|e| e.pending_restart)
                            .map(|e| e.key),
                    );
                }
            }
        }
        Ok(ConfigWriteResult {
            changed,
            pending_restart,
        })
    }

    /// This server's entries, restart keys changed since it started marked.
    fn own_list(&self, scope: Option<Scope>) -> sapphire_framework_settings::Result<Vec<Entry>> {
        sapphire_framework_settings::list(
            self.settings.registry(),
            scope,
            Some(self.settings.start()),
        )
    }

    /// Ask the bridge with `call`: `None` when there is no bridge to ask, or it went away
    /// under the call; the bridge's own errors as it answered them.
    async fn ask<T, F, Fut>(&self, call: F) -> Result<Option<T>, RpcError>
    where
        F: FnOnce(Arc<BridgeClient>) -> Fut,
        Fut: Future<Output = sapphire_ipc::Result<T>>,
    {
        let Ok(bridge) = self.relay.bridge().await else {
            return Ok(None);
        };
        match call(bridge).await {
            Ok(answer) => Ok(Some(answer)),
            Err(sapphire_ipc::Error::Rpc(e)) => Err(e),
            Err(_) => Ok(None),
        }
    }
}

/// Why a batch with `theirs`, the keys this server does not know, fails with no bridge
/// running: a key under one of the bridge's names needs the bridge; any other may be the
/// bridge's or nobody's, and the message says both.
fn no_bridge_for(theirs: &Batch) -> RpcError {
    let unknown = theirs.keys().into_iter().find(|key| {
        let top = key.split('.').next().unwrap_or(key);
        !settings::BRIDGE_NAMES.contains(&top)
    });
    match unknown {
        Some(key) => RpcError::internal(format!(
            "unknown setting {key} (or sapphire-bridge, which may know it, is not running)"
        )),
        None => relay::not_running(),
    }
}

/// A bridge call's failure: the bridge's own error as it answered it; a connection that
/// failed under the call, not running. The transport's error is not passed on, since a
/// codec error may quote the frame.
fn from_bridge(err: sapphire_ipc::Error) -> RpcError {
    match err {
        sapphire_ipc::Error::Rpc(e) => e,
        _ => relay::not_running(),
    }
}

/// A settings failure, as the error that carries it back. The message is the client's to
/// read; it is never logged.
fn failed(err: sapphire_framework_settings::Error) -> RpcError {
    RpcError::internal(err.to_string())
}

/// A method's parameters, or the error that says they are malformed.
fn parse<T: serde::de::DeserializeOwned>(params: Value, method: &str) -> Result<T, RpcError> {
    serde_json::from_value(params)
        .map_err(|e| RpcError::invalid_params(format!("malformed {method}: {e}")))
}

fn encode<T: serde::Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|e| RpcError::internal(e.to_string()))
}
