# API keys in the workspace, without the bridge

Issue: #364. Builds on corundum-framework's `corundum-framework-keys` (its spec:
corundum-framework's `docs/superpowers/specs/2026-10-31-api-keys-design.md`). Replaces
#199's external devices (`2026-10-11-external-devices-design.md`). Related: #357 (single
binary for people who do not sync), #363 (the bridge's own GUI).

## Why

An HTTP client of an app (a recording pendant, a webhook, another machine's ACP client)
proves who it is with a bearer token. Since #199 the token belongs to an *external device*
in the workgroup's ledger, which the bridge owns: the app's HTTP layer asks the bridge about
every new token and answers 503 when it cannot. So without the bridge, or without a
workgroup, no HTTP client can be let in — and #357 expects many people not to sync.

The ledger was synced so that a key set up once works on every host, and so that a host
without HTTP can manage the keys of one that has it. The second is what matters in
practice: a server that publishes HTTP has no GUI, and the machine with the GUI publishes no
HTTP. A workspace is synced too, and every host that serves an app's workspace holds the
same copy of it, so the keys can live there instead, with no bridge involved.

## Decisions

1. **Keys live in the served workspace:** `<workspace>/.<app>/api_keys/<id>.toml`, a
   `corundum_framework_keys::KeyStore`. `.<app>/` already syncs (the synced vectors live
   in `.<app>/embedded/`), so nothing in the sync filter changes. Without a workgroup the
   directory is simply local.
2. **A key belongs to the workspace, not to the workgroup or the host.** There is no
   `apps` list: the store is one app's. Switching the served workspace switches the keys.
   A client that uses two apps holds a key from each.
3. **The bridge takes no part.** The HTTP layer checks the store itself; there is no
   `Unavailable`, no 503 for a missing bridge. A key added on any host that holds the
   workspace is accepted wherever the workspace has synced to.
4. **Name: API key.** The CLI subcommand becomes `api-key`; the GUI page "API keys". The
   words "external device" go. New tokens start `sapphire-ak-`.
5. **Existing external devices are imported once,** keeping their ids and hashes, so their
   tokens keep working and anything that names them by id (sapphire-agent's room profiles)
   still matches.
6. **The bridge's external-device surface is removed:** its control-plane methods, its CLI,
   the relay entries, and the bridge's use of the ledger. The ledger files stay where they
   are, unused; nothing deletes them.
7. **An app's own tools must not write under `.<app>/api_keys/`.** A tool that could would
   let an agent issue itself a key. The framework has no file-writing tool and its indexer
   skips dot directories; each app checks its own tools when it adopts this (see Rollout).

## Design

### Where the store is

`sapphire_framework_server::api_keys::dir(ctx, workspace_root) -> PathBuf` returns
`<root>/.<app>/api_keys`. Every user of the store goes through it: the HTTP verifier, the
CLI, the GUI and the import.

### HTTP

`AppServer`'s HTTP layer uses a `WorkspaceKeyVerifier`, which wraps
`corundum_framework_keys::KeyVerifier` for the workspace this app serves:

- it finds the served root in the app's selection file (`workspace.toml`, which the
  server rewrites on a switch), checking the file's modification time on each request and
  re-reading it only when that changed, so any process — the server, or an app building
  its own verifier — can make one with `WorkspaceKeyVerifier::for_app(ctx)`;
- it keeps one `KeyVerifier` per root, replacing it (and so its cache) when the root
  changes;
- with no workspace served it refuses every token (401).

`sapphire-framework-keys` keeps its `Verifier` trait, `AuthConfig` and `protect`: apps
wrap the verifier in their own (sapphire-agent maps a key to a room profile).
`BridgeVerifier` and `Verdict::Unavailable` go, and `Authenticated::id` becomes corundum's
`KeyId` (imported keys keep their grain-id text; new keys get a UUIDv7). With no verifier
configured, `protect` refuses with 401 instead of 503. corundum's own axum layer is
asteria's; sapphire keeps its own because of the trait.

The 30 s success cache stays (corundum's default): a retire takes effect within the sync
delay plus 30 s, as it did with the workgroup ledger.

### CLI

`FrameworkCommand::ApiKey(corundum_framework_keys::KeyCommand)`, as `api-key`:

- it opens the store of the workspace this app serves, as recorded in its selection file
  (the same record `workspace show` reads), and works on the files, with or without a
  running server — the headless case of the issue;
- with no workspace selected it says so and exits 1;
- `external-device` stays one release as a hidden alias that warns and runs `api-key`.
  Its `--app` arguments are refused with a message: keys are no longer per app.

### GUI

Settings › External devices becomes Settings › API keys: corundum-framework-gui's
`KeysPage` with a `LocalKeys` backend over the served workspace's store. The GUI process
reaches the workspace files directly, as the CLI does (the desktop GUI runs on the host,
and on a phone the server is in the same process). When the served workspace changes, the
page is rebuilt over the new store. With no workspace served, the page says so instead.

`views::ExternalDeviceList`, the client's `external_devices` snapshot and its
external-device commands are removed.

### Import from the workgroup ledger

On server start, and whenever a workspace becomes the served one, the server runs the
import unless this host has done it for this app before:

1. Read `<bridge dir>/workgroups/*/root/external_devices/*.toml` directly — read-only,
   without the bridge running — with `sapphire_registry::ExternalDevices`.
2. For each record that is not retired and whose `apps` contains this app, `import` a
   `Key` into the served workspace's store: same id, same name and description,
   `hash = "sha256:<token_sha256>"`, same `created_at` and `rotated_at`. An id the store
   already has is skipped (another host imported it and the workspace synced).
3. When every record was handled, write `<app data dir>/api_keys_imported` so a later
   workspace switch does not import the old keys again.

A failure (unreadable ledger, unwritable store) is logged and retried at the next start.
Every host imports the same records under the same ids, so hosts that import at once agree
once the workspace syncs.

### Bridge

- `sapphire-framework-bridge-api`: `EXTERNAL_DEVICE_*`, their params and results,
  `ExternalDeviceCommand` and `ExternalDeviceRequest`/`Outcome` are removed, and so are
  their entries in `RELAYED` and `NOT_RELAYED`. Breaking: the API's major version goes up.
- `sapphire-framework-bridge`: `external.rs`, the control plane's handlers and
  `sapphire-bridge external-device` are removed. `Workgroup::external_devices` goes.
- `sapphire-framework-bridge-client`: the external-device calls go.
- `sapphire-framework-registry`: `ExternalDevices` stays, deprecated, for the import; it
  goes when the import does.

## Errors

- A store file that does not parse is skipped with a warning (corundum's behaviour); the
  other keys keep working.
- The workspace's root gone: the verifier refuses, as with no workspace.
- `api-key` with no workspace selected: an error naming `workspace select`, exit 1.

## Testing

- HTTP end to end with no bridge: `api-key add` on the workspace, a request with the
  token → 200; a wrong token → 401; after `api-key retire` and the TTL (short in the test)
  → 401. Replaces the 503 test.
- Sync stand-in: a key file copied into another host's copy of the workspace is accepted
  there, with no restart.
- Workspace switch: a key of the old workspace is refused after the switch; one of the new
  workspace is accepted.
- Import: from a temporary workgroup ledger, only unretired records listing the app are
  imported, with id and hash kept, and the old token is accepted; a second run imports
  nothing; an id already in the store is skipped; the marker stops a later switch from
  importing.
- CLI: `external-device add` warns and adds an API key; `--app` is refused.
- GUI: Settings shows "API keys" with corundum's page over the served workspace's store
  (egui_kittest).
- bridge-api: the relay-list test still accounts for every method.

## Rollout

1. corundum-framework releases `corundum-framework-keys` and `KeysPage`.
2. This repository adopts it in one release: HTTP, CLI, GUI, import, bridge removal. The
   bridge-api major version goes up; the CHANGELOG's Breaking section says that tokens keep
   working, that keys are now per workspace, and that `external-device` is now `api-key`.
3. Each app updates the framework and checks that no tool writes under `.<app>/`.
   sapphire-agent also moves `device_auth.rs` off `BridgeVerifier` and its startup check
   off `external_device.list`; its room profiles keep naming keys by name or id.
4. A later release removes the `external-device` alias, the
   import and `ExternalDevices`.

## Out of scope

- #363: the bridge's own GUI. After this, the bridge has no external-device screen to
  host.
- Sharing one key between apps.
- Recording which key made a change (#92): when it comes, the change records the key's id
  and name, so no other host needs the store to show it.
