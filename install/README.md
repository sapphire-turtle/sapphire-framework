# Install scripts

> 日本語: [README.ja.md](README.ja.md)

One pair of scripts installs every sapphire application from its GitHub release, and
installs or updates [`sapphire-bridge`](../bridge/README.md) along with it: the applications
sync through the host's bridge.

```sh
# Linux, macOS
curl -fsSL https://raw.githubusercontent.com/sapphire-turtle/sapphire-framework/main/install/install.sh | sh -s -- <app>...
```

```powershell
# Windows
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/sapphire-turtle/sapphire-framework/main/install/install.ps1))) <app>...
```

| App | Installs |
|---|---|
| `bridge` | `sapphire-bridge` only |
| `agent`, `agent-desktop` | `sapphire-agent`, `sapphire-agent-desktop` |
| `journal`, `journal-desktop` | `sapphire-journal`, `sapphire-journal-desktop` |
| `sync`, `sync-desktop` | `sapphire-sync`, `sapphire-sync-desktop` |

| `install.sh` | `install.ps1` | |
|---|---|---|
| `--dir <dir>` | `-Dir <dir>` | Install into `<dir>` instead of `~/.local/bin` |
| `--version <x.y.z>` | `-Version <x.y.z>` | Install that version instead of the latest (one app only) |
| `--no-bridge` | `-NoBridge` | Leave `sapphire-bridge` alone |

- Releases exist for Linux x86_64 / aarch64, macOS aarch64 and Windows x86_64.
- The latest version is the highest `x.y.z` among the repository's releases with the
  application's tag prefix, the same rule as the `update` command.
- A binary already installed by `cargo install` (or, on Linux and macOS, by a package manager)
  is left alone; update it the same way.
- `install.ps1` adds the directory to the user `PATH`; `install.sh` tells you when it is not on
  `PATH`.

## Adding an application

Add a row to the table at the top of both scripts: the name, the repository, the release
file name before `-<os>-<arch>`, and the tag prefix in the application's `release-plz.toml`.
The release files must follow [the shared naming](../ci/README.md#conventions).
