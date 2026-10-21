# Settings Registry (parts 1–2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Read and write the bridge's settings by key — `sapphire-bridge config get/set/unset/list` and the `config.*` IPC methods — with typed validation, one `config.toml` per scope, secrets kept out of synced files, and an in-process restart for settings that need one.

**Architecture:** A types-only API crate (`sapphire-framework-settings-api`) carries the schema and values over IPC; a store crate (`sapphire-framework-settings`) resolves values across scopes and writes batches with `toml_edit`, asking each owner for its files and a final validation. The bridge is the first owner: its `net.toml` / `embedding.toml` merge into `config.toml` (format 1 → 2), its embedding writers become store batches, and `run` becomes a loop that rebuilds the bridge on `bridge.restart`.

**Tech Stack:** Rust 2024, serde, `toml` 1.1 (reading), `toml_edit` (writing), clap derive, the existing JSON-RPC IPC (`sapphire-framework-ipc`), tokio.

**Spec:** `docs/superpowers/specs/2026-10-21-settings-registry-design.md`

## Global Constraints

- Scopes are exactly `device`, `workgroup`, `workspace`; CLI flags `--device` / `--workgroup` (the bridge has no workspace scope).
- A key is its TOML path in its scope's `config.toml` (`embed.remote.endpoint` = `[embed.remote] endpoint`).
- Reserved top-level names: `net`, `embed`, `sync`, `retrieve`, `workspace`.
- Secrets are device-scope only, stored in their own owner-only (0600) file, never returned by `get`/`list`, never accepted as a command-line value.
- A batch is checked and validated in full before any file is written; files are written with temp-file-and-rename.
- `toml_edit` keeps comments and order of edited files.
- Bridge directory format goes from `1` to `2`; the device migration keeps old files as `*.toml.v1`.
- API crates are versioned independently (major = API version) and must not depend on 0.x framework crates: `sapphire-framework-settings-api` starts at `1.0.0` and is listed in `release-plz.toml` **without** `version_group`; `sapphire-framework-settings` is lockstep (`version_group = "framework"`, version `0.21.0`).
- Workspace dev-dependencies are path-only (no `version`); every non-dev path dependency has a `version`.
- Builds in a scratchpad worktree use `CARGO_TARGET_DIR` on disk (e.g. `~/.cache/claude-cargo-targets/sapphire-framework-314`), never `/tmp`.
- One root `CHANGELOG.md`; entries under `## [Unreleased]`. The file migration is listed as **Breaking**.

## Review Focus

1. **A user's hand-written comments in `net.toml` survive the migration and later `config set`s** — the migration copies the `[net]` content with `toml_edit` (comments included), not through serde. Test in Task 6.
2. **A device whose bridge is stopped while another device migrated the workgroup files** — the v2 bridge must read `root/config.toml` even if `root/embedding.toml` is still present from a late sync; the old file is migrated again only if `root/config.toml` has no `[embed]`. Test in Task 6.
3. **`config set` on a key whose current file is unparsable** — the batch is refused with the file's parse error; the file is not overwritten. Test in Task 3.
4. **`config set --stdin` with an empty pipe or only a newline** — refused ("no value was given"), nothing written, existing secret kept. Test in Task 8.
5. **`bridge.restart` while an app server is connected** — the app server's connection drops and it reconnects; the reply reaches the caller before the socket closes. Test in Task 9.

---

## File Structure

| Path | Responsibility |
|---|---|
| `crates/sapphire-framework-settings-api/` (new) | Wire types: `Scope`, `Kind`, `Apply`, `SettingDef`, `Value`, `EntryValue`, `SecretState`, `Origin`, `Entry`; `Kind::parse` / `Kind::check`. No I/O. |
| `crates/sapphire-framework-settings/src/registry.rs` (new) | `Registry`: owners' defs, duplicate and reserved-name checks, key / prefix lookup. |
| `crates/sapphire-framework-settings/src/owner.rs` (new) | `SettingsOwner` trait, `Staged`, `FieldError`. |
| `crates/sapphire-framework-settings/src/read.rs` (new) | Reading scope files, resolution, `list` / `get`, `StartState` and `pending_restart`. |
| `crates/sapphire-framework-settings/src/store.rs` (new) | `Op`, `Batch`, applying a batch with `toml_edit`, validation, atomic and private writes. |
| `crates/sapphire-framework-settings/src/error.rs` (new) | `Error`. |
| `crates/sapphire-framework-bridge/src/settings.rs` (new) | The bridge as owner: key table, `file`/`secret_file`, `validate`, `BridgeSettings` handle. |
| `crates/sapphire-framework-bridge/src/config_file.rs` (new) | `DeviceConfig` / `WorkgroupConfig` structs read from `config.toml`; format 1 → 2 migration. |
| `crates/sapphire-framework-bridge/src/{dir,net,workgroup,relay,embed_settings,lib,control,command}.rs` | Switch to `config.toml`, store-backed writers, IPC, restart loop. |
| `crates/sapphire-framework-bridge-api/src/{lib.rs,config.rs,cli.rs}` | `config.*` / `bridge.restart` method names and params; `ConfigCommand` clap types. |
| `crates/sapphire-framework-bridge-client/src/client.rs` | Typed `config_*` and `restart` calls. |

---

### Task 1: `sapphire-framework-settings-api` — schema and value types

**Files:**
- Create: `crates/sapphire-framework-settings-api/Cargo.toml`, `src/lib.rs`
- Modify: `Cargo.toml` (members; `toml_edit = "0.23"` in `[workspace.dependencies]` for Task 2), `release-plz.toml` (new `[[package]]` without `version_group`, with a comment as for `bridge-api`)

