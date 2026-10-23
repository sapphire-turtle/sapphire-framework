# The GUI's settings pages

Part 4 of #314 (the settings registry, `2026-10-21-settings-registry-design.md`). It builds
on Part 3a (clients reach the bridge through the app server, `2026-10-22-bridge-relay-design.md`)
and Part 3b (the app server's settings, merged `config.*`, `server.restart`,
`2026-10-22-app-settings-design.md`).

## Goal

The GUI reads and writes every setting `<app> config` does — the bridge's, the framework's
and the app's — without knowing any key in particular, and restarts whichever process a
change waits for.

Success means:

- every key in `config.schema` can be seen and edited in the GUI, in every scope it allows;
- adding a key to any owner needs no GUI change;
- a value from another scope says where it comes from; a secret's value is never shown;
- an edit is written as one batch, and a rejected one says which field is wrong;
- a change that needs a restart says so, per process, and one button restarts it.

## Decisions

1. **A page per section.** Each `SettingDef.section` is a page of the Settings screen, with a
   tab per scope its keys allow.
2. **Sections named like an existing screen join it.** Today that is Embedding: its status
   display stays and its settings form replaces the hand-made one.
3. **Field errors from both sides.** The GUI checks each field as it is typed (`Kind::parse`,
   `Kind::check`); the bridge and the app server attach the keys of a rejected batch to the
   error (`RpcError.data`), and the GUI shows each under its field.

## Pages

Pages are built from `config.schema`, in schema order, one per distinct `section`:

- a section whose keys are all the app's (`process == App` and the key starts with
  `<app_name>.`) is listed under the app's heading, before the app's own `SettingsSection`
  pages;
- a section named like an existing screen (compared case-insensitively with the screen's
  label; today only "Embedding") is shown inside that screen, below its own content;
- every other section is listed under a "Configuration" heading, after "Sync".

Until the first schema arrives, the list shows only the existing pages. A page with an
unsaved draft is marked in the list.

### A page

- **Tabs**: the scopes any of the page's keys allow (`SettingDef::allows`), in the order
  Device, Workgroup, Workspace. With no workspace selected (`ServerState.current == None`)
  the Workspace tab is disabled with the hint `No workspace is selected`.
- **Rows**, one per key the tab's scope allows, in schema order:
  - the key, its widget, and `doc` as a weak one-line hint;
  - the value's origin when it is not this tab's scope: `from device`, `from workgroup`,
    `from workspace`, or `default`;
  - a small "restart" mark for `Apply::Restart` keys;
  - when this tab's scope sets the key, an "×" that unsets it here (a draft edit);
  - a "changed elsewhere" mark when the key has a draft and the server value changed under
    it.
- **Footer**: `Revert` and `Apply`, enabled only with a draft; a spinner while a load or
  write runs, during which the page's inputs are disabled.
- With no app server: the existing `unavailable` line. With no bridge: the bridge's keys
  (`process == Bridge`) are read-only, with the note `sapphire-bridge is not running`.

### Widgets by kind

| Kind | Widget |
|---|---|
| Bool | checkbox |
| Int { min, max } | integer drag value clamped to the range |
| Enum | combo box of its choices |
| String, Url, Path, SocketAddr | single-line text, parsed with `Kind::parse` as typed |
| List | multi-line text, one item per line, each parsed as the item kind |
| Secret | password field, empty, hint `set` / `not set`; part of the draft only once typed into |

A text that does not parse shows its error under the field, and Apply stays disabled while
any field of the tab is invalid.

## Data

### Commands

The settings are not part of the 2-second poll. The client gains:

- `Command::ConfigLoad` → `CommandOutput::Config(ConfigState)`: `config.schema` and one
  `config.list` per scope (`None`, `Device`, `Workgroup`, and `Workspace` when a workspace is
  selected), in one command;
- `Command::ConfigWrite { scope, set: Vec<ConfigEntry>, unset: Vec<String> }` →
  `CommandOutput::Written(ConfigWriteResult)`: first `config_check_set` / `config_check_unset`
  for the non-empty parts, then `config_set`, then `config_unset`; any failure stops it;
