<img src="docs/icon.svg" alt="" width="72" height="72">

# sapphire-bridge

The host-wide sapphire daemon. One process per OS user, shared by every sapphire application
on the machine.

> 日本語版: [README.ja.md](README.ja.md)

## What it is

Apps each run their own app server, and that server owns a workspace's cache — a file only one
process may open. The bridge sits above them. It holds this host's device identity, knows which
workgroups the host belongs to, and tells an app server where its peers are.

It never looks inside a workspace: it routes *to* the app server that owns one, and decides
*whether* a peer's bytes may flow, never what they mean.

## Installing

Every sapphire application's install script installs the bridge along with it. For the
bridge alone:

```sh
# Linux, macOS
curl -fsSL https://raw.githubusercontent.com/sapphire-turtle/sapphire-framework/main/install/install.sh | sh -s -- bridge
```

```powershell
# Windows
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/sapphire-turtle/sapphire-framework/main/install/install.ps1))) bridge
```

It goes to `~/.local/bin`, where `sapphire-bridge update` can replace it later. Then run it as
a service with `sapphire-bridge service install`. See [`install/`](../install/README.md) for
the options and the other applications.

## Running it

```console
$ sapphire-bridge serve
$ sapphire-bridge status
$ sapphire-bridge workgroup create home --device-name laptop
$ sapphire-bridge device list
```

With no subcommand at all, it prints the help. `serve` starts the bridge and stays in the
foreground; every other subcommand is a one-shot command against a running bridge — except
`workgroup create` and `device retire`, which work directly on the bridge directory because
there is nothing to ask yet (or, for `retire`, no control-plane method).

> an OS service unit installed by an older build still names `run`; run
> `sapphire-bridge service install` again after upgrading, so the unit names `serve`.

Starting a second bridge is not an error: it reports the pid of the one already running and
exits non-zero.

### Updating

```sh
$ sapphire-bridge update --check   # only say whether a newer release exists
$ sapphire-bridge update           # download and install it, after asking
```

`update` replaces the binary with this platform's file from the newest GitHub release,
downloaded over HTTPS. It never downgrades, and it leaves a binary installed with
`cargo install`, `cargo binstall` or a package manager alone (it says what to run instead).
Restart the service afterwards to run the new version.

## Commands

| Command | What it does |
|---|---|
| `serve` (default) | Run the bridge in the foreground |
| `status` | Report the running bridge's version, node id, workgroup and registered workspaces |
| `device list` | List the workgroup's devices, and which are reachable |
| `device retire <selector>` | Retire a device by name or id |
| `workgroup create <name> --device-name <name>` | Found a workgroup, recording this host as its first device |
| `workgroup list` | Show the workgroup this host belongs to |
| `workspace list` | List the workspaces this host serves (read-only) |

`pair`, `workgroup join` and invites are not implemented yet.

## Where it keeps things

`<platform data root>/sapphire-bridge/`, overridden whole by `SAPPHIRE_BRIDGE_DIR`:

```
sapphire-bridge/
    format          # directory format version
    node.key        # iroh secret key -> this device's node id
    bridge.lock     # single-instance guard, not a role election
    config.toml     # this device's settings: [net], [embed]
    routes.toml     # workspace_id -> the app server that owns it
    run/            # IPC sockets and spawn locks (SAPPHIRE_RUNTIME_DIR overrides it)
    workgroups/<workgroup-id>/
        root/       # device ledger
        root/config.toml  # the workgroup's shared settings
        root/devices/<device-id>.toml
```

The directory is mode `0700` on Unix, and `node.key` is `0600`: that file **is** this device's
identity.

## Configuration

`config.toml`, every field optional (`[net]` below; `[embed]` holds the embedding settings):

```toml
[net]
wake_on_sync = true    # start a stopped app server when a peer asks for its workspace
discovery = true       # use discovery services to find peers
relays = []            # relay URLs; empty disables relays
```

Read and write settings by key with `sapphire-bridge config list|get|set|unset`
(`--device` / `--workgroup` choose the file; `--restart` restarts the bridge for keys that need
it, and `sapphire-bridge restart` does so on its own). A secret such as `embed.remote.key` is
prompted for, or read from standard input with `--stdin`, and is never printed.
A workgroup's shared settings live in `root/config.toml`. Directories from before this layout
(`net.toml`, `embedding.toml`) are migrated on first start and kept as `*.toml.v1`.

`RUST_LOG` sets the log filter, as everywhere else in the framework.

## License

MIT OR Apache-2.0.
