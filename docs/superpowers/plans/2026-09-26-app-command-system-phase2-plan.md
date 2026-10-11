# App Command System Phase 2: the bridge CLI re-homed, and the leftovers swept — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> If your harness has no such skill, execute the tasks in order, one at a time, running the
> listed commands and committing at the end of each task. Do not skip the "run the test and
> watch it fail" steps: they are what proves the test exercises the new code.

- Date: 2026-09-26
- Scope: `sapphire-framework-bridge` (`command.rs` re-homed onto the shared vocabulary),
  `apps/sapphire-bridge` (default verb, READMEs), `sapphire-framework-ipc`
  (`Error::Spawn` renamed to what it means now), `sapphire-framework-server`
  (the dead live counter removed), `docs/ARCHITECTURE.md`.
- Revises: [`2026-09-24-app-command-system-design.md`](../specs/2026-09-24-app-command-system-design.md)
  decisions 1, 6 and 10 in their Phase 2 parts.
- Related: sapphire-framework issue #149; Phase 1 is issue #146 (plan:
  [`2026-09-24-app-command-system-plan.md`](./2026-09-24-app-command-system-plan.md), all
  landed). The journal / ledger / tally / sapphire-sync CLI migrations stay in their own
  repositories and are **not** touched here.
- Branch: work on `feat/app-command-system` (the current branch), on top of Phase 1.

**Goal:** The bridge's 860-line `command.rs` speaks the same flat command vocabulary every
app CLI now speaks — `serve` / `status` / `service` / `workspace` / `workgroup` / `device` —
with the bridge-specific `log` kept beside it. And the two leftovers Phase 1 left behind are
cleared: `sapphire_ipc::Error::Spawn` is renamed to what it actually means now
("nothing is listening"), and the write-only live-connection counter in `AppServer::run`
is deleted.

**Architecture — and the re-homing ruling.** The bridge keeps **its own enum with the same
shape**, `BridgeCommand`, rather than reusing `FrameworkCommand`. The decision comes from
the code, not taste:

