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
4. **Keys are named by feature, and a key is its TOML path.** `embed.remote.endpoint` is
   `endpoint` in the `[embed.remote]` table, `net.relays` is `relays` in `[net]`. The
   framework and the bridge reserve the top-level names `net`, `embed`, `sync`, `retrieve`
   and `workspace`; an app's keys start with the app's name (`notes.default_template`). The
   registry rejects a duplicate key when it is built.
5. **One settings file per owner and scope.** Every scope of an owner has one
   `config.toml`, laid out the same way, so a scope only chooses the file and a device
   override is the same path in the device file — as `git config` does. The bridge's
   per-topic files are merged:

   | | device | workgroup |
   |---|---|---|
   | before | `<bridge>/net.toml`, `<bridge>/embedding.toml` | `root/embedding.toml`, `root/net.toml` |
   | after | `<bridge>/config.toml` | `root/config.toml` |

   The topic files were never separate for a reason beyond their topic: `net.toml` was the
   bridge's only settings file when it was designed, and `embedding.toml` was added beside
   it. With the CLI and the GUI as the main way to edit settings, one file per scope is
   simpler to locate, validate and document. The bridge's files are not merged into an
   app's `config.toml`: the bridge is one process per OS user serving every app, and knows
   no app. Secrets stay in their own owner-only files.
6. **Secrets are device-scope only, entered by prompt or stdin.** A key of kind `Secret` is
   stored in its own owner-only file (mode 0600), never in a synced scope. `get` and `list`
   show only whether it is set. `config set <secret-key>` with no value prompts without echo;
   `--stdin` reads it from a pipe; a value given on the command line is an error, so it never
   reaches shell history. An OS keyring and reading from an environment variable are left
   for later.
