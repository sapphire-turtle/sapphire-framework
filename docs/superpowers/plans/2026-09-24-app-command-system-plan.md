# App Command System: framework-provided flat CLI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> If your harness has no such skill, execute the tasks in order, one at a time, running the
> listed commands and committing at the end of each task. Do not skip the "run the test and
> watch it fail" steps: they are what proves the test exercises the new code.

**Goal:** Replace `ServerCommand { Run|Status|Stop|Service }` and the start-on-demand
machinery with a framework-provided flat command vocabulary — `serve` / `status` /
`service` / `workspace` / `workgroup` / `device` — that every app binary composes into its
own CLI, plus signal-driven graceful shutdown for the now always-on server.

**Architecture:** `FrameworkCommand` is a clap `Subcommand` the app flattens next to its own
subcommand enum; the framework's `dispatch` helper executes its variants and renders a typed
status report shared with the IPC `server.info` response. The server runs until SIGTERM or
SIGINT; `stop` as a CLI command is dropped, and spawn-on-demand (`ensure_server` /
`SpawnConfig` / `SpawnLock` / idle exit / `ManagedBy::Spawned` as an operating mode) is
removed from `sapphire-framework-ipc` and `sapphire-framework-server`. Control-plane routing
follows ownership: `workspace init` and `map`'s write, plus the registry they maintain, live
on the app server's IPC; only the workgroup-wide ledger (`workspace list`'s second layer,
`map`'s selector resolution) and the `workgroup` / `device` commands go to the bridge's
endpoint directly.

**Tech Stack:** Rust 2024 (toolchain 1.98.0), clap 4 (derive), tokio 1 (signal handling),
serde + serde_json, `sapphire-framework-ipc`, `sapphire-framework-bridge-api`,
`sapphire-framework-service`, `sapphire-framework-workspace`, `sapphire-framework-backend`
(`WorkspaceRegistry`).

