# The app's own settings, merged `config.*` and `server.restart`

Part 3b of #314 (the settings registry, `2026-10-21-settings-registry-design.md`). It builds
on Part 3a (`2026-10-22-bridge-relay-design.md`): clients reach the bridge only through
their app server.

## Goal

An application registers its settings with the framework, and its users read and write them
— together with the framework's and the bridge's — through one command and one endpoint.
Settings that need a restart can be applied from the client by restarting the app server in
place.

Success means:

- `<app> config list|get|set|unset` covers the bridge's keys, the framework's keys
  (`http.*`, `update.check`) and the app's keys, with `--device`, `--workgroup` and
  `--workspace`;
- a batch that mixes bridge and app-server keys is written whole or not at all;
- `<app> config set … --restart` (and later the GUI) restarts whichever process the change
  needs, and the app server restarts without leaving its process;
- a setting that keeps the app server from starting can be fixed with `<app> config` while
  the server is down;
- the app reads its settings through a handle and hears about live changes.

## Decisions

1. **Device file `<config_dir>/config.toml`**, replacing `server.toml` (migrated), and
   **shared with the application's own config file** (sapphire-agent keeps `[anthropic]`,
   `[matrix]`, tokens and more there; sapphire-timer `[cache]`, `[workspace]`, a top-level
   `sync_interval_minutes`). The framework checks and writes only its own keys and the
   app's, and leaves the rest of the file as it is. Workspace file
   `{root}/.{app}/config.toml`. No workgroup scope for app-server keys.
2. **Mixed batches are checked on both sides first.** `config.set` / `config.unset` gain
   `dry_run`; the app server checks its part, has the bridge check its part, and only then
   writes (bridge first, then its own). The new fields make `sapphire-framework-bridge-api`
   3.0 (below).
3. **In-place restart, as the bridge does it.** The app hands the framework a function that
   builds its `AppServer`; `server.restart` rebuilds from it on the same listeners.
4. **Offline edits.** With no app server running, `<app> config` edits the framework's and
   the app's keys in their files directly; bridge keys are an error.
5. **A `Settings` handle with change notification.** The build function receives it;
   `Live` keys are read from it and announced through a watch channel.

## Keys and owners

The app server builds one `Registry` (from `sapphire-framework-settings`) of two owners:

- **The framework**: `http.listen` (SocketAddr), `http.allowed_hosts` (List of String),
  `http.mcp.enabled` (Bool, default false), `http.metrics.enabled` (Bool, default false),
  `update.check` (Bool, default true). All `Apply::Restart`, all `Scope::Device` only, section
  "Server".
- **The app**: the `AppSettings` the app passes to `serve_restarting` (below) — its defs and
  an optional whole-file `validate`; the framework, not the app, decides the files (it wraps
  `AppSettings` in a `SettingsOwner`). Every key starts with `<app_name>.`; its scopes may be
  `Device` and `Workspace` (not `Workgroup`); a secret is `Device` only, as everywhere.

Reserved top-level names in the app server's registry: `http`, `update`, `sync`, `retrieve`,
`workspace` (framework), `<app_name>` (the app), and `net`, `embed` (the bridge's, owned by
no one here), so the merged namespace never has a key two processes claim. `sync`,
`retrieve` and `workspace` are only reserved; no keys are defined for them yet.