7. **Each key declares how a change applies: `Live` or `Restart`.** A `Live` key is applied
   when it is written (through the owner's reload hook). A `Restart` key is applied at the
   next start, and a restart is available as an RPC that rebuilds the process's components
   in place — same process, same pid, same lock — so it works the same in the foreground,
   as an OS service, and on Windows. No separate "reload" command: hand edits of `Live`
   keys are picked up by the existing settings watch.
8. **Writes are batches.** Settings that are only valid together (a remote model's
   endpoint, model and dimension) are set in one call — `config set a=1 b=2` in the CLI, Apply
   in the GUI — and the batch is written entirely or not at all.
9. **The existing loaders have the final word.** Each value is checked against its kind
   first; then the owner rebuilds its existing structs (`NetConfig`, `ModelSettings` and
   the device's embedding switches) from the edited documents with the same
   `toml::from_str` and `validate()` that read them at startup. A batch they reject is not
   written.
10. **Comments and order are kept.** Files are edited with `toml_edit`.
11. **`embedding` stays.** The `embedding` CLI keeps its interface and becomes a translation
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
    /// The `config.toml` that holds a scope's values, as of now.
    fn file(&self, scope: Scope) -> Result<PathBuf>;
    /// The owner-only file that holds a secret key.
    fn secret_file(&self, key: &str) -> Result<PathBuf>;
    /// The final check: the edited documents, as the owner's own loaders read them.
    fn validate(&self, staged: &Staged) -> Result<(), Vec<FieldError>>;
}
```

A key's place in the file is its own path (decision 4), so the owner only names a file per
scope. `file` is resolved when called, not fixed, so an owner can map a scope to another
scope's file: the bridge stores workgroup keys in the device file on a host without a
workgroup (see part 2). `Staged` gives the owner each touched file's edited text, so
`validate` runs the same `toml::from_str` the owner uses at startup.

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
   (`embed.remote`), which removes every key under it that the target scope holds — this is
   how a model slot is removed. Keys of other scopes that share the table (the device's
   `embed.remote.enabled` beside the models on a host without a workgroup) stay.
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

- `device` → `<bridge>/config.toml`.
- `workgroup` → `<bridge>/workgroups/<id>/root/config.toml`. **On a host without a
  workgroup**, `<bridge>/config.toml` — the same paths in the device file — and `list`
  reports their origin as `workgroup` with a note that the host has no workgroup. When the
  host joins one, the workgroup's file wins and the device file's model keys are ignored and
  reported as shadowed, as `embedding show` does today.
- `embed.remote.key` → `<bridge>/embedding.key`, unchanged.

For example, a device file:

```toml
[net]
discovery = true
relays = ["https://relay.example.org"]

[embed]
cache_dir = "/var/cache/sapphire"

[embed.local]
enabled = false
```

and the workgroup's:

```toml
[embed.remote]
endpoint = "https://api.example.org"
model = "text-embedding-3-small"
dimension = 1024
```

A model slot is configured when its `model` (local) or `endpoint` (remote) is set in the
scope that holds the models; a table holding only `enabled` configures nothing. Setting
`embed.local.model` alone configures the local slot with the defaults of its other fields,
as an empty `[local]` table does today.

The bridge reads each file into one struct — `[net]` into `NetConfig`, `[embed]` into the
model settings and the device's switches — and `validate` uses the same code: `NetConfig`
(and the relay URL check of `relay::relays`), and `ModelSettings::validate`. A remote slot
with only some of its fields set fails there, which is why the slot's fields are set in one
batch. The registry, not the struct, decides which keys each scope may hold; the struct
accepts the union and rejects unknown keys.

`net.embedded_relay` moves into the device file as the `[net.embedded_relay]` table, unchanged
apart from its place.

A test checks that each key's default in the registry is the default of the struct that
reads it (`NetConfig::default()`, `LocalModel::default()`), so the two cannot drift.

When #307 lands it adds `embed.local.qtype` to this table.

**Not in this version:**

- `net.embedded_relay.*`. Whether the table is present is its on/off switch, and `hostname`
  and the TLS fields are required together, which fits per-key editing poorly; it is an
  operator's setting and stays a hand edit. A follow-up issue covers it.
- Workgroup-scope `net.*` keys (the announced relays of the old `root/net.toml`). Nothing
  writes them in production yet (`publish_net` has no caller); the bridge reads `[net]` from
  `root/config.toml` where it read `root/net.toml`, so a later version can add the keys.

### Migrating the old files

The bridge directory's format goes from 1 to 2. `BridgeDir::open` already has the place for
the migration (`found < BRIDGE_FORMAT_VERSION`); it runs once, under the instance lock,
before anything reads the settings:

- `<bridge>/net.toml` becomes the `[net]` table of `<bridge>/config.toml`.
- `<bridge>/embedding.toml` becomes `[embed]`: `cache_dir` stays, `[local]` / `[remote]`
  become `[embed.local]` / `[embed.remote]`, and its `[models.local]` / `[models.remote]`
  move into the same tables.
- The old files are renamed to `*.toml.v1` rather than deleted, so a user can see what was
  moved.

The workgroup's synced files are migrated by whichever upgraded bridge opens the workgroup
first: `root/embedding.toml` becomes `[embed]` and `root/net.toml` becomes `[net]` of
`root/config.toml`, and the old files are deleted, which syncs. Two bridges migrating at
once write the same content, so last-writer-wins converges.

**All devices of a workgroup must be upgraded together.** A bridge of format 1 reads only
`root/embedding.toml`, so once another device has migrated it, the old bridge sees no
embedding models until it is upgraded. The project is before 1.0 and the embedding settings
are recent (#186), so this is accepted and stated as a breaking change in the CHANGELOG.

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
  with it), and the next iteration reads `config.toml` and the workgroup again.
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
- a workgroup key on a host without a workgroup lands in the device `config.toml`, and is
  shadowed once the host joins a workgroup;
- the format 1 → 2 migration, for the device files (including `[models]`) and the
  workgroup's synced files, is idempotent and keeps every value; a v1 directory opened by a
  v2 bridge serves the same embedding and network settings as before;
- `bridge.restart`: the reply arrives, the pid is unchanged, the control endpoint answers
  again, and a changed `net.*` value is in effect after it;
- the existing `embedding` tests pass unchanged.

## Part 3 (outline): App integration

- The app registers a `SettingsOwner` with the framework (through the same builder that
  registers the app's commands). Its keys are prefixed with the app's name; the framework
  contributes its own (`sync`, `retrieve.*`). Following decision 5, the device scope is
  `<config_dir>/config.toml` — today's `<config_dir>/workspace.toml`, renamed and migrated —
  and the workspace scope is `{root}/.{app}/config.toml`, both laid out by key path.
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
- Per-key settings of `net.embedded_relay`, and workgroup-scope `net.*` keys.
- A reload command separate from restart.
- Retiring the `embedding` command and the `embed.*` RPCs (after part 4, separately).
