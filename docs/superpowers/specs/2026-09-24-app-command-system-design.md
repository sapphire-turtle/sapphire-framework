# App command system: framework-provided flat CLI (serve / status / service / workspace / workgroup / device)

- Date: 2026-09-24
- Scope: `sapphire-framework-server` (command system), `sapphire-framework-ipc`
  (spawn machinery removal, signal handling), `sapphire-framework-bridge-api`
  (shared IPC client methods). Phase 2 (separate issue): re-homing the bridge CLI onto
  the shared foundation.
- Revises: [`2026-09-16-process-architecture-design.md`](./2026-09-16-process-architecture-design.md)
  decision 5 — servers are always-on daemons; start-on-demand is replaced by
  "the service manager starts the one server process".
- Related: sapphire-framework issue #142; follow-up migrations of journal / ledger /
  tally / sapphire-sync CLIs happen in their own repositories.

## Background

`crates/sapphire-framework-server/src/command.rs` currently nests
`ServerCommand { Run, Status, Stop, Service }` under a `server` subcommand, with
`["server", "run"]` hard-coded into the service spec and `SpawnConfig`. The bridge CLI
(`sapphire-framework-bridge/src/command.rs`, 860 lines) already implements the target
shape — bare invocation starts the daemon, flat top-level commands — but only for the
bridge, in its own implementation.

The goal is for the framework to provide the common command system directly, so every
app's CLI speaks one vocabulary, and future GUIs reach the same surfaces over the same
IPC. The bridge remains the owner of workgroups and devices; the actual workspace
read/write stays on the app side (app server owns its workspace files).

## Decisions

Agreed during brainstorming on 2026-09-24:

1. **One command vocabulary, provided by the framework.** Every app binary gets:

   ```
   <app> serve                       — start the server (bare invocation == serve)
   <app> status                      — liveness + app-specific state
   <app> service install|uninstall|status
   <app> workspace init|list|map     — create / list / remote→local mapping
   <app> workgroup create|list|join
   <app> device list|invite|retire
   ```

   The `workspace` commands are served primarily over **the app's own server IPC**,
   not the bridge: the ledger of an app's workspaces is the app's registry
   (`WorkspaceRegistry`, embedded as `[workspace.<id>]` tables in the config that lives
   in the workspace marker directory), and the process that owns that file is the app
   server. `workspace init` therefore requires the app server to be running — it is
   where the marker/sync-id creation and the registry entry are executed, so a
   workspace created through the CLI or the GUI is one the server already knows.
   Only `workspace list`'s second layer (the workgroup-wide list) and `workspace map`'s
   name resolution consult the bridge ledger, as described in decision 6.

2. **Flat merge via clap flatten, not double nesting.** The framework exposes a
   `FrameworkCommand` `#[derive(Subcommand)]` enum; app CLIs compose it as a second
   field next to their own `#[command(subcommand)]` enum, so app-specific commands
   (`entry list`, `preset edit`, …) sit at the same top level as framework commands.

   ```rust
   #[derive(Parser)]
   struct Cli {
       #[command(subcommand)]
       app: AppCommand,             // app-specific: entry / cache / ...
       #[command(flatten)]
       framework: FrameworkCommand, // serve / status / service / workspace / ...
   }
   ```

   The framework provides a dispatch helper for its own variants; apps match only their
   own. (clap 4 supports flattening a `Subcommand` into a container with another
   subcommand; this is verified by a parse test in the first task.)

3. **Always-on daemons; start-on-demand is removed.** `serve` takes no flags and always
   runs in the foreground under a service manager. The idle-exit machinery
   (`idle_exit`, `ManagedBy::Spawned` semantics as an operating mode) and the
   spawn-on-demand machinery (`ensure_server` / `SpawnConfig` re-exec) are removed.
   Startup comes from the installed service; graceful shutdown comes from
   **SIGTERM/SIGINT handling in `run()`** (today the loop only watches the IPC stop
   channel; signal handling is new work). Commands that need a running server
   (`workspace init`, `workspace map`, `status` beyond liveness) therefore report
   "no server is running" and exit 1 rather than starting one — there is no
   start-on-demand to fall back on, and starting a daemon as a side effect of a
   one-shot command is exactly what the removal is about.

4. **`stop` is dropped.** Start/stop/restart of the daemon is the service manager's
   business (`systemctl --user stop <app>`). The framework CLI does not re-implement a
   second way to stop; the IPC `server.shutdown` path loses its reason to exist as a CLI
   command (it may remain as the internal stop mechanism behind SIGTERM handling —
   implementation detail, not a CLI surface).

