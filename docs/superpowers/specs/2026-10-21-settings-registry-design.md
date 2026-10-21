# Settings registry: typed settings by key, set from the CLI and the GUI

Issue: #314. Related: #186 (embedding settings, spec `2026-10-10-embedding-settings-design.md`),
#307 (local model quantization), #313 (Qwen3-VL-Embedding-8B).

## Why

Settings live in TOML files in four places, and only the embedding settings have a CLI
(`embedding`) and a GUI view. Everything else — `net.toml` above all — is edited by hand,
and a hand edit of `net.toml` needs a restart nobody can trigger from a UI. Adding one
dedicated command per setting does not scale: #307 alone adds a setting.

What we want:

1. One generic way to read and write a setting by key, in the CLI (`config get/set/unset/list`)
   and in the GUI (one settings screen), for the bridge's settings and the app's.
2. The target scope chosen explicitly, like `git config --global/--local`: in the CLI by a
   flag, in the GUI by tabs (like VS Code's User / Workspace tabs).
3. Values checked before they are written, by the same code that reads them.
4. Secrets (API keys) never typed on a command line, never printed, never synced.
5. A setting that needs a restart says so, and the restart can be done from the CLI and the
   GUI without stopping the process.

## Scope of this spec

The work is split into four parts, each with its own plan and PR:

1. **Registry core** — key definitions, scopes and override resolution, `toml_edit`
   storage, secret storage, batch validation. *(This spec, in full.)*
2. **Bridge integration** — the bridge's keys, the `config.*` IPC methods, the
   `sapphire-bridge config` and `restart` commands, the in-process restart, the offline
   path, and `embedding` rebuilt on top. Closes #314. *(This spec, in full.)*
3. **App integration** — an API for apps to register their own keys, `config.*` and restart
   on the app server, and `<app> config` routing to the bridge and the app server.
   *(Outline only; its own spec.)*
4. **GUI** — the settings screen with Device / Workgroup / Workspace tabs, Apply, the
   restart banner; the Embedding screen reduced to status. *(Outline only; its own spec.)*

Parts 1 and 2 ship together: the core is designed against its first real owner.

## Decisions

1. **Apps register their own keys (registry approach).** The framework does not know an
   app's schema — apps live in other repositories — so it provides a registry and each
   owner (the bridge, the framework, each app) contributes a table of key definitions.
   A schemaless key/value editor was rejected: it cannot validate.
2. **Three scopes.**
   - `device` — this host only; never synced.
   - `workgroup` — synced to every device of the workgroup (the bridge's workgroup root).
   - `workspace` — synced with one app's workspace (`{root}/.{app}/config.toml`).

   A single `shared` scope was rejected: a workgroup setting reaches every app on every
   device, a workspace setting reaches one app's workspace, and the user should see which
   one they are changing.
3. **Each key has one home scope; overriding is opt-in per key.** A key lists the scopes it
   accepts; the first is its home. Only a key marked `overridable` accepts `device` in
   addition to its home, and a device value then wins. Keys whose value must agree across
   devices (a model's dimension) stay single-scope. No key opts in in the first version; the
   mechanism exists for later keys.
4. **Keys are named by feature, not by file.** `embed.remote.endpoint`, `net.relays`. Where a
   key is stored is the owner's business, so a setting can move between files without
   renaming it. The framework and the bridge reserve the top-level names `net`, `embed`,
   `sync`, `retrieve` and `workspace`; an app's keys start with the app's name
   (`notes.default_template`). The registry rejects a duplicate key when it is built.
5. **Secrets are device-scope only, entered by prompt or stdin.** A key of kind `Secret` is
   stored in its own owner-only file (mode 0600), never in a synced scope. `get` and `list`
   show only whether it is set. `config set <secret-key>` with no value prompts without echo;
   `--stdin` reads it from a pipe; a value given on the command line is an error, so it never
   reaches shell history. An OS keyring and reading from an environment variable are left
   for later.
6. **Each key declares how a change applies: `Live` or `Restart`.** A `Live` key is applied
   when it is written (through the owner's reload hook). A `Restart` key is applied at the
   next start, and a restart is available as an RPC that rebuilds the process's components
   in place — same process, same pid, same lock — so it works the same in the foreground,
   as an OS service, and on Windows. No separate "reload" command: hand edits of `Live`
   keys are picked up by the existing settings watch.
7. **Writes are batches.** Settings that are only valid together (a remote model's
   endpoint, model and dimension) are set in one call — `config set a=1 b=2` in the CLI, Apply
   in the GUI — and the batch is written entirely or not at all.
8. **The existing loaders have the final word.** Each value is checked against its kind
   first; then the owner rebuilds its existing structs (`NetConfig`, the device
   `embedding.toml`, `ModelSettings`) from the edited documents with the same
   `toml::from_str` and `validate()` that read them at startup. A batch they reject is not
   written.
9. **Comments and order are kept.** Files are edited with `toml_edit`.
10. **`embedding` stays.** The `embedding` CLI keeps its interface and becomes a translation
    into config batches; the `embed.*` RPCs stay for existing clients. In the GUI, editing
    moves to the settings screen (part 4).

## Part 1: Registry core — crate `sapphire-framework-settings`

### Types

The schema and value types are plain serde types, shared by the CLI, the GUI and the IPC
API. Writing is behind a `store` feature (it brings `toml_edit`), so the GUI depends on the
types only.

```rust
pub enum Scope { Device, Workgroup, Workspace }

pub enum Kind {
    Bool,
    Int { min: i64, max: i64 },
    Enum(Vec<String>),
    String { non_empty: bool },
    Url,               // http(s) only
    Path,
    SocketAddr,
    List(Box<Kind>),
    Secret,
}

pub enum Apply { Live, Restart }

pub struct SettingDef {
    pub key: String,          // "embed.remote.endpoint"
    pub kind: Kind,
    pub scopes: Vec<Scope>,   // scopes[0] is the home scope
    pub overridable: bool,    // accepts Device as well as the home scope
    pub apply: Apply,
    pub default: Option<Value>, // None: unset has its own meaning (e.g. "auto")
    pub section: String,      // GUI grouping: "Network", "Embedding"
    pub doc: String,          // one line, shown by `list` and the GUI
}

pub enum Value { Bool(bool), Int(i64), String(String), List(Vec<Value>) }
// Secret values travel as `Value::String` over the local IPC only (as `embed.key_set` does
// today) and are never returned: a read answers `SecretState::{Set, Unset}`.

pub struct Entry {
    pub key: String,
    pub value: EntryValue,        // Value, Secret(SecretState), or Unset
    pub origin: Origin,           // Scope, or Default
    pub pending_restart: bool,
}
```

The definitions use owned strings so the schema can be served over IPC (`config.schema`)
and so an app can build its table at runtime; owners write them as a function returning a
`Vec<SettingDef>`.

### Owners

```rust
pub trait SettingsOwner {
    /// This owner's key definitions.
    fn defs(&self) -> Vec<SettingDef>;
    /// Where a key is stored in a scope, as of now.
    fn locate(&self, key: &str, scope: Scope) -> Result<Location>;
    /// The final check: the edited documents, as the owner's own loaders read them.
    fn validate(&self, staged: &Staged) -> Result<(), Vec<FieldError>>;
}

pub enum Location {
    /// A value inside a TOML file: the file, and the path of tables and the key in it.
    Toml { file: PathBuf, path: Vec<String> },
    /// A whole file holding one secret, owner-only.
    SecretFile(PathBuf),
}
```

`locate` is resolved when called, not fixed in the definition, so an owner can express
"a workgroup key on a host without a workgroup is stored in the device file's `[models]`"
(see part 2). `Staged` gives the owner each touched file's edited text, so `validate` runs
the same `toml::from_str` the owner uses at startup.

### Resolution

For each key, the effective value is the first of:

1. the `device` value, if the key is `overridable` and it is set;
2. the value in the key's home scope;
3. the default.

`list` returns one `Entry` per key with its origin. `list --scope S` returns only what is
set in `S`. `get` returns one entry.

### Writing

A `Store` applies a batch of `set` and `unset` operations for one scope:

1. **Check each operation.** The key exists; the scope is allowed for it; the value parses
   as its kind and is in range. A failure names the key and what is allowed (the
   choices of an `Enum`, the range of an `Int`, the scopes of the key).
2. **Edit in memory.** Each touched TOML file is loaded once as a `toml_edit` document; a
   set inserts or replaces the item at its path, creating tables as needed; an unset
   removes it and then any table left empty. An unset may name a **prefix** of keys
   (`embed.remote`), which removes the whole table — this is how a model slot is removed.
3. **Validate.** The owner's `validate` sees every touched file's edited text. Any error
   aborts the batch; nothing is written.
4. **Write.** Each file is written atomically (temporary file and rename, as
   `routes::write_atomic` does). Secret files are written owner-only (0600), as
   `embed_settings::write_private` does. Unsetting a secret deletes its file.

The batch is all-or-nothing up to step 4. Step 4 writes several files one after another;
a failure between two renames leaves the batch partly applied. That is accepted: batches
span at most two files in practice, every file is individually valid after step 3, and a
retry converges.

### Pending restart

When a process starts it records the effective values of its `Restart` keys. `list`
compares the current effective values with that record and sets `pending_restart` on each
key that differs, so a change made while the process was stopped — or by hand — is
reported as well.

### Errors

One error type with variants for an unknown key, a scope not allowed for the key, a value
that does not parse or is out of range, a secret given on the command line, the owner's
validation failures (each with the key or file at fault), and I/O. Messages are
user-facing; the CLI prints them as they are.

## Part 2: Bridge integration

### Keys

| key | kind | scope | apply |
|---|---|---|---|
| `net.wake_on_sync` | Bool | device | Restart |
| `net.discovery` | Bool | device | Restart |
| `net.relays` | List(Url) | device | Restart |
| `net.use_default_relays` | Bool | device | Restart |
| `embed.cache_dir` | Path | device | Live |
| `embed.local.enabled` | Bool, unset = auto | device | Live |
| `embed.remote.enabled` | Bool, unset = auto | device | Live |
| `embed.local.model` | Enum([`LOCAL_MODEL`]) | workgroup | Live |
| `embed.local.dimension` | Int 64..=2048 | workgroup | Live |
| `embed.local.max_tokens` | Int 1..=`MAX_TOKENS` | workgroup | Live |
| `embed.remote.endpoint` | Url | workgroup | Live |
| `embed.remote.model` | String, non-empty | workgroup | Live |
| `embed.remote.dimension` | Int ≥ 1 | workgroup | Live |
| `embed.remote.key` | Secret | device | Live |

Where they are stored:

- `net.*` → `<bridge>/net.toml`, at the same names.
- `embed.cache_dir`, `embed.{local,remote}.enabled` → `<bridge>/embedding.toml`
  (`cache_dir`, `[local] enabled`, `[remote] enabled`).
- `embed.{local,remote}.*` model fields → `<bridge>/workgroups/<id>/root/embedding.toml`
  (`[local]`, `[remote]`). **On a host without a workgroup** they are stored in the device
  file's `[models.local]` / `[models.remote]`, as today, and `list` reports their origin as
  `workgroup` with a note that the host has no workgroup. When the host joins one, the
  workgroup's file wins, as today.
- `embed.remote.key` → `<bridge>/embedding.key`.

The bridge's `validate` parses `net.toml` into `NetConfig` (which also runs the relay URL
check of `relay::relays`), the device `embedding.toml` into its `DeviceFile`, and the
workgroup's file into `ModelSettings` and calls `ModelSettings::validate`. A remote slot with
only some of its fields set fails there, which is why the slot's fields are set in one batch.

A test checks that each key's default in the registry is the default of the struct that
reads it (`NetConfig::default()`, `LocalModel::default()`), so the two cannot drift.

When #307 lands it adds `embed.local.qtype` to this table.

**Not in this version:**

- `net.embedded_relay.*`. Whether the table is present is its on/off switch, and `hostname`
  and the TLS fields are required together, which fits per-key editing poorly; it is an
  operator's setting and stays a hand edit. A follow-up issue covers it.
- The workgroup's announced `root/net.toml`. Nothing writes it in production yet
  (`publish_net` has no caller).

### IPC

New methods in `sapphire-framework-bridge-api`, handled in `control.rs`:

| method | params | result |
|---|---|---|
| `config.schema` | — | `Vec<SettingDef>` |
| `config.list` | `{ scope?: Scope }` | `Vec<Entry>` |
| `config.set` | `{ scope?: Scope, entries: [{ key, value }] }` | `{ pending_restart: bool }` |
| `config.unset` | `{ scope?: Scope, keys: [String] }` | `{ pending_restart: bool }` |
| `bridge.restart` | — | `{}` — sent before the restart begins |

A missing `scope` means each key's home scope. After a write that touches a `Live` embedding
key, the handler calls `reload_embed`, as the `embed.*` handlers do today. The 5-second
`watch_embed_settings` stays, so a hand edit is still picked up.

`BridgeClient` gets typed methods for each.

### CLI

```
sapphire-bridge config list   [--device | --workgroup] [--show-origin]
sapphire-bridge config get    <key>
sapphire-bridge config set    <key>=<value>... [--device | --workgroup] [--restart]
sapphire-bridge config set    <secret-key> [--stdin]
sapphire-bridge config unset  <key | prefix>... [--device | --workgroup] [--restart]
sapphire-bridge restart
```

- **Values.** A `Bool` accepts `true`, `false`, `on`, `off`. A `List` accepts a
  comma-separated list (`net.relays=https://a,https://b`) or a TOML array (`[...]`). Other
  kinds are parsed as their kind.
- **Secrets.** `config set embed.remote.key` prompts without echo; with `--stdin` it reads one
  value from stdin and trims the trailing newline. `config set embed.remote.key=...` is an
  error. The existing `read_key` in `bridge-api/src/cli.rs` is reused.
- **Output.** `list` prints `key = value` per line, `<set>` / `<unset>` for secrets, the
  origin with `--show-origin`, and a closing line naming the keys waiting for a restart.
  `set` and `unset` print `restart required: run \`sapphire-bridge restart\`` when they made
  a restart pending and `--restart` was not given.
- **The bridge running.** The command goes over IPC.
- **The bridge stopped.** The `Store` writes the files directly and the command prints
  "the bridge is not running; it applies these settings when it starts", as
  `embedding_offline` does. `--restart` and `restart` with no bridge running are errors
  that say so.

The `config` subcommand's clap types live in `sapphire-framework-bridge-api` next to
`EmbeddingCommand`, so part 3 can reuse them in the app CLI.

### Restart in place

`command::run` becomes a loop around the part that depends on the settings:

```rust
let _lock = InstanceLock::acquire(&dir)?;   // held across restarts
let _log = logging::install(&dir)?;
loop {
    let mut bridge = build_bridge(dir.clone(), version).await?;
    if let Some(factory) = &embed { bridge = bridge.embed_factory(Arc::clone(factory)); }
    match bridge.run().await? {
        Exit::Stopped => break,
        Exit::Restart => tracing::info!(target: BRIDGE_TARGET, "restarting (requested over IPC)"),
    }
}
```

- `Bridge` holds a `restart: Notify`. `serve_loops` waits on it in its `select!` alongside
  the listeners; when it fires, the loops return `Exit::Restart`, the `Bridge` is dropped
  (the iroh endpoint, the IPC listeners, the workgroup driver and the status writer stop
  with it), and the next iteration reads `net.toml` and the workgroup again.
- The `bridge.restart` handler sends its reply and then notifies, so the caller gets an
  answer before the socket closes.
- The lock and the log writer live outside the loop: the pid, the lock file and the log
  file stay the same.
- Connected app servers lose the control connection for the length of the restart and
  reconnect as they do after a bridge crash today. The CLI's `restart` and `--restart` wait
  until the control endpoint answers again (with a timeout) before returning.

### `embedding` on top of the registry

The `EmbeddingCommand` keeps its interface. Its offline path and the `embed.*` RPC handlers
are reimplemented as config batches:

| `embedding` command | batch |
|---|---|
| `local set …` | set `embed.local.*` |
| `local clear` | unset `embed.local` |
| `remote set --endpoint --model --dimension` | set `embed.remote.{endpoint,model,dimension}` |
| `remote clear` | unset `embed.remote` |
| `{local,remote} device on/off` | set `embed.<slot>.enabled` |
| `{local,remote} device auto` | unset `embed.<slot>.enabled` |
| `key set` / `key clear` | set / unset `embed.remote.key` |

`embedding show` and `embed.settings` stay as they are: they report status (the active
slot, AVX2, what the provider is doing) as well as settings. `embed_settings::set_model`,
`set_device`, `set_key` and `clear_key` are replaced by the store; `load` and `resolve`
stay.

### Tests

Core:

- resolution order (device override, home scope, default) and `origin`;
- a scope not allowed for a key is rejected and the error names the allowed scopes;
- each kind's parsing and range errors;
- `toml_edit` keeps comments and order in an edited file;
- unset removes the key and empty tables; a prefix unset removes the table;
- a batch that fails validation writes nothing;
- secrets: never returned by `get`/`list`, written 0600, deleted on unset;
- `pending_restart` against a recorded start state;
- duplicate keys across owners are rejected.

Bridge:

- defaults match `NetConfig::default()` and `LocalModel::default()`;
- `config set/list/unset` against a running test bridge, including a live embedding change
  taking effect without a restart;
- the offline path writes the files the running bridge then reads;
- a workgroup key on a host without a workgroup lands in the device file's `[models]`;
- `bridge.restart`: the reply arrives, the pid is unchanged, the control endpoint answers
  again, and a changed `net.*` value is in effect after it;
- the existing `embedding` tests pass unchanged.

## Part 3 (outline): App integration

- The app registers a `SettingsOwner` with the framework (through the same builder that
  registers the app's commands). Its keys are prefixed with the app's name; the framework
  contributes its own (`sync`, `retrieve.*`), stored in the device-scope
  `<config_dir>/workspace.toml` and the workspace-scope `{root}/.{app}/config.toml`.
- The app server serves `config.*` for its owners and `server.restart` (the same in-place
  restart as the bridge).
- `<app> config …` takes the same arguments as the bridge's, plus `--workspace`; it routes
  each key to its owner — bridge keys to the bridge, app and framework keys to the app
  server — and writes files directly when the owner is not running.
- Its own spec decides how app settings are reloaded live and what the app server's
  restart rebuilds.

## Part 4 (outline): GUI

- One settings screen in `SyncPanel`, with Device / Workgroup / Workspace tabs, built from
  `config.schema` with no knowledge of particular keys: a widget per kind, grouped by
  `section`, a password field for secrets that never shows the stored value.
- Edits are a draft per tab, written as one batch by **Apply**; errors are shown at the
  field. Keys set in another scope show where their value comes from.
- A banner for pending restarts ("Restart the bridge to apply these changes") with a
  **Restart** button, per process (bridge, app server).
- The Embedding screen keeps its status display and links to the settings screen for
  editing.

## Out of scope

- An OS keyring, and secrets from environment variables.
- Settings of `net.embedded_relay` and the workgroup's announced `net.toml`.
- A reload command separate from restart.
- Retiring the `embedding` command and the `embed.*` RPCs (after part 4, separately).