**Interfaces:**
- Produces (all `Clone, Debug, PartialEq, Serialize, Deserialize`; enums `#[serde(rename_all = "snake_case")]`):
  - `enum Scope { Device, Workgroup, Workspace }` with `Display` (`device`, …) and `FromStr`.
  - `enum Kind { Bool, Int { min: i64, max: i64 }, Enum { choices: Vec<String> }, String { non_empty: bool }, Url, Path, SocketAddr, List { item: Box<Kind> }, Secret }`
  - `enum Apply { Live, Restart }`
  - `struct SettingDef { key: String, kind: Kind, scopes: Vec<Scope>, overridable: bool, apply: Apply, default: Option<Value>, section: String, doc: String }` with `fn home(&self) -> Scope` (= `scopes[0]`) and `fn allows(&self, scope: Scope) -> bool` (home, or `Device` when `overridable`, or any listed scope).
  - `enum Value { Bool(bool), Int(i64), String(String), List(Vec<Value>) }` (`#[serde(untagged)]`), `Display` (lists joined with `,`).
  - `enum SecretState { Set, Unset }`; `enum EntryValue { Value(Value), Secret(SecretState), Unset }`; `enum Origin { Scope(Scope), Default }`
  - `struct Entry { key: String, value: EntryValue, origin: Origin, pending_restart: bool, note: Option<String> }`
  - `impl Kind { fn parse(&self, text: &str) -> Result<Value, String>; fn check(&self, value: &Value) -> Result<(), String> }`

- [ ] **Step 1: Write the failing tests** in `src/lib.rs` `mod tests`:

```rust
#[test] fn bools_accept_on_off_true_false() {
    for (t, v) in [("true", true), ("on", true), ("false", false), ("off", false)] {
        assert_eq!(Kind::Bool.parse(t).unwrap(), Value::Bool(v));
    }
    assert!(Kind::Bool.parse("yes").unwrap_err().contains("true, false, on, off"));
}
#[test] fn ints_name_their_range() {
    let k = Kind::Int { min: 64, max: 2048 };
    assert_eq!(k.parse("1024").unwrap(), Value::Int(1024));
    assert!(k.parse("32").unwrap_err().contains("64..=2048"));
}
#[test] fn enums_list_their_choices() {
    let k = Kind::Enum { choices: vec!["q8_0".into(), "f16".into()] };
    assert!(k.parse("q4").unwrap_err().contains("q8_0, f16"));
}
#[test] fn urls_are_http_only() {
    assert!(Kind::Url.parse("https://a.example").is_ok());
    assert!(Kind::Url.parse("ftp://a.example").is_err());
}
#[test] fn lists_take_commas_or_a_toml_array() {
    let k = Kind::List { item: Box::new(Kind::Url) };
    let want = Value::List(vec![Value::String("https://a".into()), Value::String("https://b".into())]);
    assert_eq!(k.parse("https://a,https://b").unwrap(), want);
    assert_eq!(k.parse(r#"["https://a", "https://b"]"#).unwrap(), want);
    assert_eq!(k.parse("").unwrap(), Value::List(vec![]));
}
#[test] fn secrets_are_never_parsed_from_text() {
    assert!(Kind::Secret.parse("sk-123").unwrap_err().contains("--stdin"));
}
#[test] fn device_is_allowed_only_when_overridable() {
    let mut d = def("embed.remote.model", Scope::Workgroup);
    assert!(!d.allows(Scope::Device));
    d.overridable = true;
    assert!(d.allows(Scope::Device));
}
#[test] fn values_round_trip_through_json() { /* Value::List of Int → serde_json → back, equal */ }
```

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-settings-api` — expected: compile errors (crate empty).

- [ ] **Step 3: Implement.** `Cargo.toml`: version `1.0.0`, deps `serde` only (the TOML-array form of `List` is parsed by hand: strip `[`/`]`, split on `,`, trim, strip one pair of `"` — no `toml` dependency). `Kind::check` validates an already-typed `Value` (used for IPC input); `parse` = text → `Value` → `check`. `Path` and `SocketAddr` check with `std::path::Path` / `str::parse::<SocketAddr>`. Add a guard test as in `bridge-api` (`the_crate_stays_free_of_framework_crates`: manifest contains no `sapphire-framework-` dependency).

- [ ] **Step 4: Run** `cargo test -p sapphire-framework-settings-api` — expected: all pass.

- [ ] **Step 5: Commit** `feat(settings-api): schema and value types for settings by key (#314)`

---

### Task 2: `sapphire-framework-settings` — registry, owners, reading and resolution

**Files:**
- Create: `crates/sapphire-framework-settings/Cargo.toml` (version `0.21.0`; deps `sapphire-framework-settings-api` `1.0.0`, `serde`, `toml`, `toml_edit`, `thiserror`; dev `tempfile`), `src/{lib,error,owner,registry,read}.rs`
- Modify: `Cargo.toml` (members), `release-plz.toml` (`version_group = "framework"`)

