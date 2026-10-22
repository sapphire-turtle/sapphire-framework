# Clients reach the bridge through their app server

Part 3a of #314 (the settings registry, `2026-10-21-settings-registry-design.md`). Part 3b —
the app's own settings, `<app> config` and `server.restart` — builds on this and has its own
spec.

## Goal

An application's CLI and GUI talk to one process: their app server. Today they also connect
to the bridge directly for workgroups, devices, embedding and external devices, so every
client handles two endpoints, two connection states and two API versions. After this change
the app server relays those calls, and a client needs the bridge's types but never its
endpoint.

Success means:

- the app's CLI and GUI open no connection to the bridge;
- every command and screen that worked against the bridge works the same through the app
  server, with the same messages;
- `sapphire-bridge`'s own CLI is unchanged.

## Decisions

1. **Transparent relay.** The app server serves an allowlist of the bridge's methods under
   their own names and forwards each call's params unchanged, returning the bridge's result
   or error unchanged. A client uses the existing typed `BridgeClient` over its app-server
   connection. There is no envelope method. (In Part 3b the app server stops forwarding
   `config.*` and answers it itself, merging its own keys with the bridge's; clients do not
   change.)
2. **`sapphire-framework-server-api` depends on `sapphire-framework-bridge-api`** and
   re-exports what clients need from it. This makes server-api's major move with
   bridge-api's; the version numbers are left to release-plz.
3. **The app server is required.** Without a running app server, the app's bridge-facing
   commands fail with `no {app} server is running`. An app server is required in any case;
   the bridge is optional (sync and embedding only).

## The relay

### Allowlist

Forwarded (the constants in `sapphire-framework-bridge-api`):

| Group | Methods |
|---|---|
| state | `bridge.status`, `bridge.peers`, `bridge.workspaces` |
| workgroup | `bridge.workgroup_create`, `bridge.invite`, `bridge.join`, `bridge.device_retire`, `bridge.device_priority_set` |
| embedding settings | `embed.settings`, `embed.model_set`, `embed.device_set`, `embed.key_set`, `embed.key_clear` |
| external devices | `external_device.list`, `external_device.add`, `external_device.rotate`, `external_device.retire`, `external_device.restore`, `external_device.set_apps` |
| settings | `config.schema`, `config.list`, `config.get`, `config.set`, `config.unset` |
| restart | `bridge.restart` (see below) |

Not forwarded, because only an app server or the HTTP listener calls them on its own behalf:
`bridge.register`, `bridge.unregister`, `bridge.incoming`, `embed.info`, `embed.embed`,
`embed.embed_images`, `external_device.authenticate`. A client calling one gets the usual
`method not found`.

The allowlist is one constant slice in the relay module; a test asserts that every
client-facing method the GUI and the app CLI call is on it.

### The bridge connection

The app server keeps at most one connection to the bridge. When the app runs sync through
the bridge (`sync_via_bridge`), the relay uses the sync runtime's connection. Otherwise the
relay makes its own on the first relayed call, with the same connector, and reconnects the
same way (backoff from 500 ms to 30 s, as `keep_connected` does today). An app that never
relays never connects.

### Errors

- No bridge running, or the connection drops mid-call: the relay answers with the new error
  code `BRIDGE_NOT_RUNNING` (server-api; in the server-defined range of JSON-RPC codes) and
  the message `no sapphire-bridge is running`. The client turns it back into the
  `NotRunning` error it gets today, so its messages do not change.
- Anything the bridge answers — including `method not found` from an older bridge that lacks
  a method — goes back unchanged.

### Restart

`bridge.restart` is forwarded, and its answer waits until the app server is connected to the
new bridge (or 30 s pass, which is an error naming the timeout). A client therefore sees the
restart finish before the call returns, and needs no polling loop of its own.

### What is not logged

Relayed params and results are never logged, at any level: `embed.key_set`, `config.set`
of a secret and `external_device.add` / `rotate` carry API keys and tokens. The relay logs
only the method name and the outcome (ok, the error code).

## Clients

### The app's CLI

`workgroup`, `device`, `embedding` and `external-device` connect to the app server
(`connect_app`, kind `cli`) instead of `BridgeClient::connect_running`, and call the same
`BridgeClient` methods over that connection. `workspace show` gets the workgroup list
through the same connection instead of a second one to the bridge.

### The GUI

The GUI keeps one connection, to its app server. The bridge half of its poll (`status`,
`peers`, `workspaces`, `embed.settings`, `external_device.list`) and `bridge_command` run
over it. `Endpoints.bridge` goes; `ServiceExes.bridge` stays, since installing the bridge's
service still runs `sapphire-bridge service install` locally. The GUI drops its direct
dependency on `sapphire-framework-bridge-client`. With no app server running, the GUI shows
the bridge's state as unknown.

The in-process mobile build (bridge and app server in one process) needs no change beyond
this: its GUI already reaches the app server in process.

### Unchanged

`sapphire-bridge`'s own CLI, which has no app server to go through. The app server's own
uses of the bridge (registration, sync, embedding, the HTTP token check).

## Compatibility

The app's CLI, GUI and app server ship in one binary, so they agree. The pair that can
differ is the app server and the bridge, which sync already deals with. Params pass through
untouched, so fields a newer bridge adds reach the client, and a method an older bridge
lacks fails as it would have directly.

## Tests

- Relay integration tests (app server against `StubBridge`): an allowlisted method returns
  the bridge's answer; a non-allowlisted one is `method not found`; no bridge gives
  `BRIDGE_NOT_RUNNING`; an app without sync relays over its own lazy connection;
  `bridge.restart` returns after the app server reconnects; params of `embed.key_set` never
  appear in the captured logs.
- The allowlist test above.
- The app CLI's bridge-facing commands, run against an app server with a stub bridge
  behind it.
- The GUI client's exec and poll paths over a single app-server connection.

## Out of scope

- The app's own settings, `<app> config`, `server.restart`, the merged `config.*` (Part 3b).
- Relaying notifications: no client subscribes to the bridge's.
- The GUI's settings screen (Part 4).