5. **`status` = liveness + extension point.** The common part keeps the current
   semantics (running / version / pid / managed_by, exit 1 with a message when down).
   Apps inject app-specific state (e.g. sync status: enabled/id/peers/paused) through a
   typed **status report** the dispatch helper renders. The same structure is what the
   IPC status response carries, so CLI and (future) GUI render one shape.

6. **Control-plane routing follows ownership.** `workspace` commands talk to the
   process that owns the state they touch: `workspace init` and `workspace map`'s
   write go to **the app's server** (it owns the marker directory, the registry and the
   sync ids); `workspace list` shows the local registry first and then the workgroup's
   ledger; the workgroup-wide ledger (what exists in the workgroup, replicated into
   `workgroups/<id>/root/workspaces/*.toml`) is read over the bridge endpoint —
   `Endpoint::in_dir("sapphire-bridge", runtime_dir())`, the bridge's IPC methods
   (`bridge.workspaces` for the list, `bridge.*` as `map`'s selector resolution back-end)
   — directly, not relayed through the app server. In Phase 1 the *local* layer of
   `workspace list` is read by the CLI directly from the marker's `config.toml` (it needs
   no server process and stays useful when one is not running); whether to move that read
   behind the app server's IPC in Phase 2 is open. `workgroup` and `device` commands are
   the bridge's business end to end: they open the bridge endpoint and call the bridge's
   IPC methods directly (path 1, unchanged). Every one of these commands reports clearly
   when the process it needs is not running: no server, no start-on-demand, exit 1 with
   a message naming the process. The bridge's existing method namespace
   (`bridge.register/unregister/peers/status/invite/join/workspaces`) is the back-end,
   extended/renamed as the shared surface requires. A thin shared IPC client for these
   methods ships with the framework (used by app CLIs now, by GUIs later).

7. **`workspace init` not `new`** — matches existing app verbs (`sapphire-journal init`)
   and the `git init` + `git clone` analogy: `init` creates the local (marker) home,
   `map` ties it to a remote workspace. `init` runs **on the app server over IPC**
   (decision 1): the CLI connects, the server creates the marker and sync id and adds
   the registry entry, so the workspace is immediately usable by the server and visible
   to a GUI sharing the same registry. Without `--sync` the bridge ledger is untouched;
   with `--sync` the server additionally runs its existing `sync.enable` path
   (replica + `reregister()` → `bridge.register`), which is what publishes the
   workspace into the workgroup ledger. `init` is idempotent: pointing it at an
   initialised directory reports "already exists" and exits 0.

8. **Bootstrap args de-hard-coded.** `service_spec()` emits `args: ["serve"]`; the
   `SpawnConfig` hard-coding disappears with the spawn machinery (decision 3). No
   per-app override is added — the frame is fixed at `serve`.

9. **Migration: cut over, no aliases.** `server run|status|stop` are removed outright;
   no deprecation aliases (the apps are all first-party and are being reworked for p2p
   sync anyway). journal / ledger / tally / sapphire-sync CLI migration is a separate
   follow-up in those repos.

10. **Phasing.** Phase 1 (this issue #142): the framework-side command system, status
    extension point, the workspace/workgroup/device commands over their owning
    processes' IPC, spawn/idle removal, signal handling. Phase 2 (separate issue): the
    bridge's own 860-line `command.rs` is re-homed onto the shared foundation (keeping
    its `log` subcommand as bridge-specific).

## Non-goals

- `service start|stop|restart` abstractions over systemd/launchd (decision 4).
- App-side syncing of workspace content (stays app-server-owned; this spec covers the
  control-plane commands only).
- GUI implementation (the design only ensures one IPC shape it can reuse).
- journal / ledger / tally / sapphire-sync migration details (separate repos).

## Testing

- Parse tests: `serve` / `status` / `service …` / `workspace …` / `workgroup …` /
  `device …` parse at top level **alongside** an app-specific subcommand through the
  flatten composition; `workspace init --sync` parses.
- `status`: exit code 1 + message when no server; liveness fields + app extension rows
  when running; the same report shape over CLI and IPC.
- `serve`: SIGTERM/SIGINT triggers graceful shutdown (socket removed, tasks aborted).
- `workspace init`: with a running app server, creates the marker + registry entry and
  the server knows the workspace afterwards; idempotent (second call: "already exists",
  exit 0); without a running server: exit 1 + message, nothing started; `--sync`
  additionally ends with the workspace present in the bridge ledger.
- `service_spec().args == ["serve"]`; spawn machinery removal keeps existing service
  install tests green after update.
- Migrate existing `command.rs` tests to the new shapes; no orphan tests left behind.