**Interfaces:**
- Consumes: Task 1 types.
- Produces:
  - `trait SettingsOwner: Send + Sync { fn defs(&self) -> Vec<SettingDef>; fn file(&self, scope: Scope) -> Result<PathBuf>; fn secret_file(&self, key: &str) -> Result<PathBuf>; fn validate(&self, staged: &Staged) -> std::result::Result<(), Vec<FieldError>>; fn scope_note(&self, scope: Scope) -> Option<String> { None } }`
  - `struct Staged { files: BTreeMap<PathBuf, String> }` with `fn text(&self, path: &Path) -> Option<&str>` (the edited text of a touched file; `None` = untouched, read it from disk) and `fn read(&self, path: &Path) -> Result<Option<String>>` (staged text, else disk, else `None`).
  - `struct FieldError { key: Option<String>, file: Option<PathBuf>, message: String }`
  - `struct Registry` — `fn new(owners: Vec<Arc<dyn SettingsOwner>>, reserved_for: &[(&str, usize)]) -> Result<Registry>`; `fn def(&self, key) -> Result<(&SettingDef, &dyn SettingsOwner)>`; `fn under(&self, prefix) -> Vec<&SettingDef>` (keys equal to or starting with `prefix.`); `fn schema(&self) -> Vec<SettingDef>`. `reserved_for` maps a top-level name to the owner index allowed to use it (`net`/`embed` → bridge); an app owner (part 3) gets its name.
  - `fn list(reg: &Registry, scope: Option<Scope>, start: Option<&StartState>) -> Result<Vec<Entry>>`; `fn get(reg, key) -> Result<Entry>`
  - `struct StartState(BTreeMap<String, Option<Value>>)` — `fn capture(reg: &Registry) -> Result<StartState>` (effective values of `Apply::Restart` keys).
  - `enum Error { UnknownKey(String), ScopeNotAllowed { key, scope, allowed: Vec<Scope> }, InvalidValue { key, message }, SecretOnCommandLine(String), Duplicate(String), Reserved { key, name }, Unparsable { file: PathBuf, message }, Rejected(Vec<FieldError>), Io(std::io::Error) }` with user-facing `Display`.

