# Removing the bridge: each app server is its own sync node

Issue: #442 (tracking). Follow-ups: #440 (store / Flathub distribution, its own spec),
#441 (relay and discovery settings). Resolves #316 (the embedded relay: deleted).
Supersedes the bridge half of `2026-09-16-process-architecture-design.md` (§4.3, §5, the
bridge rows of §1), `2026-10-22-bridge-relay-design.md`, `2026-10-31-bridge-desktop-design.md`
and `2026-11-01-bridge-package-design.md`; revises `2026-10-09-bridge-embedding-design.md`,
`2026-10-10-embedding-settings-design.md`, `2026-10-08-primary-device-design.md` and
`2026-10-16-mobile-android-design.md`.

## Why

Sapphire Bridge is one daemon per user and host. It holds the device identity, the
workgroup ledger and pairing, authorizes peers, owns the host's one iroh endpoint and
splices data streams to the app servers, runs the one embedding model, and elects primary
devices. Every app server reaches it over IPC, and every client reaches it through its app
server (#314's relay).

That made sense while several apps had to share one identity. Now it costs more than it
gives:

- **Two things to install, run and update.** The bridge is its own Velopack package, its own
  service, its own release workflow and update feed, and its own window. An app whose bridge
  is down, outdated or of another API version does not sync, and the GUI needs a banner,
  a restart button and a relay error code (`BRIDGE_NOT_RUNNING`) for it.
- **Two GUI modes.** Desktop shows the bridge's pages as a summary that opens the bridge's
  window (`BridgePages::Linked`, `ClientMode::BridgeOnly`); the phone shows them in the app
  (`InApp`). The phone already hosts the bridge in-process, so the shared-daemon model is
  already the exception, not the rule.
- **Two settings owners.** `config.*` merges the app server's keys with the bridge's, and a
  mixed batch is dry-run on both and written in two steps.
- **Sandboxes.** Flatpak, the Mac App Store and the mobile stores isolate apps from each
  other. A per-user daemon shared by several sandboxed apps cannot exist there. Removing it
  does not make the apps store-ready by itself (#440 lists what else stands in the way), but
  every store path needs it gone.

Nothing needs cross-app device management: retiring a lost laptop app by app is
acceptable (decided 2026-11-26), and the only cross-app link in use (agent → timer) is an
ordinary MCP client connection that never went through the bridge.

Success means:

- no `sapphire-bridge` binary, package, service, data directory or API crate;
- each app server syncs its workspace on its own iroh endpoint, with its own pairing,
  device ledger, election and embedder;
- desktop and phone run the same code path, differing only in carrier (UDS / in-process)
  and directories;
- existing device ids — written into content, e.g. journal's `updated_by` — survive.

## Decisions

1. **The app server is the sync node.** It owns an iroh endpoint and `node.key`
   (`<data dir>/node.key`), accepts pairing, Hello and sync sessions on it directly, and
   no longer dials anything over IPC to sync. The data plane splice, `routes.toml`,
   `wake_on_sync` and the `ManagedBy` registration are gone.
2. **No workgroups. The workspace is the unit of membership.** The device ledger moves into
   the synced workspace at `{root}/.{app}/devices/<device-id>.toml` (the record format of
   `sapphire-framework-registry` unchanged). An invite ticket names the workspace; joining it
   is how a device gets the workspace. `workgroup.toml`, the workgroup's workspace list and
   `--remote-workspace` are gone.
3. **One identity per app, not per host.** Two endpoints cannot share a NodeId, so each app
   has its own key. A device is a record in each workspace's ledger; the same physical
   machine may have different device ids in different apps' workspaces. Content only ever
   refers to device ids of its own workspace, so nothing depends on them agreeing.
4. **The embedder runs in the app server, on both desktop and phone.** The `EmbedProvider`
   hook moves from the bridge to the app server; `sapphire-framework-bridge-embed` becomes
   `sapphire-framework-embed`, a feature an app opts into. Remote (OpenAI-compatible, which
   covers a local llama.cpp server) wins when configured; otherwise local Qwen3-VL as today.
   The model is workspace-scoped, so vector compatibility follows the workspace.
5. **The election moves with the endpoint.** Hello (`<prefix>/hello/1`, §2), the primary /
   secondary election and the availability record run per app server, among the devices of
   its workspace. The intended deployment is an always-on host running every app headless,
   which then wins every app's election as it wins the bridge's today.
6. **sapphire-sync stays** as the folder-sync app. It is an app like the others: its own
   endpoint, ledger and pages, and no special role.
7. **Migration is by hand, ids kept.** No automatic migration code beyond a one-shot ledger
   import (§7). Devices re-pair; a re-pairing device claims its old record so its id stays.
8. **Relays become app settings; the embedded relay is deleted.** `[net]` moves to the app
   server's settings registry (§8). The `embedded-relay` feature and `net.embedded_relay`
   are removed, not moved (#316): whoever wants a relay runs iroh's own `iroh-relay`.
   Folding a relay into app servers would need one per app, and a public relay that also
   holds app data is a needless exposure. Reworking the relay and discovery settings
   (custom relays, turning off n0's defaults, the sapphire-turtle.org relay) is #441.

## 1. Process model after the change

| State | Owner |
|---|---|
| Workspace files, caches, `sync.redb` | the app server (unchanged) |
| `node.key`, endpoint, pairing tickets | the app server |
| Device ledger | the workspace (`.{app}/devices/`), written by its app server |
| Embedding model and worker | the app server (feature `embed`) |
| Availability record | the app server (`<data dir>/availability.toml`) |

"Every state has exactly one writer" still holds, with one writer fewer.

IPC (`sapphire-framework-ipc`) stays for CLI, stdio MCP and GUI → app server. The control
plane of the bridge (`bridge.*` methods) is replaced by app-server methods (§5). Mobile keeps
`Carrier::InProcess` and stops starting a bridge task (`serve_in_process` is gone).

## 2. Endpoint and ALPNs

One iroh endpoint per app server, bound when sync is enabled and dropped when it is
disabled. It binds an ephemeral UDP port by default; a device setting `net.bind_port`
pins it for hosts behind a hand-configured firewall. Discovery and relays as today.

ALPNs are namespaced by a **protocol prefix** the app chooses: `<prefix>/pair/1`,
`<prefix>/hello/1`, `<prefix>/ws/2`. The prefix defaults to `AppContext::app_name`
(`sapphire-journal/ws/2`) and an app overrides it with `AppContext::protocol_prefix`.
The sapphire apps keep the default: their names are distinctive enough. An app on the
framework with a generic name (`photos`) sets one that is unlikely to collide, such as
`example.org/photos` or `alice_photos`. ALPNs are opaque byte strings (RFC 7301), so
dots and slashes are fine; the prefix must be non-empty and keep each ALPN within 255
bytes, checked when the context is built.

The prefix is the app's wire identity: changing it splits the app from every device that
still uses the old one, so it is fixed for the life of the app. Because it differs per app,
a ticket or address pasted into the wrong app fails at the TLS handshake instead of
reaching a session. On the endpoint:

- `<prefix>/pair/1` — pairing, unauthenticated, ticket-gated (the protocol of
  `sapphire/pair/1`, now carrying a workspace id instead of a workgroup id).
- `<prefix>/hello/1` (and its shared-connection variant) — Hello, members only.
- `<prefix>/ws/2` (and its shared-connection variant) — sync sessions, members only. The
  stream header loses `workspace_id`-based routing: an endpoint serves one workspace
  (#215), so a header naming another is `Refused`.

The old `sapphire/…` names are not kept: no device talks to both an old bridge and a new
app server (§7).

Authorization is "the caller's NodeId is a non-retired record of the current workspace's
ledger". It moves from `bridge::workgroup` to the server's sync runtime unchanged in logic.

## 3. Ledger in the workspace

- Path: `{root}/.{app}/devices/<device-id>.toml`, one file per device, same header and
  fields (`name`, `node_id`, `description`, `priority`, `created_at`, `retired_at`).
- Conflict policy: last-writer-wins, no conflict copies under `.{app}/devices/` — the rule
  the workgroup root enforces with its `*.conflict-*` ignore file, applied as a sync filter
  rule for this directory.
- Protected: workspace file operations (MCP write tools included) cannot change it,
  as with `.{app}/api_keys/` (`Error::ProtectedPath`).
- Switching workspace (`workspace.select`) switches ledger. A device that serves workspace A
  and later B is a member of each separately.

## 4. Pairing and joining

- `<app> device invite [--ttl 10m]` on a member prints a ticket (`sapphire:…`): the inviter's
  address, the workspace's sync id and name, a one-time secret. GUI: Settings › Devices ›
  Invite, with a QR code on desktop and phone.
- `<app> workspace join <ticket> [dir] [--as <device-name>]` on the new device pairs, writes
  the marker with the received sync id (as `sync.map` does now), and starts syncing.
  `--as` claims an existing, non-retired record — its `node_id` is replaced by the joiner's
  — instead of creating one; that is how a re-paired device keeps its id (§7).
- `workspace init --sync` on the first device creates the workspace and a ledger with one
  record, itself.
- `device list | retire | restore | set` operate on the current workspace's ledger, through
  the server or, with no server running, on the files directly (as `api-key` does).

## 5. API and CLI surface

- **Removed:** `sapphire-framework-bridge-api`, `sapphire-framework-bridge-client`, the
  `bridge.*` methods and the relay (`RELAYED` / `NOT_RELAYED`, `BRIDGE_NOT_RUNNING`), the
  `workgroup` CLI verb and MCP tool `workgroup_show`, `BridgeConnector`,
  `AppServer::sync_via_bridge`.
- **Added to `sapphire-framework-server-api`** (major bump, so every app bumps its server
  dependency in one step): `device.list` / `invite` / `retire` / `restore` / `set`,
  `sync.peers` (the old `bridge.peers`: peers, reachability, roles), `embed.info`.
  `server.info`'s `StatusReport` absorbs the bridge part of the status (endpoint up, node id,
  peers, embedder state).
- **In-process API** for app code that asked the bridge who it is: `SyncRuntime::me()`,
  `roles()`, `peers()`. sapphire-agent's `placement` switches to these and drops its
  `BridgeClient`.
- **Settings:** `net.*` and `embed.*` keys move to the app server's registry with the scopes
  of §6 and §8. `config.*` loses its two-owner merge and dry-run.
- `device_list` stays an MCP tool; `device invite` / `retire` stay human-only.

## 6. Embedding

- `EmbedProvider` / `EmbedFactory` move to `sapphire-framework-server`; the implementation
  crate is `sapphire-framework-embed` (renamed from `-bridge-embed`), behind the app's
  facade feature `embed`. An app without it reports `embed.info` → `enabled: false` and
  searches FTS-only, as with no bridge today.
- Keys and scopes:
  - workspace (`{root}/.{app}/config.toml`, synced): the model of each slot
    (`embed.local.model` / `dimension` / `max_tokens`, `embed.remote.endpoint` / `model` /
    `dimension`) — what was workgroup-shared.
  - device (`<config dir>/config.toml`): `embed.local.enabled` / `embed.remote.enabled`
    (auto: local off without AVX2, remote on), `embed.local.qtype`, `embed.cache_dir`.
  - secret: `embed.remote.key` in `<config dir>/secrets/` (0600), replacing
    `<bridge dir>/embedding.key`.
- **Shared weights on desktop.** With `embed.cache_dir` unset, desktop uses hf-hub's own
  default cache (`HF_HUB_CACHE`, else `HF_HOME/hub`, else `~/.cache/huggingface/hub`), so
  the model is on disk once for every app — and for any other Hugging Face tool of the
  user. This reverses the bridge's "never `HF_HOME`" rule, which no recorded reason backs.
  Phones (and a future sandboxed build, #440) default to the app's own cache dir.
  `embed.cache_dir` overrides both.
- **Memory.** Each app loads its model on first request and unloads after 10 minutes idle,
  as the bridge does. Two apps embedding at once hold two copies; accepted. A device that
  minds sets the local slot off and points remote at one llama.cpp / OpenAI-compatible
  server.

## 7. Migration (by hand)

Sole user (2026-10-31): no migration code beyond one import command.

1. On one device, with the old bridge still installed: `<app> device import-workgroup`
   copies the workgroup ledger's records (ids, names, priorities, retirements) into the
   current workspace's `.{app}/devices/`, clears every `node_id` but its own new one, and
   removes the bridge's routing row for the app. Run once per app.
2. On every other device: upgrade, then `<app> workspace join <ticket> --as <name>` with a
   ticket from step 1's device, per app.
3. When every app is migrated: `sapphire-bridge service uninstall`, uninstall the package,
   delete `<data root>/sapphire-bridge/`.

`import-workgroup` and the importer of #199's external devices are deleted one release
later. Untested on Windows and macOS hardware; say so in the PR.

## 8. Relay and network settings

This spec only moves the keys; their redesign is #441.

- Device scope (`<config dir>/config.toml`): `net.discovery`, `net.relays`,
  `net.use_default_relays`, `net.bind_port`.
- Workspace scope (`{root}/.{app}/config.toml`): `net.relays` and `net.use_default_relays`
  announced to every member — merged with the device's as `relays()` merges them today.
- Removed: `net.wake_on_sync` (it only fired for legacy `Spawned` registrations) and
  `net.embedded_relay` with the `embedded-relay` feature (decision 8).

## 9. GUI

- Settings gains, in every app on every platform: Devices (list, invite with QR, retire,
  priority), Network, Embedding. These are today's `InApp` pages minus workgroup.
- Removed: `BridgePages`, `ClientMode::BridgeOnly`, `app::run_bridge`, `bridge_window`, the
  Sapphire Bridge summary, the bridge restart button and its health banner.
- `FrameworkClient` connects to one process again; the snapshot's bridge part comes from
  `server.info`.

## 10. Crates and repository

| Before | After |
|---|---|
| `sapphire-framework-bridge` | split: endpoint, pairing, invite, Hello, election, availability and relay configuration into `sapphire-framework-net` (used by `-server`); control plane, routes, data splice, workgroup, `wgsync`, status file and the embedded relay deleted |
| `sapphire-framework-bridge-embed` | `sapphire-framework-embed` |
| `sapphire-framework-bridge-api`, `-bridge-client` | deleted (crates.io: yank nothing, publish no more) |
| `bridge/headless`, `bridge/desktop` | deleted, with `release-bridge.yml`, `docs/release-checklist-bridge.md` and the bridge's update feed |
| `sapphire-framework-registry` | unchanged; its root is now `.{app}/devices/` |

`-server` gains a dependency on iroh through `-net` behind a new `p2p` feature (on in the
apps' facade `native` and `mobile`), so a build without it — most of `-server`'s tests, and
the `StubBridge`-style fixture that becomes a loopback transport — stays iroh-free.

## 11. Order

1. `sapphire-framework-net` from the bridge's pieces, with the bridge still building on it
   (pure move, tests follow).
2. The server owns the endpoint and ledger; `workspace join` / `device *`; server-api major.
   The bridge path is not kept alongside: every app moves in the same framework release.
3. Embedding into the server; `-embed` rename; settings scopes.
4. GUI: one mode; Devices / Network / Embedding pages; mobile drops `serve_in_process`.
5. Apps: journal, agent (`placement`), ledger, timer, tally, sync — each bumps framework and
   server-api, removes `sync_via_bridge`, opts into `embed` where it searches.
6. Delete the bridge crates, binaries, package, workflow and docs; rewrite
   `ARCHITECTURE.md`'s bridge and workgroup sections.

## 12. Testing

- The bridge's switchboard and iroh transport tests become server tests: two servers on
  loopback transports pair, sync, retire; a retired NodeId is refused on every ALPN.
- Election tests move as they are (they never needed the switchboard).
- Ledger: concurrent edits of one record converge with no conflict copy; MCP write tools get
  `ProtectedPath` under `.{app}/devices/`.
- Join with `--as`: the record keeps its id, its old `node_id` is refused afterwards.
- Embedding: provider present / absent / remote; workspace-scoped model change reaches a
  peer and its vectors are re-targeted.
- Two apps on one host: two endpoints, a ticket of one app refused by the other at the ALPN
  (`<prefix>/pair/1` does not match); an override of `protocol_prefix` is used in every
  ALPN; a prefix that is empty or makes an ALPN longer than 255 bytes is rejected.

## 13. Risks

- **N × M pairing.** Five apps on three devices is ten joins after the first. Accepted; QR
  invites keep it short.
- **Duplicate model memory** when two apps embed at once (§6). Accepted; remote is the escape.
- **Big-bang API change.** Every app moves server-api major together, with no bridge path to
  fall back on. Accepted for a sole user (2026-11-26): an app not yet released on the new
  framework simply does not sync until it is.
- **More Hello traffic** (one per app per peer). Small; the 10 s / 40 s timing stays.
- **Sandboxing is not done by this.** CLI / stdio MCP reach, service registration and
  workspace locations remain (#440).