1. `FrameworkCommand::dispatch(self, server: AppServer, version)` (Phase 1, Task 1) takes
   an `AppServer` — it is how `serve` gets `server.run()`, how `service` gets
   `server.service_spec()`, and how `status` / the workspace verbs get the app name. The
   bridge has no `AppServer` and must not: building one would drag `-backend`,
   `-workspace`, `-session` and `-sync` (and through the dev-deps, the bridge's own crate)
   into the host daemon.
2. The dependency direction is fixed both ways: `-bridge` must not depend on `-server`
   (layering — the bridge is the switchboard *above* app servers, `ARCHITECTURE.md`'s
   crate table), and `-server` must not depend on `-bridge` (Phase 1 declined exactly
   that to keep its `workgroup create` / `device retire` directives printed instead of
   executed).
3. The verbs do not even coincide: the bridge's `workspace` group is read-only `list`
   (spec §1 — placing a workspace is the owning application's business; the bridge's own
   test `there_is_no_way_to_map_a_workspace_from_here` pins this), while the app side has
   `init` / `list` / `map`.

What *is* shared is the vocabulary, and this plan pins it verb-by-verb: `Run` becomes
`Serve` (spec decision 1 — the bare invocation is `serve`; here that still means "start the
bridge in this process", and "already running" stays a normal outcome that reports the pid
and exits 1, because the bridge's single-instance guard is a lock, not a bind race).
`DeviceCommand::Forget` becomes `Retire`, the ledger's own word — the app-server side
already prints `run: sapphire-bridge device retire …`, so with the rename that directive
becomes a command that actually exists. Sub-enum shapes (`WorkgroupCommand`,
`DeviceCommand`, `WorkspaceCommand`) stay in each owner's crate: the parse tests on both
sides assert the same verb sets, and `ARCHITECTURE.md` lists the vocabulary once. The
alternative — moving the sub-enums into `sapphire-framework-bridge-api` so both crates
derive from one type — was rejected: bridge-api is the serde-only control-plane protocol
crate, and clap derive does not belong in it.

**The bare invocation and `serve` (spec decision 1, bridge edition).** `sapphire-bridge`
with no subcommand and `sapphire-bridge serve` are the same thing, exactly as `<app>` with
no subcommand is `<app> serve`. What differs from an app server is the *process semantics*,
and that is correct: an app server's `serve` is a bind race on its IPC endpoint, while the
bridge's `serve` takes `InstanceLock` first and reports `the bridge is already running
(pid N)` — a normal outcome a user can act on, exit 1, not an error. The bridge also
installs its file log and writes `status.json`; none of that belongs in a shared
dispatcher. The bridge's graceful SIGTERM/SIGINT handling (Phase 1 gave `AppServer::run`
signal arms; `serve_loops` has none) is deliberately **out of scope** — see Open questions.

**Tech Stack:** Rust 2024 (toolchain 1.98.0), clap 4 (derive), thiserror,
`sapphire-framework-ipc`, `sapphire-framework-bridge-api`, `sapphire-framework-service`,
`sapphire-framework-bridge`, `apps/sapphire-bridge`.

**Spec:** `docs/superpowers/specs/2026-09-24-app-command-system-design.md` — decision 10
(Phase 2: the bridge's own `command.rs` re-homed onto the shared foundation, keeping its
`log` subcommand as bridge-specific), decision 6's Phase 2 open point (see Open questions),
decision 9 (no aliases — `run` and `forget` are renamed outright).

**Depends on:** Phase 1, landed: `FrameworkCommand` + the flat sub-enums
(`sapphire-framework-server/src/command.rs`), `connect_or_absent`
(`sapphire-framework-ipc/src/spawn.rs`), `BridgeClient::connect_running`
(`sapphire-framework-bridge-api/src/client.rs`), signal-driven shutdown
(`sapphire-framework-server/src/lib.rs`).

## Global Constraints

- Code, comments, commit messages and tests in **English** (`CONTRIBUTING.md`).
- CI runs `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  and `cargo test --all-features --locked`. All three must pass after every task.
- Every public item carries a doc comment (`#![warn(missing_docs)]` is set crate-wide).
- **No aliases for the renamed commands** (spec decision 9): `sapphire-bridge run` and
  `device forget` are renamed to `serve` and `device retire` outright — a parse test pins
  that the old spellings no longer parse. The upgrade note in the Risks section covers the
  one operational consequence (an installed unit that still names `run`).
- The IPC crate stays free of the workspace/search/sync stacks — the
  `dependency_surface_is_documented` test in `roundtrip.rs` pins it; none of the tasks
  below adds a dependency anywhere.
- **Commands never start daemons** (spec decisions 3, 6, 7): a one-shot command that needs
  a process which is not running reports it and exits 1. The one designed exception is the
  bridge's `status`: when no bridge is live, it reports the last `status.json` snapshot
  (exit 0, marked stale) and only exits 1 when there has never been one. That fallback is
  the bridge's own design, kept as is.
- Every renamed or deleted item's tests are migrated in the same task — no orphan tests
  referencing old names may survive a task's commit.
- Documents under `docs/superpowers/` are dated historical records: this plan and the
  sweep do **not** edit them (the grep in Task 4 excludes them on purpose).

## File Structure

```
crates/sapphire-framework-ipc/src/
    error.rs           # MODIFIED (Task 1): Spawn(String) → NotRunning(String);
                       #   the message is the payload, the spawn-era prefix is gone

crates/sapphire-framework-backend/src/
    ipc.rs             # MODIFIED (Task 1): the one Spawn construction → NotRunning
                       #   (message text already correct: "`{app} serve`")

crates/sapphire-framework-bridge-api/src/
    client.rs          # MODIFIED (Task 1): the two Spawn constructions → NotRunning;
                       #   "start it with `sapphire-bridge run`" → "`sapphire-bridge serve`"

crates/sapphire-framework-server/src/
    lib.rs             # MODIFIED (Task 2): the write-only `live` AtomicU64 counter, its
                       #   clone in the accept arm and the atomic import are deleted

crates/sapphire-framework-bridge/src/
    command.rs         # REWRITTEN (Task 3): Run → Serve, Forget → Retire, the flat
                       #   vocabulary's order, "no sapphire-bridge is running" wording,
                       #   bridge_service_spec args ["serve"], updated tests
    lib.rs             # unchanged re-export surface (BridgeCommand + bridge_service_spec)

apps/sapphire-bridge/src/
    main.rs            # MODIFIED (Task 3): default BridgeCommand::Serve; doc comments
apps/sapphire-bridge/
    README.md          # MODIFIED (Task 3): run → serve, forget → retire, upgrade note
    README.ja.md       # MODIFIED (Task 3): same, in Japanese

docs/ARCHITECTURE.md   # MODIFIED (Task 4): the bridge CLI prose speaks the flat vocabulary
```

---

### Task 1: `Error::Spawn` is now `Error::NotRunning`

**Files:**
- Modify: `crates/sapphire-framework-ipc/src/error.rs`
- Modify: `crates/sapphire-framework-backend/src/ipc.rs`
- Modify: `crates/sapphire-framework-bridge-api/src/client.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `sapphire_ipc::Error::NotRunning(String)` — where `Error::Spawn(String)` is today,
    same tuple shape, so every `?`-conversion and `From` keeps working. The doc comment
    says what the variant means *now*; the render is the payload itself, because all three
    construction sites already carry full sentences.
  - The three construction sites keep their messages, with one wording fix:
    `sapphire-framework-backend/src/ipc.rs` already advises `` `{app} serve` ``; the
    `connect` error in `sapphire-framework-bridge-api/src/client.rs` advises
    `` `sapphire-bridge serve` `` instead of the dead `` `sapphire-bridge run` ``.

**Design notes:**
- Phase 1 removed the spawn machinery (`ensure_server` / `SpawnConfig`), leaving this
  variant meaning "the process you meant to talk to is not running" — but its name and its
  `could not start the server:` prefix still describe a starter that no longer exists.
- Safety proof before the rename: `Error::Spawn` has **no match sites** anywhere — only
  the definition and three constructions. The grep in Step 1 pins that, so the rename
  cannot break a `match` silently.

- [ ] **Step 1: Prove the blast radius**

```bash
grep -rn "Error::Spawn" crates apps --include='*.rs'
```

Expected — exactly four hits, all constructions or the definition, no `match` arms:

```
crates/sapphire-framework-backend/src/ipc.rs:50:                sapphire_ipc::Error::Spawn(format!(
crates/sapphire-framework-bridge-api/src/client.rs:40:                sapphire_ipc::Error::Spawn(
crates/sapphire-framework-bridge-api/src/client.rs:58:            return Err(sapphire_ipc::Error::Spawn(
crates/sapphire-framework-ipc/src/error.rs:57:    Spawn(String),
```

- [ ] **Step 2: Make the workspace fail (watch the rename bite)**

In `crates/sapphire-framework-ipc/src/error.rs`, replace the variant (currently
`error.rs:56-58`, between `Timeout` and `ServiceVersionMismatch`):

```rust
    /// The process this caller meant to talk to is not running.
    ///
    /// The payload is the sentence a caller prints or wraps: it names the process and,
    /// where starting one is the answer, the command that starts it (`serve`, or the
    /// service manager). Start-on-demand is gone (2026-09-24 spec decision 3), so
    /// nothing in this crate — and nothing behind this error — starts a process.
    #[error("{0}")]
    NotRunning(String),
```

(Deletes the old `/// The server could not be started.` doc comment and the
`could not start the server: {0}` render with it.)

- [ ] **Step 3: Fix the three construction sites**

1. `crates/sapphire-framework-backend/src/ipc.rs:50` — `Error::Spawn` →
   `Error::NotRunning`; the `format!("no {app} server is running; start it with
   `{app} serve` or install its service")` text stays as is (it is already correct).
2. `crates/sapphire-framework-bridge-api/src/client.rs:40` (`connect`) —
   `Error::Spawn` → `Error::NotRunning`, and the message's advice line becomes
   `"no bridge is running; start it with `sapphire-bridge serve` \"` — the rest of the
   sentence unchanged.
3. `crates/sapphire-framework-bridge-api/src/client.rs:58` (`connect_running`) —
   `Error::Spawn` → `Error::NotRunning`; `"no sapphire-bridge is running"` unchanged.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p sapphire-framework-ipc --all-features
cargo test -p sapphire-framework-backend --all-features
cargo test -p sapphire-framework-bridge-api --all-features
```

Expected: PASS — the rename is name-only; no test referenced `Spawn` (the Step 1 grep is
the proof, and `roundtrip.rs`'s error tests only touch `ServiceVersionMismatch` and the
codec paths).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
git add crates/sapphire-framework-ipc crates/sapphire-framework-backend crates/sapphire-framework-bridge-api
git commit -m "refactor(ipc): Error::Spawn is Error::NotRunning — nothing starts servers any more"
```

---

### Task 2: Delete the dead live-connection counter in `AppServer::run`

**Files:**
- Modify: `crates/sapphire-framework-server/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: none — pure deletion. `run()`'s accept arm no longer books connections into an
  `AtomicU64` nobody reads, and the crate's only atomic import goes with it.

**Design notes — where it was found and why it is dead:**
- The counter lives at `crates/sapphire-framework-server/src/lib.rs`: created at line 265
  (`let live = Arc::new(AtomicU64::new(0));`), written at lines 399 / 403 / 406
  (`fetch_add` on accept, `Arc::clone` for the spawned task, `fetch_sub` when `serve`
  returns).
- Its only *reader* was the idle-exit ticker of the pre-Phase-1 `run()`
  (`if live.load(Ordering::Relaxed) > 0` — visible in the pre-Phase-1 tree, e.g. commit
  `2e7ad0b`'s `lib.rs` around line 303). Commit `a07687b` ("stop on SIGTERM/SIGINT; remove
  start-on-demand") deleted the idle ticker and with it the reader. Writes alone keep the
  compiler happy and clippy silent — which is why Phase 1's review flagged it as a
  leftover rather than the compiler catching it.
- Not to be confused with namesakes that are alive and pinned by tests: `tests/live.rs`
  (the loop-detection tests), the peer frame counters in `-bridge/src/peer.rs`, and the
  dial-backoff counter in `-server/src/sync/mod.rs`. None of those is touched.

- [ ] **Step 1: Prove the counter is write-only**

```bash
grep -n "AtomicU64\|live\.fetch\|use std::sync::atomic" crates/sapphire-framework-server/src/lib.rs
```

Expected — one import, one construction, three write sites, **no `load`**:

```
24:use std::sync::atomic::{AtomicU64, Ordering};
265:        let live = Arc::new(AtomicU64::new(0));
399:                    live.fetch_add(1, Ordering::Relaxed);
403:                    let live = Arc::clone(&live);
406:                        live.fetch_sub(1, Ordering::Relaxed);
```

(A wider `grep -rn "live\.load" crates/ --include='*.rs'` shows no reader of *this*
counter either — the only `load`s are `-bridge`/peer.rs's frame counters, which have their
own tests.)

- [ ] **Step 2: Delete**

1. Line 24: remove `use std::sync::atomic::{AtomicU64, Ordering};` (no other use of either
   name in the file — the Step 1 grep is the proof).
2. Line 265: remove `let live = Arc::new(AtomicU64::new(0));`.
3. Lines 399-407: the accept arm loses the counter and its plumbing. After:

```rust
                accepted = listener.accept() => {
                    let conn = accepted?;
                    let router = Arc::clone(&router);
                    let app = ctx.app_name;
                    let info = info.clone();
                    connections.spawn(async move {
                        let _ = serve(conn, router, app, info).await;
                    });
                }
```

4. Sanity: `grep -n "AtomicU64\|live\.fetch\|sync::atomic" crates/sapphire-framework-server/src/lib.rs`
   → no output.

- [ ] **Step 3: Run the tests to verify they pass**

```bash
cargo test -p sapphire-framework-server --all-features
```

Expected: PASS — deleting write-only code has no observable behavior; the connection
lifecycle tests (`concurrent.rs`, `command_lifecycle.rs`) exercise exactly the code that
stays. No new test: there is nothing to assert about a counter that does not exist.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
git add crates/sapphire-framework-server
git commit -m "refactor(server): drop the live-connection counter whose reader the idle exit took with it"
```

---

### Task 3: The bridge CLI re-homed onto the flat vocabulary

**Files:**
- Rewrite: `crates/sapphire-framework-bridge/src/command.rs`
- Modify: `apps/sapphire-bridge/src/main.rs`
- Modify: `apps/sapphire-bridge/README.md`, `apps/sapphire-bridge/README.ja.md`
- (`crates/sapphire-framework-bridge/src/lib.rs` keeps its re-export line
  `pub use command::{BridgeCommand, bridge_service_spec};` unchanged — the names stay.)

**Interfaces:**
- Consumes: `sapphire_framework_service::{ServiceCommand, Environment, SystemManager}`;
  `sapphire_bridge_api::BridgeClient` and its `peers` / `status` / `invite` / `join` /
  `workspaces` methods; `InstanceLock`, `Workgroup`, `StatusFile`, the `logging` module —
  all already in the crate; `Endpoint::in_dir(BRIDGE_NAME, runtime_dir())` via the
  crate's own private `connect()` helper (kept: its `Option` shape is what `status`'s
  snapshot fallback needs, and absence must be a reported outcome, not an error chain).
- Produces:
  - `BridgeCommand` — `#[derive(Debug, Default, clap::Subcommand)]` with the shared
    vocabulary's shape and the bridge's one extra verb:

    ```rust
    /// The bridge daemon's commands.
    ///
    /// The same flat vocabulary every sapphire CLI speaks (2026-09-24 spec decisions
    /// 1/2), with this bridge's own `log` beside it. The bridge keeps its own enum
    /// rather than reusing the app side's `FrameworkCommand`: that dispatcher takes an
    /// `AppServer`, and this crate is the switchboard *above* app servers — it must not
    /// depend on the `-server` crate. Two verbs work directly on the bridge directory
    /// instead of over IPC: `workgroup create`, because there is nothing to ask about a
    /// workgroup that does not exist yet, and `device retire`, because the control
    /// plane has no method for it. Both see their effect at once: the ledger is
    /// re-read on every authorization.
    #[derive(Debug, Default, clap::Subcommand)]
    pub enum BridgeCommand {
        /// Run the bridge in this process (the bare invocation).
        #[default]
        Serve,
        /// Report what the bridge knows — live, or from the last snapshot it wrote.
        Status,
        /// Show the bridge's log. Bridge-specific: only this process writes one.
        Log {
            /// Keep printing as the log grows, as `tail -f` does.
            #[arg(long)]
            follow: bool,
            /// How many lines of the log's end to show.
            #[arg(long, default_value = "20")]
            lines: usize,
        },
        /// Install, remove or report this host's bridge service.
        #[command(subcommand)]
        Service(ServiceCommand),
        /// What the workgroup contains — read-only.
        #[command(subcommand)]
        Workspace(WorkspaceCommand),
        /// This host's workgroup.
        #[command(subcommand)]
        Workgroup(WorkgroupCommand),
        /// The workgroup's devices.
        #[command(subcommand)]
        Device(DeviceCommand),
    }
    ```

    Order follows the shared vocabulary (`serve`, `status`, …, `service`, `workspace`,
    `workgroup`, `device`) with `log` after the shared pair it extends. Variant order is
    declaration order for clap's help, so this is also the help text's order.
  - `DeviceCommand::Retire { selector }` — where `Forget` is today; doc comment keeps the
    tombstone rationale ("The record stays as a tombstone: a device id is written into
    synced content and must keep resolving."). The private fn `device_forget` becomes
    `device_retire`; body unchanged (`Workgroup::open` + `devices()?.retire(selector)`).
  - `WorkgroupCommand` and `WorkspaceCommand` — shapes unchanged (`create`/`list`/`join`,
    and read-only `list`); only doc-comment wording is refreshed.
  - `bridge_service_spec(version)` — `args: vec!["serve".to_owned()],` (was `"run"`);
    `RunAs::InvokingUser` and `privileges: None` unchanged (a root bridge would put the
    bridge directory under `/root`, and the bridge has no filesystem access to separate).
    The doc comment's "The unit runs the bare binary with `run`" becomes `serve`.
  - `dispatch(self, version: &'static str) -> Result<i32>` — same signature as today
    (the bridge's own `Error`; no `AppServer` to hand in), arms renamed:

    ```rust
    impl BridgeCommand {
        /// Carry out the command, returning the process exit code.
        pub async fn dispatch(self, version: &'static str) -> Result<i32> {
            match self {
                BridgeCommand::Serve => run(version).await,
                BridgeCommand::Status => status(version).await,
                BridgeCommand::Log { follow, lines } => log_command(follow, lines),
                BridgeCommand::Service(command) => {
                    let spec = bridge_service_spec(version);
                    command
                        .run(&spec, &Environment::detect(), &SystemManager)
                        .map_err(Error::from)
                }
                BridgeCommand::Workspace(command) => match command {
                    WorkspaceCommand::List => workspace_list(version).await,
                },
                BridgeCommand::Workgroup(command) => match command {
                    WorkgroupCommand::Create { name, device_name } => {
                        workgroup_create(&name, &device_name)
                    }
                    WorkgroupCommand::List => workgroup_list(),
                    WorkgroupCommand::Join {
                        ticket,
                        device_name,
                    } => workgroup_join(version, ticket, device_name).await,
                },
                BridgeCommand::Device(command) => match command {
                    DeviceCommand::List => device_list(version).await,
                    DeviceCommand::Invite {
                        name,
                        ttl,
                        workgroup,
                    } => device_invite(version, name, ttl, workgroup).await,
                    DeviceCommand::Retire { selector } => device_retire(&selector),
                },
            }
        }
    }
    ```
  - `run(version)` — the fn keeps its name and its body's semantics
    (`InstanceLock::acquire` first; `Err(Error::AlreadyRunning(pid))` → print
    `the bridge is already running (pid {pid})` and `Ok(1)`; log install; `build_bridge`;
    `bridge.run()`). Only the doc comment changes: the verb it implements is now `serve`,
    the bare invocation is the same thing, and "already running" is a normal outcome, not
    an error.
  - Wording: the five `println!("no bridge is running")` absence lines (in `status`,
    `device_list`, `device_invite`, `workgroup_join`, `workspace_list`) and the private
    `connect()` helper's doc all become **`no sapphire-bridge is running`** — matching
    `BridgeClient::connect_running`'s message, so every surface names the binary the same
    way.

**Design notes:**
- `status` keeps its two-source design untouched: live over the control plane
  (`client.status()` + `client.peers()`), or the last `status.json` snapshot when nothing
  answers (printed identically, flagged `— last seen before the bridge stopped`, exit 0);
  exit 1 only when there has never been a bridge here. It does **not** adopt Phase 1's
  `StatusReport` — that shape is the app server's liveness report; the bridge has strictly
  more to say (node id, workgroup, routes, peers), and grafting one onto the other would
  lose the snapshot fallback, which is the bridge's own worth. The "shared foundation" the
  re-homing adopts is the *vocabulary* and the exit-code discipline, not the report type.
- `apps/sapphire-bridge/src/main.rs`: the default becomes
  `let command = cli.command.unwrap_or(BridgeCommand::Serve);` (was
  `BridgeCommand::Run`); the module doc's "`sapphire-bridge run`" becomes
  "`sapphire-bridge serve`" and "`device forget`" becomes "`device retire`". The
  `const _: fn()` assertion pinning `ServiceCommand` + `bridge_service_spec` stays.
- The app-server side needs **no change** in this task: its `WorkgroupCommand::Create` /
  `DeviceCommand::Retire` arms already print directives in the bridge's new spelling
  (`run: sapphire-bridge workgroup create --device-name …` / `… device retire …`, Phase 1
  `sapphire-framework-server/src/command.rs:177,284`), and with the rename those directives
  name commands that exist.
- READMEs: `sapphire-bridge` (bare) described as "same as `sapphire-bridge serve`"; the
  `device forget <selector>` row becomes `device retire <selector>`; the prose "except
  `workgroup create` and `device forget`, which work directly on the bridge directory"
  says `retire`; the command table's `run` (default) row becomes `serve` (default).
  A one-line upgrade note goes under "Running it": *an OS service unit installed by an
  older build still names `run`; run `sapphire-bridge service install` again after
  upgrading, so the unit names `serve`.* (Same note in README.ja.md.)

- [ ] **Step 1: Write the failing tests**

In `crates/sapphire-framework-bridge/src/command.rs`, update `mod tests`'s `Probe`
(unchanged shape: `#[derive(Parser)] struct Probe { #[command(subcommand)] command:
BridgeCommand }`) and replace/add:

```rust
    #[test]
    fn the_subcommands_parse() {
        for args in [
            vec!["b", "serve"],
            vec!["b", "status"],
            vec!["b", "device", "list"],
            vec!["b", "device", "retire", "phone"],
            vec![
                "b",
                "workgroup",
                "create",
                "home",
                "--device-name",
                "laptop",
            ],
            vec!["b", "workgroup", "list"],
            vec!["b", "workspace", "list"],
            vec!["b", "service", "install"],
            vec!["b", "log", "--lines", "50"],
        ] {
            assert!(Probe::try_parse_from(&args).is_ok(), "{args:?}");
        }
    }

    #[test]
    fn bare_invocation_defaults_to_serve() {
        // `sapphire-bridge` with no subcommand is `sapphire-bridge serve` (2026-09-24
        // spec decision 1, bridge edition) — main.rs expresses it with
        // `unwrap_or(BridgeCommand::Serve)`, and the default pins that.
        #[derive(Parser)]
        struct Bare {
            #[command(subcommand)]
            command: Option<BridgeCommand>,
        }
        let bare = Bare::try_parse_from(["b"]).unwrap();
        assert!(bare.command.is_none());
        assert!(matches!(BridgeCommand::default(), BridgeCommand::Serve));
    }

    #[test]
    fn the_renamed_commands_have_no_aliases() {
        // Decision 9: cut over, no aliases. `run` and `forget` are gone as spellings.
        assert!(Probe::try_parse_from(["b", "run"]).is_err());
        assert!(Probe::try_parse_from(["b", "device", "forget", "phone"]).is_err());
    }

    #[test]
    fn there_is_no_way_to_place_a_workspace_from_here() {
        // Placing a workspace on this host is the owning application's business (spec §1).
        for args in [
            vec!["b", "workspace", "map", "notes", "/tmp/x"],
            vec!["b", "workspace", "init"],
            vec!["b", "workspace", "init", "--sync"],
        ] {
            assert!(Probe::try_parse_from(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn there_is_no_stop_command() {
        // Decision 4: starting and stopping the daemon is the service manager's business.
        assert!(Probe::try_parse_from(["b", "stop"]).is_err());
    }
```

And in `service_spec_tests`, the pinned args become the new verb:

```rust
    #[test]
    fn the_bridge_service_spec_runs_the_bridge() {
        let spec = bridge_service_spec("0.0.0");
        assert_eq!(spec.args, vec!["serve".to_owned()]);
        assert!(
            matches!(spec.system_run_as, RunAs::InvokingUser),
            "a root bridge would put the bridge directory under /root"
        );
    }
```

Keep as-is (they already match the new vocabulary or test untouched behavior):
`the_log_subcommand_parses`, `there_is_no_separate_pair_command`,
`status_against_no_bridge_exits_non_zero`, both `status_fallback_tests`,
`the_pairing_subcommands_parse`, `an_invite_needs_a_name`, `a_join_needs_a_ticket`,
`joining_without_a_running_bridge_says_so_rather_than_starting_one`,
`a_ttl_is_read_as_seconds`, `the_bridge_service_subcommands_parse`,
`the_bridge_service_spec_names_the_bridge`. Where those tests spell `device forget`
(none do — the parse list used `forget` only in `the_subcommands_parse`, replaced above),
nothing else to migrate. Delete the now-redundant duplicates
`mapping_a_workspace_is_still_not_a_bridge_command` (pairing_cli_tests) and the
`there_is_no_way_to_map_a_workspace_from_here` in `mod tests` — superseded by
`there_is_no_way_to_place_a_workspace_from_here` above, so no two tests pin one fact.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p sapphire-framework-bridge --all-features command
```

Expected: FAIL — `BridgeCommand::Serve` and `DeviceCommand::Retire` do not exist; `Run`
and `Forget` do, so the `the_renamed_commands_have_no_aliases` assertions also fail
(the old spellings still parse).

- [ ] **Step 3: Implement**

1. Rewrite the enum, `dispatch`, and the sub-enum renames exactly as specified under
   Produces; rename `device_forget` → `device_retire` (body and its `Ok(1)` semantics
   unchanged); update `run`'s and `bridge_service_spec`'s doc comments; align the five
   absence messages to `no sapphire-bridge is running`.
2. `apps/sapphire-bridge/src/main.rs`: `unwrap_or(BridgeCommand::Serve)` + doc-comment
   wording (`serve`, `retire`).
3. `apps/sapphire-bridge/README.md` and `README.ja.md`: the table row, the prose, the
   bare-invocation line, and the upgrade note, as listed under Design notes.
4. Module doc at the top of `command.rs`: "Two of them work directly on the bridge
   directory instead — `workgroup create`, … and `device retire`, because the control
   plane has no method for it" (was `device forget`), and the vocabulary sentence from
   the enum's doc comment.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p sapphire-framework-bridge --all-features
cargo test -p sapphire-framework-server --all-features
```

Expected: PASS — the bridge's parse, status-fallback, pairing and service-spec suites
green under the new names, and the `-server` suite (whose directive strings now name real
bridge verbs) untouched-green.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
git add crates/sapphire-framework-bridge apps/sapphire-bridge
git commit -m "feat(bridge): re-home the bridge CLI onto the shared flat command vocabulary"
```

---

### Task 4: Architecture prose, and the repository-wide sweep

**Files:**
- Modify: `docs/ARCHITECTURE.md`
- Test: the three CI gates over the whole workspace; a grep proving no stale spelling
  survived outside the historical records.

**Interfaces:**
- Consumes: everything above.
- Produces: prose that describes what now exists, and proof nothing else still speaks the
  old names. The facade (`crates/sapphire-framework/src/lib.rs`) needs no change: it
  re-exports the whole bridge crate as the `bridge` module (`pub use
  sapphire_framework_bridge as bridge;`), so `BridgeCommand` rides along under the same
  name, and its prelude keeps the app side's `FrameworkCommand` set from Phase 1.

- [ ] **Step 1: Find the stale prose**

```bash
grep -rn "sapphire-bridge run\|device forget\|BridgeCommand::Run\|\"run\"" \
  crates apps docs --include='*.rs' --include='*.md' | grep -v docs/superpowers
```

Expected: hits in `docs/ARCHITECTURE.md` (the bridge CLI paragraph) and nothing in `crates`
or `apps` (Task 3 cleared those). Hits under `docs/superpowers/` are the dated records and
stay.

- [ ] **Step 2: Update `docs/ARCHITECTURE.md`**

1. The crate table's `apps/sapphire-bridge` row (~line 98): the CLI list becomes
   `` `serve` / `status` / `log` / `service` / `workspace` / `workgroup` / `device` ``.
2. The bridge section's CLI paragraph (~lines 194-196, "CLI は `sapphire-bridge`（`status`,
   `log [--follow]`, `device …`, `workgroup …`, `workspace list`）。Phase 2（後続 issue）で
   この CLI も `FrameworkCommand` のフラット語彙へ載せ替える — 現時点では bridge の CLI は
   そのまま動く。"): rewrite to say the bridge's CLI now speaks the same flat vocabulary —
   `serve` / `status` / `service` / `workspace` / `workgroup` / `device` — composed in the
   bridge's own `BridgeCommand` (the shared vocabulary, bridge-side execution) beside the
   bridge-specific `log`; `workgroup create` and `device retire` run on the bridge
   directory directly, and the app CLIs' directives for them (Phase 1) now name commands
   that exist; `workspace` is read-only `list` because placing a workspace is the owning
   application's business (spec §1).
3. The `wake_on_sync` paragraph's `ManagedBy::Spawned` sentences (~lines 188-190) stay:
   the route records' legacy state and the wake rule are the follow-up issue's business,
   untouched by this plan (see Open questions).

- [ ] **Step 3: Run the tests to verify they pass**

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features --locked
```

Expected: all three green — the whole workspace, since Tasks 1-3 are each already green
and this task adds prose only. Re-run the Step 1 grep afterwards: no hits outside
`docs/superpowers/`.

- [ ] **Step 4: Commit**

```bash
git add docs/ARCHITECTURE.md
git commit -m "docs(arch): the bridge CLI speaks the flat command vocabulary"
```

## Risks

- **The renames are user-visible and alias-free** (Task 3): `sapphire-bridge run` and
  `device forget` stop existing. Spec decision 9 rules no aliases (the vocabulary is days
  old, the clients are first-party), so the cost is one upgrade step: **an OS service unit
  installed by an older build still names `run`, and a new binary will refuse it with
  clap's "unrecognized subcommand" — the bridge stops until `sapphire-bridge service
  install` is re-run** (the unit embeds args at install time). Documented in both
  READMEs; the `service` subcommand itself is untouched, so the reinstall path exists.
- **Two same-shaped enums can drift** (Task 3): the bridge's `WorkgroupCommand` /
  `DeviceCommand` and the app side's are separate types by design (the protocol crate must
  stay clap-free; the dependency graph allows no other home). The parse tests assert the
  same verb sets on both sides, and `ARCHITECTURE.md` lists the vocabulary once; a future
  verb must land in both parse tests or CI stays green while the vocabulary splits.
- **`status`'s snapshot fallback wording changes** (Task 3): the absence line becomes
  `no sapphire-bridge is running` (it was `no bridge is running`). Exit codes and the
  snapshot behavior are untouched; no test pinned the old wording.
- **The rename could hide a `match`** (Task 1): disproven up front — the Step 1 grep
  shows `Error::Spawn` has no match sites, only a definition and three constructions, so
  the compiler, not a silent arm, is the only consumer.
- **Deleting the counter could hide a reader added since** (Task 2): the Step 1 grep is
  the current tree's proof (writes only, no `load`); if an out-of-tree consumer existed it
  would be reading a private field — impossible.

## Open questions

- **`workspace list`'s local layer (spec decision 6's Phase 2 open point) — ruling: keep
  the CLI's direct read of the marker's `config.toml`.** Phase 1 shipped exactly that, and
  it is a feature, not a debt: the local rows are the app's own truth, readable while no
  app server is running, which moving them behind the app server's IPC would break for
  zero gain (the read is one small TOML parse the server does the same way). If a future
  GUI wants the same rows it reads the file too, as it does the registry the server
  maintains; if that ever becomes the wrong shape, moving the read behind a
  `workspace.list`-shaped IPC read is a one-crate change on top of Phase 1's
  `workspace.init` handler, which already owns the write side.
- **`ManagedBy::Spawned` stays.** It is still load-bearing data: legacy `routes.toml`
  rows and the `wake_on_sync` rule key on it (`-bridge/src/data.rs:428`), and the IPC
  handshake type carries it. This plan renames neither — the spawn machinery's *routing*
  redesign is the follow-up issue `ARCHITECTURE.md` already names, not this CLI plan.
- **The bridge's `serve` has no SIGTERM/SIGINT arms.** `serve_loops` is a three-way
  select over its listeners; a signal ends the process the hard way, skipping the status
  writer's final snapshot and the log guard's flush. Phase 1's `Sigterm` helper
  (`sapphire-framework-server/src/lib.rs:88-130`) is the template and the change is small,
  but it is process behavior, not command vocabulary — left to its own follow-up issue so
  this plan's diff stays on the command system.
- **`sapphire_ipc::Error::Timeout(&'static str)` and `SHUTDOWN_METHOD`** keep their names:
  the timeout is still real (handshake windows), and the bridge's control plane has no
  shutdown method at all, so nothing here is historical like `Spawn` was.