- [ ] **Step 1: Write failing tests** (`src/read.rs`, `src/registry.rs`), with a test owner over a temp dir whose `file(Device)` = `dev.toml`, `file(Workgroup)` = `wg.toml`:
  - `duplicate_keys_across_owners_are_rejected` → `Error::Duplicate("net.relays")`.
  - `a_reserved_name_is_rejected_for_another_owner` → owner 1 defines `embed.x`, `reserved_for = [("embed", 0)]` → `Error::Reserved`.
  - `the_home_scope_wins_over_the_default`: `wg.toml` = `[a]\nb = 3`, def default `Int(1)` → `get("a.b")` = `Value(Int(3))`, `origin = Scope(Workgroup)`.
  - `a_device_value_overrides_only_when_overridable`: both files set `a.b`; `overridable=false` → workgroup value; `true` → device value, `origin = Scope(Device)`.
  - `unset_without_default_is_unset`: → `EntryValue::Unset`, `origin = Default`.
  - `secrets_report_only_whether_they_are_set`: secret file present → `EntryValue::Secret(Set)`; absent → `Secret(Unset)`; the file's content appears nowhere in `format!("{:?}", entries)`.
  - `list_with_a_scope_shows_only_what_it_holds`.
  - `an_unparsable_file_is_an_error_naming_it` → `Error::Unparsable { file: …/wg.toml, .. }`.
  - `pending_restart_compares_with_the_start`: `StartState::capture`, then change `wg.toml` → that key `pending_restart == true`, a `Live` key never is.

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-settings` — expected: compile errors.

- [ ] **Step 3: Implement.** Reading uses `toml::Table` and walks the dotted key; TOML integers/bools/strings/arrays convert to `Value` (anything else is `Unparsable` with the key's path). The effective value: device (if `def.overridable`) → home scope → `def.default`. `Entry::note` = `owner.scope_note(origin scope)` (the bridge uses it for "this host has no workgroup").

- [ ] **Step 4: Run** `cargo test -p sapphire-framework-settings` — expected: pass.

- [ ] **Step 5: Commit** `feat(settings): registry, scopes and resolution (#314)`

---

### Task 3: `sapphire-framework-settings` — writing batches

**Files:**
- Create: `crates/sapphire-framework-settings/src/store.rs`
- Modify: `src/lib.rs` (exports)

**Interfaces:**
- Consumes: Task 2 `Registry`, `SettingsOwner`, `Staged`, `Error`.
- Produces:
  - `enum Op { Set { key: String, value: Value }, Unset { key: String } }` (an `Unset` key may be a prefix)
  - `fn apply(reg: &Registry, scope: Option<Scope>, ops: Vec<Op>) -> Result<Applied>` — `scope: None` means each key's home scope; `Some(s)` must be allowed for every key.
  - `struct Applied { pub keys: Vec<String>, pub restart: bool, pub live: bool }` (which keys changed; whether any is `Restart` / `Live`).
  - `fn write_atomic(path: &Path, text: &str) -> io::Result<()>` and `fn write_private(path: &Path, text: &str) -> io::Result<()>` (0600 on unix) — moved here from the bridge's `routes::write_atomic` / `embed_settings::write_private` semantics; the bridge keeps its own `write_atomic` for `routes.toml`.

- [ ] **Step 1: Write failing tests** (temp-dir owner as in Task 2, its `validate` rejecting `a.max < a.min`):
  - `set_keeps_comments_and_order`: file `# mine\n[net]\n# relays\nrelays = []\ndiscovery = true\n` → set `net.discovery=false` → file equals the input with only `true`→`false`.
  - `set_creates_missing_tables_and_the_file`.
  - `unset_removes_the_key_and_empty_tables`: `[a.b]\nc = 1` → unset `a.b.c` → file is empty (or only comments).
  - `a_prefix_unset_removes_only_the_target_scopes_keys`: device file `[embed.remote]\nenabled = false\nendpoint = "https://x"` where `embed.remote.enabled` is a device key and `endpoint` a workgroup key stored (by `file`) in the same device file → unset `embed.remote` at `Workgroup` → `enabled = false` remains.
  - `a_rejected_batch_writes_nothing`: set `a.min=10, a.max=5` → `Error::Rejected`, both files byte-identical to before.
  - `a_disallowed_scope_names_the_allowed_ones`: → `Error::ScopeNotAllowed { allowed: [Workgroup], .. }`, message contains `workgroup`.
  - `a_value_of_the_wrong_kind_is_refused` (`Int` key, `Value::String`) → `InvalidValue`.
  - `an_unparsable_file_is_not_overwritten` (Review Focus 3): file `[[[` → any set into it → `Error::Unparsable`, file unchanged.
  - `secrets_are_written_private_and_deleted_on_unset`: mode `0o600` (unix); unset → file gone; unsetting an absent secret is not an error; an empty secret value is `InvalidValue`.
  - `applied_reports_restart_and_live`.

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-settings store` — expected: fail.

- [ ] **Step 3: Implement `apply`.** Order: check every op (key exists or is a prefix with ≥ 1 key; scope allowed; `kind.check`) → group by `owner.file(scope)` (secrets by `owner.secret_file`) → load each file once as `toml_edit::DocumentMut` (absent = empty; parse error = `Unparsable`) → set with implicit tables created as needed; unset then prune empty tables up the path → `Staged` of every touched file's `to_string()` → `owner.validate(&staged)` per touched owner → write all files (`write_atomic`) then secrets (`write_private` / remove). Value → `toml_edit::Value`: `Int` as integer, `List` as an inline array.

- [ ] **Step 4: Run** `cargo test -p sapphire-framework-settings` — expected: pass.

- [ ] **Step 5: Commit** `feat(settings): write settings in validated, comment-preserving batches (#314)`

---

### Task 4: The bridge as a settings owner

**Files:**
- Create: `crates/sapphire-framework-bridge/src/settings.rs`, `crates/sapphire-framework-bridge/src/config_file.rs`
- Modify: `crates/sapphire-framework-bridge/Cargo.toml` (dep `sapphire-framework-settings` `0.21.0`, `sapphire-framework-settings-api` `1.0.0`), `src/lib.rs` (modules), `src/dir.rs` (`config_toml()` = `<root>/config.toml`), `src/workgroup.rs` (`config_toml()` = `root/config.toml`)

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `config_file.rs`: `#[serde(deny_unknown_fields)] struct DeviceConfig { #[serde(default)] net: NetConfig, #[serde(default)] embed: EmbedTable }`, `struct WorkgroupConfig { #[serde(default)] net: NetConfig, #[serde(default)] embed: EmbedTable }`, `struct EmbedTable { cache_dir: Option<PathBuf>, local: Option<LocalTable>, remote: Option<RemoteTable> }`, `struct LocalTable { enabled: Option<bool>, model: Option<String>, dimension: Option<u32>, max_tokens: Option<usize> }`, `struct RemoteTable { enabled: Option<bool>, endpoint: Option<String>, model: Option<String>, dimension: Option<u32> }`. `impl EmbedTable { fn models(&self) -> Result<ModelSettings, String> }` — local slot present iff `local.model` is set (missing fields take `LocalModel` defaults); remote slot present iff `remote.endpoint` is set (missing `model`/`dimension` → `Err` naming the field); result passed through `ModelSettings::validate`. `fn read_device(path) -> Result<DeviceConfig>`, `fn read_workgroup(path) -> Result<Option<WorkgroupConfig>>` (absent = `None`).
  - `settings.rs`: `pub fn bridge_defs() -> Vec<SettingDef>` (the spec's key table, `section` = `"Network"` / `"Embedding"`); `pub struct BridgeOwner { dir: BridgeDir }` implementing `SettingsOwner` — `file(Device)` = `dir.config_toml()`; `file(Workgroup)` = the workgroup's `config_toml()`, or `dir.config_toml()` when `Workgroup::open` is `None`; `file(Workspace)` = error; `secret_file("embed.remote.key")` = `dir.embedding_key()`; `scope_note(Workgroup)` = `Some("this host has no workgroup; stored on this device")` when there is none; `validate` parses each staged `config.toml` as `DeviceConfig`/`WorkgroupConfig`, runs `embed.models()` and `relay::relays(&net, None)`'s URL check, mapping errors to `FieldError`. `pub fn registry(dir: &BridgeDir) -> settings::Result<Registry>` with `reserved_for = [("net", 0), ("embed", 0)]`.

- [ ] **Step 1: Write failing tests** in `settings.rs`:
  - `the_defaults_match_the_structs`: for `net.wake_on_sync`, `net.discovery`, `net.use_default_relays`, `net.relays` the def's `default` equals the field of `NetConfig::default()`; for `embed.local.model`/`dimension`/`max_tokens` it equals `LocalModel::default()`'s.
  - `every_key_is_a_path_of_the_structs`: setting each non-secret key to a valid value in an empty temp `config.toml` (device keys) / workgroup file passes `validate` — catches a def whose path the struct rejects.
  - `a_partial_remote_slot_is_rejected`: set only `embed.remote.endpoint` → `Error::Rejected` naming `model`.
  - `workgroup_keys_go_to_the_device_file_without_a_workgroup`: no workgroup → set `embed.local.model` at home scope → `dir.config_toml()` contains `[embed.local]`; `get` reports `origin = Workgroup` and the note.
  - `with_a_workgroup_they_go_to_its_root`.
  - `a_bad_relay_url_is_rejected`.

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-bridge settings` — expected: fail.

- [ ] **Step 3: Implement** as in Interfaces. The key table, exact values:

| key | kind | scopes | apply | default |
|---|---|---|---|---|
| `net.wake_on_sync` | Bool | Device | Restart | `true` |
| `net.discovery` | Bool | Device | Restart | `true` |
| `net.relays` | List(Url) | Device | Restart | `[]` |
| `net.use_default_relays` | Bool | Device | Restart | `true` |
| `embed.cache_dir` | Path | Device | Live | none |
| `embed.local.enabled` | Bool | Device | Live | none (auto) |
| `embed.remote.enabled` | Bool | Device | Live | none (auto) |
| `embed.local.model` | Enum[`LOCAL_MODEL`] | Workgroup | Live | none |
| `embed.local.dimension` | Int 64..=2048 | Workgroup | Live | `DEFAULT_DIMENSION` |
| `embed.local.max_tokens` | Int 1..=`MAX_TOKENS` | Workgroup | Live | `DEFAULT_MAX_TOKENS` |
| `embed.remote.endpoint` | Url | Workgroup | Live | none |
| `embed.remote.model` | String non-empty | Workgroup | Live | none |
| `embed.remote.dimension` | Int 1..=65536 | Workgroup | Live | none |
| `embed.remote.key` | Secret | Device | Live | none |

`overridable` is `false` for all. `embed.local.model` has no default so that an unset model reads as "slot not configured".

- [ ] **Step 4: Run** `cargo test -p sapphire-framework-bridge settings` — expected: pass.

- [ ] **Step 5: Commit** `feat(bridge): describe the bridge's settings as a registry owner (#314)`

---

### Task 5: Read the bridge's settings from `config.toml`; embedding writers become batches

**Files:**
- Modify: `crates/sapphire-framework-bridge/src/net.rs` (`NetConfig::load` reads `[net]` of a `config.toml` via `config_file::read_device`), `src/workgroup.rs` (`published_net` reads `[net]` of `root/config.toml`; `publish_net` writes it with the store's `write_atomic` + `toml_edit`; rename `embedding_toml()` / `net_toml()` to `pub(crate) legacy_embedding_toml()` / `legacy_net_toml()`, used only by Task 6's migration), `src/dir.rs` (remove `net_toml()`/`embedding_toml()` except as migration sources in Task 6 — keep them `pub(crate)` and rename `legacy_net_toml()` / `legacy_embedding_toml()`), `src/lib.rs:203` and `src/command.rs` `build_bridge` (load from `dir.config_toml()`), `src/embed_settings.rs`, `src/control.rs` (`embed.*` handlers), `src/command.rs` (`embedding_offline`)

**Interfaces:**
- Consumes: Task 4 `registry(dir)`, `DeviceConfig`, `WorkgroupConfig`, `EmbedTable::models`; Task 3 `apply`, `Op`.
- Produces:
  - `embed_settings::load(dir, avx2)` — same signature and `Resolved` as today; reads `DeviceConfig.embed` (switches, `cache_dir`, and its models when there is no workgroup) and `WorkgroupConfig.embed` (models). `resolve` keeps its logic; its `device: &DeviceFile` parameter becomes `&EmbedTable`; `shadowed` = workgroup present and the device table configures a slot.
  - `pub fn embed_ops(request: EmbedRequest) -> Option<Vec<Op>>` in `embed_settings.rs` (`None` for `Show`) — the translation table of the spec ("`embedding` on top of the registry"). `ModelSet` with a model sets every field of the slot (local: `model`, `dimension`, `max_tokens`; remote: `endpoint`, `model`, `dimension`); `ModelSet` with `None` → `Unset { key: "embed.<slot>" }`; `DeviceSet` → `Set`/`Unset` `embed.<slot>.enabled`; `KeySet` → `Set embed.remote.key`; `KeyClear` → `Unset embed.remote.key`. A slot/model mismatch stays an error.
  - `set_model`, `set_device`, `set_key`, `clear_key`, `DeviceFile`, `read_device*`, `write_*` are removed.

- [ ] **Step 1: Rewrite the failing tests.** Port the existing `embed_settings` tests (`without_a_workgroup_models_go_in_the_device_file`, `with_a_workgroup_models_go_in_its_root`, `a_model_must_match_its_slot_and_validate`, `the_key_file_is_private_and_clearable`, `an_unparsable_device_file_reads_as_defaults_but_is_not_overwritten`) to drive writes through `settings::apply(&registry(&dir)?, None, embed_ops(req).unwrap())` and assert on `dir.config_toml()` / the workgroup's `config_toml()`. Add:
  - `a_device_net_section_survives_an_embedding_change`: `config.toml` = `[net]\n# keep\ndiscovery = false\n` → `embed_ops(DeviceSet local off)` applied → `[net]` and `# keep` still there.
  - `net_config_reads_the_net_table`: `[net]\nwake_on_sync = false` → `NetConfig::load(&dir.config_toml())` has `wake_on_sync == false`, other fields default.
  - `a_table_with_only_enabled_configures_no_slot`.
  Update `control.rs:1593` (test writes models into the workgroup file) to write `[embed.local]` / `[embed.remote]` into `wg.config_toml()`.

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-bridge` — expected: the ported tests fail.

- [ ] **Step 3: Implement.** The `embed.*` handlers in `control.rs` become: parse params → build the `EmbedRequest` → `embed_ops` → `settings::apply(&registry(&bridge.dir)?, None, ops)` → `embed_settings_report(&bridge)` (unchanged). `embedding_offline` does the same without a bridge. The relay test at `relay.rs:339` keeps using `publish_net`.

- [ ] **Step 4: Run** `cargo test -p sapphire-framework-bridge --all-features` and `cargo test -p sapphire-bridge` — expected: pass, including the unchanged `embedding` CLI tests in `command.rs`.

- [ ] **Step 5: Commit** `feat(bridge)!: read and write the bridge's settings in one config.toml per scope (#314)`

---

### Task 6: Migrate format 1 directories and workgroup roots

**Files:**
- Modify: `crates/sapphire-framework-bridge/src/dir.rs` (`BRIDGE_FORMAT_VERSION = 2`; call `config_file::migrate_device(&self)` in `prepare` where `found < BRIDGE_FORMAT_VERSION`, before writing the stamp), `src/config_file.rs` (migrations), `src/workgroup.rs` (`Workgroup::open` calls `config_file::migrate_workgroup(&wg)` before returning it)
- Modify: `docs/superpowers/specs/2026-10-21-settings-registry-design.md` — "runs once, under the instance lock" → "runs in `BridgeDir::open`, before the format stamp is written; it is idempotent, so a CLI and a starting bridge may both run it"

**Interfaces:**
- Produces: `pub(crate) fn migrate_device(dir: &BridgeDir) -> Result<()>`, `pub(crate) fn migrate_workgroup(wg: &Workgroup) -> Result<()>`.

- [ ] **Step 1: Write failing tests** in `config_file.rs`:
  - `a_v1_directory_is_folded_into_config_toml`: write `format`=`1`, `net.toml` = `# my relays\nrelays = ["https://r.example"]\nwake_on_sync = false\n`, `embedding.toml` = `cache_dir = "/c"\n[local]\nenabled = false\n[models.remote]\nendpoint = "https://e"\nmodel = "m"\ndimension = 8\n` → `BridgeDir::at` → `config.toml` parses to the same `NetConfig`, `embed.cache_dir == "/c"`, `embed.local.enabled == Some(false)`, `embed.remote.endpoint == Some("https://e")`; `config.toml` contains `# my relays` (Review Focus 1); `net.toml.v1` and `embedding.toml.v1` exist, the originals do not; `format` reads `2`.
  - `migration_is_idempotent`: run `migrate_device` twice → same `config.toml`.
  - `an_existing_config_toml_is_merged_not_replaced`: a `config.toml` already holding `[embed.local] enabled = true` keeps it; tables from the old files are added only where `config.toml` has none.
  - `a_workgroup_root_is_migrated_and_the_old_files_deleted`: `root/embedding.toml` = `[local]\ndimension = 512\n`, `root/net.toml` = `relays = ["https://r"]` → `Workgroup::open` → `root/config.toml` has `[embed.local] dimension = 512` **and** `model = LOCAL_MODEL` (an old `[local]` table meant "configured", so the migration writes the model explicitly), `[net] relays`; both old files gone.
  - `a_late_synced_old_file_does_not_override_config_toml` (Review Focus 2): `root/config.toml` has `[embed.remote]`, and `root/embedding.toml` reappears with different values → `Workgroup::open` deletes `root/embedding.toml` without touching `[embed]`.
  - `a_newer_format_still_refuses` (existing behavior, now with `3`).

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-bridge config_file` — expected: fail.

- [ ] **Step 3: Implement.** Use `toml_edit` throughout: parse the old file as a `DocumentMut`, and move its root items into a `[net]` / `[embed]` table of the target document, so comments travel with them (`[models.x]` items move into `[embed.x]`, and `[local]`/`[remote]` into `[embed.local]`/`[embed.remote]`). An old `[local]` table without `model` (in `[models]` or the workgroup file) gets `model = LOCAL_MODEL`. Write the target with `settings::write_atomic`, then rename (device) or delete (workgroup) the old files.

- [ ] **Step 4: Run** `cargo test -p sapphire-framework-bridge --all-features` — expected: pass (`opening_creates_the_layout_and_stamps_the_format` now expects `2`).

- [ ] **Step 5: Commit** `feat(bridge)!: migrate net.toml and embedding.toml into config.toml (format 2) (#314)`

---

### Task 7: `config.*` IPC methods and the typed client

**Files:**
- Create: `crates/sapphire-framework-bridge-api/src/config.rs`
- Modify: `crates/sapphire-framework-bridge-api/Cargo.toml` (dep `sapphire-framework-settings-api` `1.0.0`), `src/lib.rs` (method names; re-export), `crates/sapphire-framework-bridge/src/lib.rs` (`Bridge` gains `start: Mutex<Option<StartState>>`, captured at the top of `serve_loops`), `src/control.rs` (handlers), `crates/sapphire-framework-bridge-client/src/client.rs`

**Interfaces:**
- Consumes: Task 2 `list`/`get`/`StartState`, Task 3 `apply`, Task 4 `registry`, `Bridge::reload_embed`.
- Produces:
  - Method names: `CONFIG_SCHEMA = "config.schema"`, `CONFIG_LIST = "config.list"`, `CONFIG_GET = "config.get"`, `CONFIG_SET = "config.set"`, `CONFIG_UNSET = "config.unset"`, `RESTART = "bridge.restart"`.
  - Params/results: `ConfigListParams { scope: Option<Scope> }`, `ConfigGetParams { key: String }`, `ConfigSetParams { scope: Option<Scope>, entries: Vec<ConfigEntry> }` with `ConfigEntry { key: String, value: Value }`, `ConfigUnsetParams { scope: Option<Scope>, keys: Vec<String> }`, `ConfigSchemaResult { settings: Vec<SettingDef> }`, `ConfigListResult { entries: Vec<Entry> }`, `ConfigWriteResult { changed: Vec<String>, pending_restart: Vec<String> }`. `ConfigSetParams`'s `Debug` must not print secret values: implement `Debug` by hand, printing `<secret>` for every value (the API crate cannot tell which keys are secret).
  - `BridgeClient::{config_schema, config_list(scope), config_get(key), config_set(scope, entries), config_unset(scope, keys), restart}`.
  - Handlers: write → if `applied.live` call `bridge.reload_embed()` → `pending_restart` = keys of `list(.., start)` with the flag.

- [ ] **Step 1: Write failing tests:** method-name test in `bridge-api` (`config.*` prefix; `RESTART` starts with `bridge.`); `config_set_params_never_print_values`; in `crates/sapphire-framework-bridge/tests/settings_ipc.rs` (using `common::start` + `common::connect`):
  - `config_set_then_list_reports_the_value_and_its_origin`.
  - `a_live_embedding_change_applies_without_restart`: `config_set(None, local slot fields)` → `embed_settings()` reports the slot configured.
  - `a_restart_key_is_reported_pending`: set `net.discovery=false` → result `pending_restart == ["net.discovery"]`, and `config_list` marks it.
  - `a_rejected_batch_is_an_rpc_error_with_the_message`.
  - `secrets_never_come_back`: set `embed.remote.key` → `config_get` → `EntryValue::Secret(Set)`.

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-bridge --test settings_ipc` — expected: fail.

- [ ] **Step 3: Implement** handlers alongside the `EMBED_*` ones in `control.rs`, using the existing `parse` / `failed` / `encode` helpers; `bridge.restart` is added in Task 9.

- [ ] **Step 4: Run** `cargo test -p sapphire-framework-bridge-api -p sapphire-framework-bridge -p sapphire-framework-bridge-client` — expected: pass.

- [ ] **Step 5: Commit** `feat(bridge): config.* methods to read and write settings by key (#314)`

---

### Task 8: `sapphire-bridge config` CLI

**Files:**
- Modify: `crates/sapphire-framework-bridge-api/src/cli.rs` (`ConfigCommand`, value parsing, printing), `crates/sapphire-framework-bridge/src/command.rs` (`BridgeCommand::Config(ConfigCommand)`, `config()` and `config_offline()`)

**Interfaces:**
- Consumes: Task 7 client and params; Task 2/3 for the offline path; `read_key`'s prompt logic.
- Produces (in `bridge-api` `cli`):

```rust
#[derive(Debug, clap::Subcommand)]
pub enum ConfigCommand {
    /// List settings and their values.
    List { #[command(flatten)] scope: ScopeFlags, #[arg(long)] show_origin: bool },
    /// Show one setting.
    Get { key: String },
    /// Set settings: `key=value`..., or one secret key alone (prompted, or `--stdin`).
    Set { #[arg(required = true)] entries: Vec<String>, #[command(flatten)] scope: ScopeFlags,
          #[arg(long)] stdin: bool, #[arg(long)] restart: bool },
    /// Remove settings (or every setting under a prefix), going back to the defaults.
    Unset { #[arg(required = true)] keys: Vec<String>, #[command(flatten)] scope: ScopeFlags,
            #[arg(long)] restart: bool },
}
#[derive(Debug, clap::Args)]
pub struct ScopeFlags { #[arg(long, conflicts_with = "workgroup")] device: bool, #[arg(long)] workgroup: bool }
impl ScopeFlags { pub fn scope(&self) -> Option<Scope> }
pub fn parse_entries(schema: &[SettingDef], entries: &[String], stdin: bool) -> Result<Vec<ConfigEntry>, String>
pub fn describe_entries(entries: &[Entry], show_origin: bool) -> Vec<String>
```

`ScopeFlags` has no `--workspace`: part 3 adds an app-side variant.

- [ ] **Step 1: Write failing tests** (`cli.rs` unit tests, and `command.rs` offline tests on a temp `SAPPHIRE_BRIDGE_DIR` as the existing embedding CLI tests do):
  - `entries_parse_by_kind`: `["net.discovery=off", "net.relays=https://a,https://b"]` → `Bool(false)`, `List[..]`.
  - `a_secret_value_on_the_command_line_is_refused`: `["embed.remote.key=sk"]` → error containing `--stdin`.
  - `a_secret_key_must_be_set_alone`: `["embed.remote.key", "net.discovery=on"]` → error.
  - `an_empty_stdin_secret_is_refused` (Review Focus 4): reader yields `"\n"` → error `no value was given`. (Factor the read as `fn read_secret(stdin: impl BufRead, tty: bool) -> io::Result<String>` so it is testable.)
  - `an_unknown_key_suggests_none_and_names_it`.
  - `list_prints_secrets_as_set_or_unset_and_origins_on_request`: `describe_entries` → `embed.remote.key = <set>`, with `show_origin` a ` (workgroup)` suffix, a final `restart required: net.discovery` line when any entry is pending.
  - `offline_set_writes_the_files_and_says_so`: no bridge → `config set net.discovery=false` returns `0`, prints `the bridge is not running; it applies these settings when it starts`, and `config.toml` has `[net] discovery = false`.
  - `offline_restart_flag_is_an_error`.

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-bridge-api --features cli cli` and `cargo test -p sapphire-framework-bridge command` — expected: fail.

- [ ] **Step 3: Implement.** Online: `config_schema` once, `parse_entries`, `config_set`/`config_unset`; if the result's `pending_restart` is non-empty and `--restart` was not given, print ``restart required: run `sapphire-bridge restart` ``; with `--restart` call Task 9's restart. Offline: `registry(&BridgeDir::open()?)` with the same parsing and `settings::apply`. `get` of an unknown key exits `1` with the error.

- [ ] **Step 4: Run** the commands of Step 2 — expected: pass.

- [ ] **Step 5: Commit** `feat(bridge): sapphire-bridge config get/set/unset/list (#314)`

---

### Task 9: Restart in place

**Files:**
- Modify: `crates/sapphire-framework-bridge/src/lib.rs` (`Bridge` gains `restart: tokio::sync::Notify` and `pub fn request_restart(&self)`; `serve_loops`'s `select!` gains `_ = bridge.restart.notified() => Ok(Exit::Restart)`; `run`/`run_shared` return `Result<Exit>`), `src/command.rs` (`run` loop; `BridgeCommand::Restart`; `--restart` wiring), `src/control.rs` (`bridge.restart` handler), `crates/sapphire-framework-bridge/tests/common/mod.rs` (fixtures ignore `Exit`)

**Interfaces:**
- Consumes: Task 7 `RESTART`, `BridgeClient::restart`.
- Produces:
  - `pub enum Exit { Stopped, Restart }` (re-exported from the crate root).
  - `pub async fn serve_restarting<F, Fut>(mut build: F) -> Result<()> where F: FnMut() -> Fut, Fut: Future<Output = Result<Bridge>>` — builds, runs, and on `Exit::Restart` logs `restarting (requested over IPC)` to `BRIDGE_TARGET` and builds again; `command::run` calls it with `build_bridge(dir.clone(), version)` plus the `EmbedFactory` (`Arc` clone), holding `InstanceLock` and the log guard outside.
  - `bridge.restart` handler: returns `{}`; spawns a task that yields once (`tokio::task::yield_now`, then a 50 ms sleep) and calls `request_restart`, so the reply is flushed first.
  - CLI: `sapphire-bridge restart` → `client.restart()`, then wait (poll `sapphire_ipc::probe` every 100 ms, up to 30 s) for the endpoint to go and come back; prints `the bridge restarted`; no bridge → prints `no sapphire-bridge is running`, exit `1`.

- [ ] **Step 1: Write failing tests** in `crates/sapphire-framework-bridge/tests/restart.rs`:
  - `restart_rebuilds_and_serves_again`: `serve_restarting` with a builder over a loopback transport and a fixed control endpoint that counts its calls; connect, `restart()` returns `Ok` (the reply arrived, Review Focus 5); the endpoint answers again; builder called twice; `status().pid == std::process::id()`.
  - `a_restart_picks_up_a_changed_restart_key`: set `net.wake_on_sync=false` over IPC → `pending_restart` contains it → restart → `config_list` shows no pending key (the new `StartState`).
  - `a_connected_client_sees_the_connection_close_and_can_reconnect` (Review Focus 5): a second client's `closed()` resolves after the restart, and a new `connect` succeeds.

- [ ] **Step 2: Run** `cargo test -p sapphire-framework-bridge --test restart` — expected: fail.

- [ ] **Step 3: Implement** as in Interfaces. Update `Bridge::run`'s doc comment and the fixtures that `await` `run_shared` (they now get `Result<Exit>`).

- [ ] **Step 4: Run** `cargo test -p sapphire-framework-bridge --all-features` and `cargo test --workspace` — expected: pass.

- [ ] **Step 5: Commit** `feat(bridge): restart the bridge in place, from the CLI and over IPC (#314)`

---

### Task 10: Documentation and changelog

**Files:**
- Modify: `CHANGELOG.md` (`## [Unreleased]`), `docs/ARCHITECTURE.md` (the bridge directory layout and the embedding settings section at lines ~236–275), `bridge/README*` or the crate `README.md` if they mention `net.toml` / `embedding.toml` (find with `grep -rn 'net.toml\|embedding.toml' --include=*.md .`)

- [ ] **Step 1: Write the entries.**
  - Added: `sapphire-bridge config list|get|set|unset` with `--device` / `--workgroup`, `--stdin` for secrets, `--restart`; `sapphire-bridge restart`; IPC `config.schema|list|get|set|unset` and `bridge.restart` (`sapphire-framework-bridge-api` 2.x minor); new crates `sapphire-framework-settings-api` (1.0) and `sapphire-framework-settings`.
  - Changed, **Breaking:** the bridge's settings move to `<bridge>/config.toml` (`[net]`, `[embed]`) and the workgroup's to `root/config.toml`; the bridge directory becomes format 2 and migrates on first start (`net.toml` / `embedding.toml` kept as `*.toml.v1`); the workgroup's synced files are migrated by the first upgraded device, so **every device of a workgroup must be upgraded together**. `[embedded_relay]` moves to `[net.embedded_relay]`.
- [ ] **Step 2: Update `ARCHITECTURE.md`'s directory tree and settings text** to name `config.toml`, the scopes and the `config` command.
- [ ] **Step 3: Run** `grep -rn 'net\.toml\|embedding\.toml' docs/ARCHITECTURE.md crates bridge --include=*.md --include=*.rs` — expected: only migration code, `*.toml.v1`, and historical specs.
- [ ] **Step 4: Run** `cargo publish --workspace --dry-run --all-features` — expected: success (catches missing `version` on path deps and dev-dep cycles).
- [ ] **Step 5: Commit** `docs: the bridge's config.toml and the config command (#314)`