**Spec:** `docs/superpowers/specs/2026-09-24-app-command-system-design.md` (issue #142),
which revises `2026-09-16-process-architecture-design.md` decision 5.

**Depends on:** the process-architecture spec steps already landed on
`feat/p2p-sync-iroh` (bridge methods `bridge.register/unregister/peers/status/invite/join/workspaces`
exist; `ServiceCommand` is reusable as is; `WorkspaceRegistry` and the `sync.*` methods
exist).

**Branch:** work on `feat/p2p-sync-iroh` (the current branch). Phase 2 — re-homing the
bridge's own `command.rs` — is a separate issue and is **not** touched here; the bridge
crate compiles and behaves exactly as today.

## Global Constraints

- Code, comments, commit messages and tests in **English** (`CONTRIBUTING.md`).
- CI runs `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  and `cargo test --all-features --locked`. All three must pass after every task.
- Every public item carries a doc comment (`#![warn(missing_docs)]` is set crate-wide).
- **No aliases for the removed commands** (spec decision 9): `server run|status|stop` and
  `RunArgs` are deleted outright.
- The bridge crate is read-only in this plan: no edit under
  `crates/sapphire-framework-bridge/` or `apps/sapphire-bridge/`.
- The IPC crate stays free of the workspace/search/sync stacks — the
  `dependency_surface_is_documented` test in `roundtrip.rs` pins it; the connect helper
  added in Task 2 must not grow dependencies.
- **Commands never start daemons** (spec decisions 3, 6, 7): a one-shot command that
  needs a process which is not running reports it and exits 1. `serve` and the service
  manager are the only things that start servers.
- Every removed public item's tests are migrated or deleted in the same task — no orphan
  tests referencing deleted APIs may survive a task's commit.

## File Structure

```
crates/sapphire-framework-ipc/src/
    spawn.rs           # MODIFIED: drops SpawnConfig / SpawnLock / ensure_server /
                       #   STALE_LOCK_AGE; keeps connect / connect_raw / probe /
                       #   handshake path / SHUTDOWN_METHOD; adds connect_or_absent
    lib.rs             # MODIFIED: re-exports shrink accordingly

crates/sapphire-framework-server/src/
    command.rs         # REWRITTEN: FrameworkCommand + dispatch + status rendering
                       #   + workspace/workgroup/device commands
    lib.rs             # MODIFIED: AppServer loses idle_exit/RunArgs/managed_by builder;
                       #   run() gains SIGTERM/SIGINT; service_spec args = ["serve"];
                       #   the router gains workspace.init
    error.rs           # MODIFIED: spawn-related variants, if any, updated

crates/sapphire-framework-backend/src/
    protocol.rs        # MODIFIED: WORKSPACE_INIT method + params/result types

crates/sapphire-framework-bridge-api/src/
    client.rs          # MODIFIED: connect() stops using ensure_server; call-through
                       #   methods for the workgroup/device/list/map-resolution commands

crates/sapphire-framework-backend/src/
    ipc.rs             # MODIFIED: connect() stops using ensure_server

crates/sapphire-framework/src/
    lib.rs             # MODIFIED: prelude re-exports FrameworkCommand

docs/ARCHITECTURE.md  # MODIFIED: ServerCommand / start-on-demand descriptions updated
```

---

### Task 1: `FrameworkCommand` with a proven flatten composition

**Files:**
- Rewrite: `crates/sapphire-framework-server/src/command.rs`
- Modify: `crates/sapphire-framework-server/src/lib.rs` (export list only — `RunArgs` /
  `ServerCommand` / `spawn_config_for` replaced by `FrameworkCommand` + `dispatch`)
- Test: inside `command.rs` (`mod tests`)

**Interfaces:**
- Consumes: clap 4 derive; `sapphire_framework_service::{ServiceCommand, Environment,
  SystemManager}`; `AppServer` (for `serve` and `service_spec`).
- Produces:
  - `FrameworkCommand` — `#[derive(Debug, Default, clap::Subcommand)]` with variants
    `Serve`, `Status`, `Service(ServiceCommand)` in this task (the workspace/workgroup/
    device ones arrive in Task 4); `#[command(subcommand)]` for `Service`, as today.
  - `FrameworkCommand::dispatch(self, server: AppServer, version: &'static str) -> Result<i32>`
    — the helper apps call for the framework half of their CLI. Reads the app name from
    `server` itself (`AppContext`), so the caller passes no redundant `app: &str`.
  - `#[derive(Parser)] struct Probe { #[command(subcommand)] app: AppCommand,
    #[command(flatten)] framework: FrameworkCommand }` (test-local) proving the composition.

**Design notes (from the spec):**
- Decision 2 requires the app CLI to compose `FrameworkCommand` **as a flattened second
  field next to its own subcommand**, so app-specific commands sit at the same top level
  as framework commands. The parse test below is the spec-mandated proof that clap 4
  accepts this shape.
- `serve` runs `AppServer::run()` with no flags (decision 3); `service install` uses
  `server.service_spec()` exactly as `ServerCommand::Service` does today.
- `dispatch` does **not** match app-specific variants: the app matches its own enum, calls
  `framework.dispatch` for the framework one, and decides precedence itself.

- [ ] **Step 1: Write the failing tests**

In `crates/sapphire-framework-server/src/command.rs`, replace the three existing test
modules with one that states the new shape (the old `the_subcommands_parse` asserts
`Run/Status/Stop` and stops compiling against the new enum):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// The composition every app binary uses: its own subcommand enum beside the
    /// framework's, flattened (spec decision 2).
    #[derive(Parser)]
    struct Probe {
        #[command(subcommand)]
        app: AppCommand,
        #[command(flatten)]
        framework: FrameworkCommand,
    }

    #[derive(Debug, clap::Subcommand)]
    enum AppCommand {
        /// An app-specific verb that must parse beside the framework's.
        Greet,
    }

    #[test]
    fn app_and_framework_commands_parse_side_by_side() {
        let probe = Probe::try_parse_from(["app", "greet"]).unwrap();
        assert!(matches!(probe.app, AppCommand::Greet));
        let probe = Probe::try_parse_from(["app", "serve"]).unwrap();
        assert!(matches!(probe.framework, FrameworkCommand::Serve));
        let probe = Probe::try_parse_from(["app", "status"]).unwrap();
        assert!(matches!(probe.framework, FrameworkCommand::Status));
    }

    #[test]
    fn the_service_subcommands_parse() {
        for args in [
            vec!["app", "service", "install"],
            vec!["app", "service", "install", "--system"],
            vec!["app", "service", "install", "--run-as", "alice"],
            vec!["app", "service", "uninstall"],
            vec!["app", "service", "status"],
        ] {
            assert!(Probe::try_parse_from(&args).is_ok(), "{args:?}");
        }
    }

    #[test]
    fn bare_invocation_defaults_to_serve() {
        // <app> with no subcommand at all means serve (spec decision 1). An app CLI
        // achieves it by `Option<AppCommand>` + `Option<FrameworkCommand>`; this probe
        // pins the framework half.
        #[derive(Parser)]
        struct Bare {
            #[command(subcommand)]
            app: Option<AppCommand>,
            #[command(flatten)]
            framework: Option<FrameworkCommand>,
        }
        let bare = Bare::try_parse_from(["app"]).unwrap();
        assert!(bare.app.is_none() && bare.framework.is_none());
        assert!(matches!(
            FrameworkCommand::default(),
            FrameworkCommand::Serve
        ));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sapphire-framework-server --all-features command`
Expected: FAIL — `FrameworkCommand` does not exist; `ServerCommand::Run` does.

- [ ] **Step 3: Implement**

Rewrite `command.rs`:

```rust
//! The framework-provided half of an application's CLI.

use sapphire_framework_service::{Environment, ServiceCommand, SystemManager};

use crate::AppServer;
use crate::error::{Error, Result};

/// The framework's commands, flattened into an application's CLI (spec decision 2).
#[derive(Debug, Default, clap::Subcommand)]
pub enum FrameworkCommand {
    /// Run the server in this process until SIGTERM or SIGINT (the bare invocation).
    #[default]
    Serve,
    /// Report whether a server is running, and which version.
    Status,
    /// Install, remove or report this application's operating-system service.
    #[command(subcommand)]
    Service(ServiceCommand),
}

impl FrameworkCommand {
    /// Carry out the command, returning the process exit code.
    ///
    /// The application matches its own variants first, then hands the framework's here;
    /// the app name and the service spec come from `server` itself.
    pub async fn dispatch(self, server: AppServer, version: &'static str) -> Result<i32> {
        match self {
            FrameworkCommand::Serve => {
                server.run().await?;
                Ok(0)
            }
            FrameworkCommand::Status => crate::command::status(&server, version).await,
            FrameworkCommand::Service(command) => {
                let spec = server.service_spec();
                command
                    .run(&spec, &Environment::detect(), &SystemManager)
                    .map_err(Error::from)
            }
        }
    }
}
```

In `lib.rs`: `pub use command::FrameworkCommand;` (replacing
`pub use command::{RunArgs, ServerCommand, spawn_config_for};`). Keep a private
`async fn status(...)` stub in `command.rs` for this task (it grows the report shape in
Task 3) so `dispatch` compiles; its body may reuse today's probe logic **without**
`ensure_server` — for this task only, calling `probe()` and failing fast is acceptable
because Task 3 replaces it. Remove `RunArgs`, `spawn_config_for`, `client_info`,
`status`'s `ensure_server` call and `stop` in the same edit — the enum no longer has the
variants, and leaving them would be dead code that clippy flags.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sapphire-framework-server --all-features`
Expected: PASS for the new parse tests; the old `status_reports_no_server_when_none_is_running`
is rewritten in Task 3 (delete it here if it no longer compiles).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test -p sapphire-framework-server --all-features
git add crates/sapphire-framework-server
git commit -m "feat(server): provide the flat FrameworkCommand for app CLIs"
```

---

### Task 2: Signal-driven shutdown, and the end of start-on-demand

**Files:**
- Modify: `crates/sapphire-framework-server/src/lib.rs` (the `run()` select loop, the
  builder, and its tests)
- Modify: `crates/sapphire-framework-ipc/src/{spawn.rs,lib.rs}`
- Modify: `crates/sapphire-framework-backend/src/ipc.rs`,
  `crates/sapphire-framework-bridge-api/src/client.rs` (their `connect`s stop spawning)
- Modify: `crates/sapphire-framework-server/tests/common/mod.rs` (fixture helpers)
- Delete: `crates/sapphire-framework-ipc/tests/race.rs`,
  `crates/sapphire-framework-ipc/tests/spawn.rs` (they test removed machinery)
- Modify: `crates/sapphire-framework-server/tests/concurrent.rs`,
  `crates/sapphire-framework-server/tests/sync_wiring.rs` (they use `ensure_server`)
- Test: `crates/sapphire-framework-server/tests/command_lifecycle.rs` (new)

**Interfaces:**
- Consumes: `tokio::signal::unix::{signal, SignalKind}` / `tokio::signal::ctrl_c()`;
  `sapphire_ipc::{connect, probe, Client::handshake}`.
- Produces:
  - `AppServer::run()` gains a third select arm: SIGINT via `tokio::signal::ctrl_c()`
    (both platforms), and SIGTERM via a Unix-only `SignalKind::terminate()` stream.
  - `AppServer` builder loses `idle_exit`, `managed_by`, and the struct fields
    `idle_exit` / `managed_by`; `DEFAULT_IDLE_EXIT` and `IDLE_TICK` are deleted; the
    `ServerInfo` handed to the handshake always reports `ManagedBy::Service`.
  - `sapphire_ipc::connect_or_absent(endpoint, app, info) -> Result<Option<(Client, ServerInfo)>>`
    — handshake or `Ok(None)` when nothing is listening (or the listener is a corpse:
    `probe` has already cleared the socket file). `Error::Spawn` loses the spawn-era
    wording; `Error::ServiceVersionMismatch` keeps "restart the service".
  - `IpcBackend::connect(endpoint, app, kind, version, ws)` and
    `BridgeClient::connect(kind, version)` — same signatures minus the `spawn` parameter,
    built on `connect_or_absent` and failing with a clear error when nothing listens.
  - `SyncRuntime::new` keeps its `managed_by: ManagedBy` parameter (it registers with the
    bridge, and the bridge's `wake_on_sync` still distinguishes the two) — but callers now
    always pass `ManagedBy::Service`; the `exe_path` parameter stays for the same reason.

**Design notes:**
- Windows: SIGTERM has no equivalent, so the Unix stream is `#[cfg(unix)]` and `ctrl_c()`
  covers SIGINT on every platform. `ctrl_c()` already swallows the default handler, so
  SIGINT must be listened to **only** through it on Unix (listening to
  `SignalKind::interrupt()` as well would double-fire for ^C).
- The shutdown path itself already exists: the select's `stop_rx.changed()` arm breaks the
  loop, aborts the sync tasks and drops the listener, which unlinks the Unix socket. A
  signal arm only needs to `break` the same way.
- `wait_until_listening` helpers in tests already spin on `probe`; they stay.
- `ensure_server`'s version-mismatch logic (`ManagedBy::Spawned` retirement) dies with
  spawn. A version mismatch against a **service** now surfaces from `connect_or_absent` as
  `Error::ServiceVersionMismatch` ("the installed service is version X, this CLI is
  version Y; restart the service") — keep the variant and raise it after the handshake,
  so `race.rs`'s message assertion migrates into `roundtrip.rs`.

- [ ] **Step 1: Write the failing tests**

`crates/sapphire-framework-server/tests/command_lifecycle.rs` (new file):

```rust
//! `serve` runs until signalled, and shuts down cleanly.
#![cfg(unix)]

use std::time::Duration;

use sapphire_framework_server::AppServer;
use sapphire_ipc::{Endpoint, ManagedBy};
use sapphire_workspace::{AppContext, AppKind};

static CTX: AppContext = AppContext::new("sapphire-signals-test");

#[tokio::test(flavor = "multi_thread")]
async fn sigterm_ends_a_running_server() {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: single-threaded test-process setup, before any other thread reads these.
    unsafe {
        std::env::set_var("SAPPHIRE_SIGNALSTEST_CACHE_DIR", tmp.path().join("cache"));
        std::env::set_var("SAPPHIRE_SIGNALSTEST_DATA_DIR", tmp.path().join("data"));
        std::env::set_var("SAPPHIRE_SIGNALSTEST_CONFIG_DIR", tmp.path().join("config"));
    }
    CTX.init(AppKind::Server);

    let endpoint = Endpoint::in_dir(CTX.app_name, tmp.path().to_path_buf());
    let server = AppServer::new(&CTX, env!("CARGO_PKG_VERSION")).endpoint(endpoint.clone());
    let handle = tokio::spawn(async move { server.run().await });

    // The signal streams are registered before the bind, so by the time the socket is
    // up, SIGTERM cannot fall through to the default handler and kill the harness.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !sapphire_ipc::probe(&endpoint).await.unwrap() {
        assert!(tokio::time::Instant::now() < deadline, "the server never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // SAFETY: this process *is* the test subject; the handler is tokio's.
    unsafe { libc::kill(std::process::id() as libc::pid_t, libc::SIGTERM) };

    tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("SIGTERM must stop the server")
        .unwrap()
        .unwrap();
    assert!(
        !sapphire_ipc::probe(&endpoint).await.unwrap(),
        "the socket file must be gone after shutdown"
    );
}
```

> Windows coverage of SIGINT comes from the existing `shutdown_stops_the_server` IPC-path
> test plus a ctrl_c unit test that would need a child process; the plan accepts that gap
> (the ctrl_c arm is three lines shared with the SIGTERM arm's `break`).

In `crates/sapphire-framework-server/src/lib.rs`'s test module, replace
`a_spawned_server_exits_when_it_goes_idle` and
`a_service_server_does_not_exit_when_idle` (both die with the idle machinery) with:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn a_server_reports_itself_as_service_managed() {
    let f = prepared();
    let endpoint = f.endpoint.clone();
    let server = AppServer::new(&CTX, "0.0.0").endpoint(endpoint.clone());
    let handle = tokio::spawn(async move { server.run().await });
    wait_until_listening(&endpoint).await;

    let (client, info) = sapphire_ipc::connect_or_absent(&endpoint, CTX.app_name, client_info())
        .await
        .unwrap()
        .expect("the server is listening");
    assert_eq!(info.managed_by, ManagedBy::Service);

    let _: serde_json::Value = client
        .call(sapphire_ipc::SHUTDOWN_METHOD, serde_json::json!({}))
        .await
        .unwrap();
    handle.await.unwrap().unwrap();
}
```

The remaining lib.rs tests lose `&SpawnConfig::disabled()` from their `ensure_server`
calls, switching to `connect_or_absent(...).unwrap().unwrap()`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sapphire-framework-server --all-features --test command_lifecycle`
Expected: FAIL — the server does not react to SIGTERM (the loop never breaks).

Also run `cargo test -p sapphire-framework-ipc --all-features` after editing the tests:
FAIL — `connect_or_absent` does not exist yet.

- [ ] **Step 3: Implement**

1. **`sapphire-framework-ipc/src/spawn.rs`:** delete `SpawnConfig`, `SpawnLock`,
   `STALE_LOCK_AGE`, `ensure_server`, `connect_compatible`, `wait_until_gone`,
   `lock_is_stale`, and `handshake_with`'s spawn-specific caller. Add:

```rust
/// Handshake with the server on `endpoint`, or report that nothing is there.
///
/// `Ok(None)` covers both "nothing is listening" and "a corpse socket": `probe` has
/// already cleared the latter. A server of a different version is an error, not a
/// replacement — start-on-demand retired servers by re-exec, and this crate no longer
/// starts anything.
pub async fn connect_or_absent(
    endpoint: &Endpoint,
    app: &str,
    client: ClientInfo,
) -> Result<Option<(Client, ServerInfo)>> {
    if !probe(endpoint).await? {
        return Ok(None);
    }
    match Client::handshake(connect(endpoint).await?, app, client).await {
        Ok(pair) => Ok(Some(pair)),
        Err(Error::Io(_) | Error::Closed) => Ok(None),
        Err(err) => Err(err),
    }
}
```

   Move the version-mismatch matching (the `Error::ServiceVersionMismatch` branch) into
   `connect_or_absent`'s handshake error path so the "restart the service" message
   survives. Update `lib.rs` re-exports:
   `pub use spawn::{SHUTDOWN_METHOD, connect, connect_raw, connect_or_absent, probe};`
   (drop `SpawnConfig`, `ensure_server`, `STALE_LOCK_AGE`). Delete
   `crates/sapphire-framework-ipc/tests/{race.rs,spawn.rs}` and the `ipc-test-server`
   bin's `--shutdownable`/`--service` flags only if orphaned — migrate
   `a_service_managed_server_of_another_version_is_not_replaced`'s message assertion into
   a small test in `roundtrip.rs` if it does not survive the deletion of the spawn path.

2. **`sapphire-framework-server/src/lib.rs`:** in `run()`, replace the idle ticker block
   and its fields with the signal arms:

```rust
#[cfg(unix)]
let mut sigterm = tokio::signal::unix::signal(
    tokio::signal::unix::SignalKind::terminate(),
)?;

loop {
    tokio::select! {
        changed = stop_rx.changed() => {
            if changed.is_err() || *stop_rx.borrow() {
                break;
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("interrupted; shutting down");
            break;
        }
        #[cfg(unix)]
        _ = sigterm.recv() => {
            tracing::info!("terminated; shutting down");
            break;
        }
        accepted = listener.accept() => { /* unchanged */ }
    }
}
```

   Delete: `idle_exit` / `managed_by` fields and builders, `DEFAULT_IDLE_EXIT`,
   `IDLE_TICK`, the `last_activity` idle bookkeeping, and the `ServerInfo` handshake's
   `ManagedBy::Spawned` initialisation (always `Service`). `service_spec().args` becomes
   `vec!["serve".to_owned()]` in this task, so the binary verb and the service frame land
   together:

   ```rust
   args: vec!["serve".to_owned()],
   ```

3. **`IpcBackend::connect`** (`crates/sapphire-framework-backend/src/ipc.rs`) and
   **`BridgeClient::connect`** (`crates/sapphire-framework-bridge-api/src/client.rs`):
   same signature minus the `spawn: &SpawnConfig` parameter, body:

```rust
let (client, _) = sapphire_ipc::connect_or_absent(endpoint, app, info)
    .await?
    .ok_or_else(|| /* the crate's own error: "no <app> server is running" */)?;
```

   For `BridgeClient::connect` (which builds `Endpoint::in_dir(BRIDGE_NAME, runtime_dir)`
   itself), the error when absent names the bridge; callers in
   `crates/sapphire-framework-bridge/src/command.rs` already print "no bridge is running"
   from their own probe, and are not touched.

4. **Tests:** `tests/common/mod.rs` — replace the two `ensure_server` calls with
   `connect_or_absent(...).unwrap().unwrap()`; `SyncRuntime::new(..., ManagedBy::Service)`.
   `tests/concurrent.rs` and `tests/sync_wiring.rs` — same substitution; concurrent.rs's
   *purpose* (many CLIs, one server) is preserved by running the server as a task and
   having every client connect (the spawn race test itself is deleted with `race.rs`);
   `tests/privilege_root.rs` does not use spawn and stays as is.

- [ ] **Step 4: Run the tests to verify they pass**

Run:
```bash
cargo test -p sapphire-framework-ipc --all-features
cargo test -p sapphire-framework-server --all-features
cargo test -p sapphire-framework-backend --all-features
```
Expected: PASS everywhere, including the new SIGTERM test on Unix.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
git add -A
git commit -m "feat(server): stop on SIGTERM/SIGINT; remove start-on-demand"
```

---

### Task 3: `status` with the typed report, and the shape shared with IPC

**Files:**
- Modify: `crates/sapphire-framework-server/src/command.rs`
- Modify: `crates/sapphire-framework-server/src/lib.rs` (the `server.info` method gains
  the extension rows)
- Test: inside both files' test modules

**Interfaces:**
- Consumes: `sapphire_ipc::connect_or_absent`; the router's `SERVER_INFO` method.
- Produces:
  - `StatusReport` — `#[derive(Debug, Clone, Serialize, Deserialize)]`, the IPC
    `server.info` payload **and** the CLI's data:

    ```rust
    pub struct StatusReport {
        /// Whether a server is answering at all.
        pub running: bool,
        /// The server's version, when it is running.
        pub version: Option<String>,
        /// Its pid, when it is running.
        pub pid: Option<u32>,
        /// How the running server was started, when it is running.
        pub managed_by: Option<ManagedBy>,
        /// The application's own rows, rendered after the framework's.
        pub app: Vec<StatusRow>,
    }

    pub struct StatusRow {
        /// The row's name, e.g. `sync`.
        pub name: String,
        /// The value shown beside it.
        pub value: String,
    }
    ```

    The CLI prints `name: value` lines and exits 1 with
    `no {app} server is running` when `running` is false.
  - `FrameworkCommand::Status` renders it via `dispatch`.
  - `AppServer::status_rows(mut self, rows: Arc<dyn Fn() -> Vec<StatusRow> + Send + Sync>)`
    — the extension point; default empty. The `server.info` router method serialises the
    same `StatusReport`, so CLI and (future) GUI read one shape.

**Design notes:**
- `SERVER_INFO` already exists in `sapphire_backend::protocol` and returns `ServerInfo`
  today. This task widens it: the response becomes `StatusReport` (with the same
  `version` / `pid` / `managed_by` fields inside), because the CLI needs one call, not
  two. `ServerInfo` in `sapphire-framework-ipc/src/handshake.rs` stays exactly as it is —
  it is the *handshake* record, not the status report.
- The liveness probe and the version check are both inside `connect_or_absent`, so
  `status` is: probe → handshake → fill the report → print → exit code. When nothing
  listens, the report is `running: false` and the app rows are skipped.

- [ ] **Step 1: Write the failing tests**

In `command.rs`'s test module:

```rust
#[tokio::test]
async fn status_reports_no_server_when_none_is_running() {
    let tmp = tempfile::tempdir().unwrap();
    let endpoint = sapphire_ipc::Endpoint::in_dir("status-test", tmp.path().to_path_buf());
    let code = run_status(&endpoint, "status-test", "0.0.0").await.unwrap();
    assert_eq!(code, 1, "no server is a non-zero exit");
}

#[tokio::test(flavor = "multi_thread")]
async fn status_shows_the_extension_rows_of_a_running_server() {
    // Uses `test_support`'s env lock like lib.rs's tests, or the `server-test-app` bin:
    // start an AppServer with `.status_rows(...)` returning one fixed row, then assert
    // `run_status` prints it and exits 0.
}
```

In `lib.rs`'s test module:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn server_info_carries_the_app_rows() {
    let f = prepared();
    let endpoint = f.endpoint.clone();
    let server = AppServer::new(&CTX, "0.0.0")
        .endpoint(endpoint.clone())
        .status_rows(std::sync::Arc::new(|| {
            vec![StatusRow { name: "sync".into(), value: "enabled".into() }]
        }));
    let handle = tokio::spawn(async move { server.run().await });
    wait_until_listening(&endpoint).await;
    let (client, _) = sapphire_ipc::connect_or_absent(&endpoint, CTX.app_name, client_info())
        .await
        .unwrap()
        .unwrap();
    let report: StatusReport = client
        .call(sapphire_backend::protocol::SERVER_INFO, serde_json::json!({}))
        .await
        .unwrap();
    assert!(report.running);
    assert_eq!(report.app.len(), 1);
    assert_eq!(report.app[0].name, "sync");

    let _: serde_json::Value = client
        .call(sapphire_ipc::SHUTDOWN_METHOD, serde_json::json!({}))
        .await
        .unwrap();
    handle.await.unwrap().unwrap();
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sapphire-framework-server --all-features`
Expected: FAIL — `StatusReport` / `status_rows` / the widened `server.info` do not exist.

- [ ] **Step 3: Implement**

1. Define `StatusReport` / `StatusRow` in `command.rs` (re-export from `lib.rs`); they
   serialise with serde so the IPC payload and the CLI output share the shape.
2. Widen the `SERVER_INFO` method in `run()`'s router to build a `StatusReport` from the
   static `ServerInfo` plus `status_rows()`'s output; `AppServer` gains the
   `status_rows: Option<Arc<dyn Fn() -> Vec<StatusRow> + Send + Sync>>` field and builder.
   `ServerInfo`'s `managed_by` is always `Service` after Task 2, so the report is
   well-formed without the old `Spawned` variant.
3. `command.rs::status(endpoint, app, version)` becomes: `connect_or_absent` → on `None`
   print `no {app} server is running`, exit 1 → on `Some`, call `SERVER_INFO`, print
   `running: true`, `version: …`, `pid: …`, `managed_by: …`, then each `app` row as
   `name: value`, exit 0. The private helper stays; `dispatch`'s `Status` arm calls it
   with the endpoint built from the server's `AppContext`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p sapphire-framework-server --all-features`
Expected: PASS — the widened report test, the status tests, and everything from Tasks
1–2 still green.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test -p sapphire-framework-server --all-features
git add crates/sapphire-framework-server
git commit -m "feat(server): a typed status report shared by CLI and IPC"
```

---

### Task 4: The workspace commands on the app server; workgroup/device on the bridge

**Files:**
- Modify: `crates/sapphire-framework-backend/src/protocol.rs` (the `workspace.init`
  method, params, result)
- Modify: `crates/sapphire-framework-server/src/lib.rs` (the router gains
  `workspace.init`, wired to the registry)
- Modify: `crates/sapphire-framework-server/src/command.rs` (new variants + arms)
- Modify: `crates/sapphire-framework-bridge-api/src/client.rs` (shared client methods)
- Test: `crates/sapphire-framework-bridge-api/tests/client_commands.rs` (new),
  `crates/sapphire-framework-server/src/command.rs` parse + behaviour tests,
  `crates/sapphire-framework-server/src/lib.rs` `workspace.init` handler test

**Interfaces:**
- Consumes: `sapphire_ipc::connect_or_absent`; the existing `sync.enable` / `sync.map`
  methods (`sapphire_backend::protocol::{SYNC_ENABLE, SYNC_MAP, SyncMapParams}`); the
  bridge's existing IPC methods and param types (`bridge.workspaces` →
  `WorkspacesResult`, `bridge.peers` → `PeersResult`, `bridge.status` → `StatusResult`,
  `bridge.invite` → `InviteParams`/`InviteResult`, `bridge.join` → `JoinParams`/
  `JoinResult`) — the bridge side of the wire does not change (Phase 2 re-homes the
  *CLI*, not the protocol); `WorkspaceRegistry` (insert / ids / display_name / get) and
  the sync id helper (`sync_id`) for the server-side init.
- Produces:
  - `FrameworkCommand` gains `Workspace(WorkspaceCommand)`, `Workgroup(WorkgroupCommand)`,
    `Device(DeviceCommand)`:

    ```rust
    #[derive(Debug, clap::Subcommand)]
    pub enum WorkspaceCommand {
        /// Create this app's workspace home in the given directory.
        Init {
            /// Where the workspace root goes.
            dir: Option<std::path::PathBuf>,
            /// Also start syncing it, which publishes it into the workgroup ledger.
            #[arg(long)]
            sync: bool,
        },
        /// List this app's workspaces: local rows first, then the workgroup's.
        List,
        /// Tie a local directory to a workspace the workgroup knows.
        Map {
            /// The workspace, by name or id, as the workgroup lists it.
            selector: String,
            /// The local directory to map it to. Defaults to the current one.
            dir: Option<std::path::PathBuf>,
        },
    }

    #[derive(Debug, clap::Subcommand)]
    pub enum WorkgroupCommand {
        /// Found a workgroup on this host.
        Create { name: String, #[arg(long)] device_name: String },
        /// Show the workgroup this host belongs to.
        List,
        /// Join the workgroup a ticket names.
        Join { ticket: String, #[arg(long)] device_name: Option<String> },
    }

    #[derive(Debug, clap::Subcommand)]
    pub enum DeviceCommand {
        /// List the workgroup's devices, and which are reachable.
        List,
        /// Create an invite ticket for a device that is about to join.
        Invite {
            #[arg(long)] name: String,
            #[arg(long)] ttl: Option<u64>,
            #[arg(long)] workgroup: Option<String>,
        },
        /// Retire a device, so it may no longer connect.
        Retire { selector: String },
    }
    ```
  - `WORKSPACE_INIT: &str = "workspace.init"` in `sapphire_backend::protocol`, with

    ```rust
    /// Parameters of [`WORKSPACE_INIT`].
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct WorkspaceInitParams {
        /// Where the workspace root goes, relative to nothing: absolute, or relative
        /// to the process's cwd, resolved by the server.
        pub dir: PathBuf,
    }

    /// Result of [`WORKSPACE_INIT`].
    #[derive(Clone, Debug, Deserialize, Serialize)]
    pub struct WorkspaceInitResult {
        /// The workspace's canonical root.
        pub root: PathBuf,
        /// Its stable id, as the registry keys it.
        pub workspace_id: String,
        /// `false` when the workspace already existed (exit 0 either way).
        pub created: bool,
    }
    ```

  - The app server's `workspace.init` handler: resolve + canonicalise `dir`, derive the
    sync id (`sync_id(app_name, root)`), create the marker directory idempotently, add
    the registry entry (`WorkspaceRegistry::insert`) into the marker's `config.toml`,
    return `WorkspaceInitResult`. The CLI's `workspace init` connects to
    `Endpoint::for_app(app)` via `connect_or_absent` and calls it — the **server** does
    the creating, so the server already knows the workspace afterwards, which is what a
    GUI sharing the registry needs (spec decision 1/7). Nothing is listening → print
    `no {app} server is running`, exit 1, never spawn (Global Constraints).
  - `--sync`: after a successful `workspace.init`, the CLI calls the existing
    `SYNC_ENABLE` (`WsParams { ws: root }`) on the same connection. That runs the
    server's existing `SyncRuntime::enable` → replica + `reregister()` →
    `bridge.register` path, which is what publishes the workspace into the workgroup
    ledger. No new bridge-side work; `--sync` without a joined workgroup degrades
    exactly as `sync.enable` does today (registration still succeeds — it is the
    workgroup publish that has nothing to publish into, logged and non-fatal).
  - `BridgeClient::connect_running(kind, version) -> Result<BridgeClient>` — like
    `connect` after Task 2, but *never starts* anything; absence is an error naming the
    bridge ("no sapphire-bridge is running"), because asking must not bring a daemon up.
  - `BridgeClient::workspaces()`, `peers()`, `status()`, `invite(InviteParams)`,
    `join(JoinParams)` — already exist; `register`/`unregister` are app-server-only and
    are **not** exposed to these commands.

**Routing (spec decisions 1, 6, 7):**

| Command | Talks to | Why |
|---|---|---|
| `workspace init` / `init --sync` | the app's server (`workspace.init`, then `sync.enable`) | the server owns the marker, the registry and the sync state; creating must leave the server knowing |
| `workspace list` | the app's server (registry rows, via a `workspace.list`-shaped read or the status rows) **then** the bridge (`bridge.workspaces`) | local first, remote second |
| `workspace map` | the bridge (`bridge.workspaces`, to resolve `selector`) **then** the app's server (`sync.map`) | the resolution is the workgroup's word, the write is the server's |
| `workgroup *`, `device *` | the bridge only | the bridge owns the workgroup ledger end to end |

- `device retire` / `workgroup create` work directly on the bridge directory in the
  bridge's own CLI today, and the control plane has no method for them. The `-server`
  crate does not depend on `-bridge`, and this plan does not add that dependency:
  Phase 1's `workgroup create` / `device retire` print
  `run: sapphire-bridge workgroup create …` / `sapphire-bridge device retire …` and exit
  1. Phase 2 re-homes the bridge CLI onto this command system and takes the verbs over.
  `workspace list`'s remote layer degrades the same way when no bridge is running:
  local rows still print, the bridge layer prints `no sapphire-bridge is running` and
  the exit code is 0 (the local half succeeded) — `map` needs the bridge, so *it* exits 1.

**Design notes:**
- `device invite` prints only the ticket on its own line (the bridge CLI's convention —
  it is the output users pipe); `workgroup join` prints the joined workgroup and device.
- The verb is `retire`, matching the ledger's word (`devices.retire`), not `forget`.
- `workspace init`'s directory default: `dir` given → that path; omitted → the current
  directory. The idempotence ("already existed") lives in the server's handler, keyed on
  the marker directory already being present.
- `Workspace::init` (the local-only helper sketched in the first draft of this plan) is
  dropped: with `workspace.init` on the server, the workspace crate gains no new public
  API in this task. The handler composes the marker creation and the registry insert
  from `Workspace`'s existing paths (`marker_dir()`, `config_path()`) and
  `WorkspaceRegistry`.

- [ ] **Step 1: Write the failing tests**

In `command.rs`'s parse test block, extend `Probe`'s assertions:

```rust
#[test]
fn the_workspace_and_bridge_commands_parse() {
    for args in [
        vec!["app", "workspace", "init"],
        vec!["app", "workspace", "init", "papers"],
        vec!["app", "workspace", "init", "--sync"],
        vec!["app", "workspace", "list"],
        vec!["app", "workspace", "map", "papers", "papers-remote"],
        vec!["app", "workgroup", "create", "--device-name", "laptop", "home"],
        vec!["app", "workgroup", "list"],
        vec!["app", "workgroup", "join", "--device-name", "laptop", "TICKET"],
        vec!["app", "device", "list"],
        vec!["app", "device", "invite", "--name", "phone"],
        vec!["app", "device", "retire", "phone"],
    ] {
        assert!(Probe::try_parse_from(&args).is_ok(), "{args:?}");
    }
}
```

In `lib.rs`'s test module (the handler itself):

```rust
#[tokio::test(flavor = "multi_thread")]
async fn workspace_init_creates_the_marker_and_tells_the_registry() {
    let f = prepared();
    let endpoint = f.endpoint.clone();
    let server = AppServer::new(&CTX, "0.0.0").endpoint(endpoint.clone());
    let handle = tokio::spawn(async move { server.run().await });
    wait_until_listening(&endpoint).await;
    let (client, _) = sapphire_ipc::connect_or_absent(&endpoint, CTX.app_name, client_info())
        .await
        .unwrap()
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let result: WorkspaceInitResult = client
        .call(
            proto::WORKSPACE_INIT,
            proto::WorkspaceInitParams { dir: dir.path().to_owned() },
        )
        .await
        .unwrap();
    assert!(result.created);
    assert!(dir.path().join(format!(".{}", CTX.app_name)).is_dir());

    // Idempotent: the second init is a success that did not create.
    let again: WorkspaceInitResult = client
        .call(
            proto::WORKSPACE_INIT,
            proto::WorkspaceInitParams { dir: dir.path().to_owned() },
        )
        .await
        .unwrap();
    assert!(!again.created);

    let _: serde_json::Value = client
        .call(sapphire_ipc::SHUTDOWN_METHOD, serde_json::json!({}))
        .await
        .unwrap();
    handle.await.unwrap().unwrap();
}
```

`crates/sapphire-framework-bridge-api/tests/client_commands.rs` (new) — pins the
client-side helpers against the real bridge types:

```rust
//! The workgroup/device commands' client half.

use sapphire_bridge_api::{InviteParams, WorkspacesResult};

#[test]
fn invite_params_carry_the_cli_arguments() {
    let params = InviteParams {
        name: "phone".into(),
        ttl: Some(600),
        workgroup: None,
    };
    // Serialisation shape is the contract with the bridge's handler; pin it.
    let json = serde_json::to_value(&params).unwrap();
    assert_eq!(json["name"], "phone");
    assert_eq!(json["ttl"], 600);
    assert!(json.get("workgroup").is_none());
}

#[test]
fn workspaces_result_round_trips() {
    let raw = serde_json::json!({ "workspaces": [] });
    let parsed: WorkspacesResult = serde_json::from_value(raw).unwrap();
    assert!(parsed.workspaces.is_empty());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sapphire-framework-backend --all-features workspace_init &&
cargo test -p sapphire-framework-server --all-features 'the_workspace_and_bridge' &&
cargo test -p sapphire-framework-bridge-api --all-features`
Expected: FAIL — the method constants, the variants, `connect_running` and the
`workspace.init` handler do not exist.

- [ ] **Step 3: Implement**

1. `sapphire_backend::protocol`: `WORKSPACE_INIT`, `WorkspaceInitParams`,
   `WorkspaceInitResult` as specified; the `workspace.`-prefix test's list gains
   `WORKSPACE_INIT` so the naming rule is enforced for the new method too.
2. The `workspace.init` handler in `lib.rs`'s router assembly (beside the existing
   `workspace.*` methods): resolve `dir` (absolute, or against the server's cwd),
   canonicalise, build the marker + registry entry with `WorkspaceRegistry::insert`,
   return `WorkspaceInitResult { created }`. `created` is false when the marker was
   already there.
3. `BridgeClient::connect_running(kind, version)`: like today's `connect` but built on
   `sapphire_ipc::probe` + `connect_or_absent`; no `SpawnConfig` (already gone in Task 2),
   and absence is an error the command layer prints as "no sapphire-bridge is running".
   Keep `from_client` unchanged (tests and the sync runtime use it).
4. The new enum variants and their `dispatch` arms per the routing table above; each arm
   is a small async fn in `command.rs` calling `connect_or_absent` (app server) or
   `connect_running` (bridge) then one client method, printing as specified. The
   app-server endpoint comes from `Endpoint::for_app(app)`; the bridge endpoint is what
   `BridgeClient::connect_running` builds.
5. `lib.rs` re-exports the three new enums beside `FrameworkCommand`.

- [ ] **Step 4: Run the tests to verify they pass**

Run:
```bash
cargo test -p sapphire-framework-backend --all-features
cargo test -p sapphire-framework-server --all-features
cargo test -p sapphire-framework-bridge-api --all-features
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
git add -A
git commit -m "feat(server): workspace commands on the app server; workgroup/device on the bridge"
```

---

### Task 5: Facade re-export, architecture docs, and the full sweep

**Files:**
- Modify: `crates/sapphire-framework/src/lib.rs` (prelude)
- Modify: `docs/ARCHITECTURE.md` (the `ServerCommand` / start-on-demand descriptions)
- Test: workspace-wide

**Interfaces:**
- Consumes: everything above.
- Produces: the facade prelude exports `FrameworkCommand` (+ the sub-enums and
  `StatusReport` / `StatusRow`) where `ServerCommand` used to be; `ARCHITECTURE.md`
  describes the flat command vocabulary, the always-on service under SIGTERM/SIGINT,
  the removed start-on-demand, and the routing split — workspace commands on the app
  server, workgroup/device on the bridge (decision 5's revision is already recorded in
  the spec; this fixes the prose to match the code).

- [ ] **Step 1: Write the failing test**

```bash
grep -n "ServerCommand" crates/sapphire-framework/src/lib.rs
```

Expected: output naming the stale re-export (that is the failing state — a build after
Task 1 already fails on it; this step makes the work visible).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p sapphire-framework --all-features`
Expected: FAIL — the prelude re-exports names that no longer exist.

- [ ] **Step 3: Implement**

1. Swap the prelude's `ServerCommand` re-export for `FrameworkCommand` and companions.
2. Update `docs/ARCHITECTURE.md`: the process-architecture section (around the
   `ServerCommand` mention at ~line 95 and the bridge CLI at ~line 184) — describe
   `<app> serve|status|service|workspace|workgroup|device`, the always-on service under
   SIGTERM/SIGINT, the `SpawnConfig`/idle-exit removal, the ownership split
   (app server ↔ bridge), and note Phase 2 (the bridge CLI's re-homing) as the follow-up.

- [ ] **Step 4: Run the tests to verify they pass**

Run:
```bash
cargo test --all-features --locked
```
Expected: PASS — the whole workspace, all three CI gates green.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
git add -A
git commit -m "docs(plan): finish the app command system sweep"
```

## Risks

- **clap flatten + subcommand composition** (Task 1): the spec mandates the parse test
  first for exactly this reason. If clap 4 rejects `#[command(flatten)]` beside
  `#[command(subcommand)]`, the fallback is the app taking
  `#[command(subcommand)] command: Option<Command>` where `Command` is an app-owned enum
  with `#[command(flatten)] Framework(FrameworkCommand)` — still one top level, still
  flat, decided in Task 1 by the test.
- **SIGTERM in tests** (Task 2): delivering a real signal to the test process kills the
  test harness if the handler is not installed yet. The test waits for the socket (the
  handler is registered before the bind) and is Unix-only. If flaky in CI, fall back to
  the `stop_rx` path (IPC `server.shutdown`) as the observable half and test the signal
  arm by spawning a child `server-test-app` process and asserting its exit.
- **`connect_or_absent` and the version-mismatch error** (Task 2): the "restart the
  service" message is asserted by `race.rs` today, which is being deleted; migrate that
  assertion into `roundtrip.rs` rather than dropping it.
- **`workspace.init` writes the app's config file** (Task 4): the handler edits the
  marker's `config.toml` while the server may also hold the config in memory elsewhere;
  read-modify-write the file through `toml` with the same serialisation the apps use,
  and pin the idempotence in the handler test. If a future GUI holds a long-lived copy
  of the registry, a later issue adds change notification — out of scope here.
- **`--sync` on an unjoined host** (Task 4): `sync.enable` succeeds and registers with
  the bridge even without a workgroup, so `workspace init --sync` on a fresh host must
  not surprise the user; the CLI prints the workspace's sync state (from `sync.status`)
  after enabling, which says plainly that no workgroup knows it yet.
- **`StatusReport` widens `SERVER_INFO`** (Task 3): any existing consumer of the IPC
  method that is not in this workspace (none known — the workspace is first-party) would
  see the extra `running` / `app` fields; serde defaulting keeps old readers alive.

## Open questions

- None blocking. The judgement calls made here and why: (a) `workspace list` prints
  local rows first and the bridge's ledger second, with the bridge's absence degrading
  to a message rather than a failure — the local half is the app's own truth, the remote
  half is the workgroup's; (b) `device retire` / `workgroup create` degrade to a printed
  directive in Phase 1 rather than pulling `-bridge` into `-server`'s dependency tree —
  the bridge's control plane has no method for them, and Phase 2, which re-homes the
  bridge CLI anyway, decides their final home; (c) `workspace init`'s local-only helper
  in the workspace crate was dropped in favour of a server-side `workspace.init` RPC, so
  the server is the one place that creates and knows the workspace (GUI + CLI share it).