The device file is checked, at start and before every write, only under `[http]` and
`[update]` — an unknown key there or a value of the wrong kind is an error — and for the old
`[mcp]` table. Every other top-level name, `sync`, `retrieve` and `workspace` included (an
app may keep its own `[workspace]` there), is the application's: its `<app_name>` table goes
to the app's `validate`, and the rest is not looked at. A write edits only the keys it names
(through `toml_edit`), so the rest of the file — comments included — is kept byte for byte,
and the file keeps its permissions (it may hold the app's tokens).

### Files

| Scope | File |
|---|---|
| Device | `<config_dir>/config.toml` (shared with the app's own config) |
| Workspace | `{root}/.{app}/config.toml` (synced with the workspace) |
| Secrets | `<config_dir>/secrets/<key>`, 0600, never synced |

Writing the workspace scope with no workspace selected fails with `no workspace is
selected`. The workspace file may still hold an old `[workspace]` table; it is a known table
to the registry, not an unknown key, and the existing clean-up on first mapping stays.

### Migration from `server.toml`

When the app server starts (and when the offline CLI opens the files), if
`<config_dir>/server.toml` exists:

- with no `config.toml`, its text is written to `config.toml` unchanged;
- with one (the app's own), `server.toml`'s tables are merged into it with `toml_edit`,
  keeping `config.toml`'s comments and content; a key `config.toml` already has wins, and
  each such key is named (never its value) in a warning. The write keeps the file's
  permissions.

Then `server.toml` is renamed `server.toml.v1`; a rename that fails after the write is a
warning (on Windows the file may be held open), and the next start merges again, changing
nothing. The layout (`[http]`, `[http.mcp]`, `[http.metrics]`, `[update]`) is already the
keys' layout. A file with the old `[mcp]` table still fails with today's message naming where
the setting moved.

`ServerConfig` is no longer deserialized from a file: `run` builds it from the registry's
effective values. `AppServer::config(cfg)` keeps overriding it for tests and embedders.

## `config.*` on the app server

The relay stops forwarding the five `config.*` methods (they leave
`sapphire_framework_bridge_api::RELAYED`; the partition test moves them to a third list,
answered by the app server). The app server registers its own handlers under the same names
and types, so clients — `BridgeClient::relayed` over the app connection — do not change.

### Reads

- `config.schema`: the app server's defs followed by the bridge's.
- `config.list {scope}`: both sides' entries, merged by key. `--workgroup` is the bridge's
  alone; `--workspace` is the app server's alone.
- `config.get {key}`: answered by the key's owner.
- With no bridge running, `schema` and `list` answer the app server's part alone, and
  `get` of a key the app server does not know answers `BRIDGE_NOT_RUNNING`. The CLI prints
  one line under such a list: `sapphire-bridge is not running; its settings are not shown`.

### Writes

`config.set` / `config.unset` split the batch by owner (the app server's registry decides;
everything else is the bridge's):

1. The app server checks its part (kinds, scopes, its owners' `validate`) without writing.
2. If the batch has a bridge part, the bridge checks it: the same call with `dry_run: true`.
3. If both pass: the bridge writes its part, then the app server writes its own.

Any error in steps 1–2 fails the whole batch and nothing is written. A bridge part with no
bridge running fails the batch with `BRIDGE_NOT_RUNNING`. An unknown key while the bridge is
down fails with `unknown setting {key} (or sapphire-bridge, which may know it, is not
running)`. A disk error in step 3 can still leave the bridge's part written; the error names
which part was not.

The result's `changed` lists both parts' keys; `pending_restart` lists every key of either
process whose effective value differs from what that process started with.

### `dry_run`

`ConfigSetParams` and `ConfigUnsetParams` gain `#[serde(default)] dry_run: bool`. The
settings crate splits `apply` into a check (everything up to and including the owners'
`validate`) and the write; with `dry_run` the bridge answers the check's outcome and writes
nothing (the result's `changed` lists what would change; `pending_restart` is the current
list).

The new fields (`dry_run` here, `SettingDef::process` below) are added to exhaustive structs,
so `sapphire-framework-bridge-api`'s next release is a major, 3.0, and the handshake's API
version (the crate's major) moves with it: a bridge older than this one fails the handshake,
so every bridge the app server talks to knows `dry_run`. The bridge and the apps are updated
together; `sapphire-framework-server-api` (which re-exports bridge-api) follows.

### Which process to restart

`SettingDef` gains `#[serde(default)] process: Process`, `Process { Bridge, App }` with
`Bridge` the default (an older bridge's defs read as `Bridge`). The app server sets `App` on
its own. Clients map each key of `pending_restart` to its process through the schema.

### Logging

Values are never logged. The app server's `config.*` handlers log only the method and the
outcome, as the relay does.

## Restart

### `server.restart`

A new method in `sapphire-framework-server-api` (`SERVER_RESTART = "server.restart"`, result
`Ack`). The server answers, waits `RESTART_DELAY` (as the bridge does), then stops the way
it stops on `shutdown` — HTTP, sync, the update check, open client connections — except that
it keeps its IPC listeners, its pid and instance lock, and its log. It then reads the
settings again, calls the build function and runs the new `AppServer` on the same
listeners. A server started with `AppServer::run()` instead answers `server.restart` with an
error: `this server cannot restart itself; restart its service`.

### The app's entry point

```rust
// today
AppServer::new(&CTX, VERSION).sync_via_bridge().extend(routes).run().await
// with settings and restart
serve_restarting(&CTX, MyOwner, |settings: &Settings| {
    AppServer::new(&CTX, VERSION).sync_via_bridge().extend(routes)
})
.await
```

`serve_restarting(ctx, owner, build)` (in `sapphire-framework-server`): migrate, read the
settings, call `build`, attach the owner and the `Settings` to the server, run it, and
repeat on a restart. `ctx` is the app's `AppContext`: the settings' files depend on it, and
they are read before `build` runs, so `build` cannot supply it. `owner` is
`impl AppSettings + Clone` (`()` for an app with no settings of its own).

**Breaking:** `FrameworkCommand::dispatch(self, server: AppServer, version)` becomes
`dispatch(self, ctx, owner, build, version)`. `serve` runs
`serve_restarting(ctx, owner, build)`; the other subcommands call `build` once if they need
an `AppServer`, and `config` uses `ctx` and `owner` for its offline path. `update`, `service`
and `mcp` read the settings too, but an unreadable settings file must not lock the user out
of them (`update` and `service` are how it gets fixed), so they warn and fall back to the
defaults.

### `Settings`

A cheap clonable handle (`Arc` inside):

- `get(&self, key) -> Option<Value>`: the effective value (secrets answer `None`; the app
  reads a secret through its owner's `secret_file`);
- `get_as::<T>(&self, key) -> Option<T>` for `bool`, `i64`, `String`, `Vec<String>`;
- `changed(&self) -> tokio::sync::watch::Receiver<u64>`: bumped after every write that
  changes a `Live` key's effective value.

The first `Settings` is read before `build` runs, so `build` can use `Restart` keys. A write
updates the handle's values for `Live` keys at once; `Restart` keys keep their start values
until the restart, and `pending_restart` compares against those (`StartState`, as the
bridge).

## `<app> config`

`FrameworkCommand::Config(AppConfigCommand)`: `list [--show-origin]`, `get <key>`,
`set <key=value>… [--stdin] [--restart]`, `unset <key>… [--restart]`, each with one of
`--device`, `--workgroup`, `--workspace` (a new `AppScopeFlags`; bridge-api's `ScopeFlags`
is unchanged). Parsing and rendering reuse bridge-api's `parse_entries`, `read_secret` and
`describe_entries`: a secret is a lone key whose value comes from the prompt or `--stdin`,
never from the command line.

After a write, `pending_restart` is reported per process: `restart required:
sapphire-bridge` and/or `restart required: {app} server`. With `--restart` the CLI restarts
what is needed — the bridge first (relayed `bridge.restart`, which waits for the new
bridge), then the app server (`server.restart`, then waits up to 30 s for the endpoint to
answer again).

### Offline

With no app server running, `<app> config` builds the same registry from the framework's
defs and `owner`, migrates `server.toml` if needed, and reads or writes the files itself,
with the same checks and atomic writes. A bridge key (anything the registry does not know)
fails with `the {app} server is not running; start it, or use sapphire-bridge config for
{key}`. `list` shows the app server's keys and the line about the bridge's.

## Tests

- Settings crate: the check-only split writes nothing and reports the same errors as a real
  write.
- Bridge: `config.set` / `config.unset` with `dry_run` validate and write nothing.
- App server (against `StubBridge` and a real bridge):
  - `config.schema` / `list` merge both sides; `--workgroup` / `--workspace` filter;
  - a mixed batch with an error on either side writes nothing on either side;
  - with no bridge: app-server keys read and write; a batch with a bridge key fails;
  - values never appear in the captured log (`http.allowed_hosts` and an app secret).
- Migration: `server.toml` becomes `config.toml` plus `server.toml.v1`; `[mcp]` still fails
  with its message; `ServerConfig` comes out as before.
- Restart: after `server.restart` the same endpoint answers, a changed `Restart` key has its
  new value, and the instance lock and pid are unchanged; `run()` answers the error.
- `Settings`: a `Live` write fires `changed()` and `get` returns the new value; a `Restart`
  write does neither.
- CLI: `set` / `get` / `list` / `unset` online and offline; a bridge key offline; a secret
  through `--stdin`; `--restart` restarts the right processes.

## Out of scope

- The GUI settings screen (Part 4).
- Keys for `sync` and `retrieve.*` (reserved only).
- A workgroup scope for app-server keys.
- `McpContext::bridge`.