- `Command::Restart(Process)` → `CommandOutput::Done`: `Bridge` calls the relayed
  `bridge.restart` (which returns once the new bridge is connected); `App` calls
  `server.restart`, waits for the connection to close, then reconnects and polls
  `server.info` until it answers or 30 s pass (`the {app} server did not come back within
  30 s`).

A command's failure keeps its field errors: `Outcome.result`'s error side becomes a
`CommandError { message: String, fields: Vec<ConfigFieldError> }` (today a `String`), where
`fields` is read from `RpcError.data` when it holds `ConfigFieldErrors`.

### When to load

When a settings page (or the Embedding screen) is shown and nothing is loaded; after an
Apply or a restart; on the page's "↻" button; and when the snapshot's selected workspace
changes. The loaded `ConfigState` lives in `Settings`; each page holds only its drafts.

### Drafts

A draft per (page, scope): `key → Edit`, where `Edit` is `Set { text, parsed }` or `Unset`.
A reload replaces the shown values of keys without a draft and marks drafted keys whose
server value changed; a draft is never discarded by a reload. Switching pages or tabs keeps
drafts. Revert drops the tab's draft.

### Apply

Apply sends the tab's draft as one `ConfigWrite` with that tab's scope. On success the draft
is cleared (secret fields emptied) and the page reloads. On failure the draft stays; each
`ConfigFieldError` whose key is on the page shows under its field, the rest and the message show
as an error line above the footer.

## Field errors on the wire

`sapphire-framework-bridge-api` gains `ConfigFieldErrors { fields: Vec<ConfigFieldError> }` and
`ConfigFieldError { key: String, message: String }` (new types, no change to existing ones; the
settings crate's own `FieldError` is re-exported from or converted to it). When `config.set`
or `config.unset` (dry run or not) is rejected for reasons tied to keys — a kind or scope
error, an owner's `validate` — the bridge and the app server answer an `RpcError` whose
`data` is the serialized `ConfigFieldErrors` and whose message stays what it is today. The
app server passes the bridge's `data` through unchanged. Messages never include values.
`<app> config` and `sapphire-bridge config` print each field error as `{key}: {message}`.
Reading it back from an `RpcError` is `ConfigFieldErrorsExt::from_rpc` in `sapphire-framework-bridge-client`
(an extension trait on `ConfigFieldErrors`, since the type lives in bridge-api).

## Restart banner

Above the settings pages and the Embedding screen, one line per process with pending keys
(entries with `pending_restart`, mapped to their process through the schema):

- `sapphire-bridge needs a restart to apply: {keys}` [Restart bridge]
- `the {app} server needs a restart to apply: {keys}` [Restart {app} server]

The button sends `Command::Restart`; the line shows a spinner meanwhile and the error after
a failure (e.g. a server run with `run()`: `this server cannot restart itself; restart its
service`). After either restart the settings reload. No order is imposed between the two.

## The Embedding screen

Its status display (models, device switches, whether a key is set) stays. Its edit forms —
`LocalForm`, `RemoteForm`, the API key field — and the GUI's `Command::Embedding` writes are
removed; the Embedding section's settings form is shown below the status instead.

## Code layout

The schema-driven pieces live in `sapphire-framework-gui` under `src/views/config/`
(pages, widgets, drafts, banner); the client changes under `src/client/`. Nothing moves to
`corundum-gui`, whose widgets and theme are a separate effort.

## Tests

- Model (no rendering): pages from a schema (placement rules, tabs per page); drafts survive
  reloads and mark changes; `Kind` checks per field; the batch Apply builds (set and unset
  split, scope); field errors routed to fields or the error line; which processes need a
  restart.
- Views, rendered headless (`test_support::render`): every kind; each tab; read-only bridge
  keys; no workspace selected; no app server; a page with a draft; the banner.
- Client integration (real bridge and app server, `tests/client.rs` style): `ConfigLoad`;
  a mixed bridge-and-app Apply; a rejected batch carries field errors; `Restart(Bridge)` and
  `Restart(App)` each come back and reload.
- Servers: a rejected `config.set` carries `ConfigFieldErrors` in `RpcError.data`, from the
  bridge and from the app server (own keys, and the bridge's passed through).

## Out of scope

- A theme or visual redesign (corundum-gui).
- Translation.
- A history of who changed a workgroup setting.
