# sapphire-bridge-desktop

Sapphire Bridge's desktop app: run bare, it opens the bridge's window — its status,
workgroup, devices, embedding, appearance, logs and About. `--page <status|workgroup|devices|embedding|appearance|logs|about>`
opens it on a page, or brings the window already open to the front on it; the apps' "Open
Sapphire Bridge" buttons start it that way.

Given a subcommand it is `sapphire-bridge`: `sapphire-bridge-desktop serve` runs the bridge,
and the service manager may start it so. The window is a client of the bridge; closing it
stops nothing.

## Installing

Each framework release carries Sapphire Bridge's installers, made with
[cargo-packager](https://github.com/crabnebula-dev/cargo-packager) by
`.github/workflows/release-bridge.yml` (sapphire-turtle/.github's `release-packager.yml`):

| OS | File |
|---|---|
| Windows | `sapphire-bridge-desktop_<version>_x64-setup.exe`, for you alone, no administrator (not code-signed yet: SmartScreen warns) |
| macOS | `sapphire-bridge-desktop_<version>_aarch64.dmg` (the app signed and notarized when the release had the keys) |
| Linux | `sapphire-bridge-desktop_<version>_x86_64.AppImage` |

Each file has a minisign signature beside it (`.sig`), against sapphire-turtle's key; the
updates are checked against it before they are installed.

The package holds this binary, as the command `sapphire-bridge`. On Windows the install puts
it on `PATH` (a small console program of that name runs it, so the command line works while
the window has no console); on macOS and Linux About › Command line links it
(`/usr/local/bin`, asking for an administrator; `~/.local/bin`). No service is registered at
install.

**First run.** The window's first start in the package welcomes you and asks whether to keep
Sapphire Bridge running in the background — Yes registers `sapphire-bridge serve` with the OS
service manager (systemd user unit, LaunchAgent, Task Scheduler). About › Service shows it
afterwards, to stop, start, remove or repair.

**Updates.** The window checks this repository's latest release (its
`sapphire-bridge-latest.json`) at start and once a day,
downloads a newer one, and About › Updates offers to restart into it; the service is stopped
before the update and started after. `sapphire-bridge update [--check]` does the same from
the command line, and so does an app's `<app> update`, which runs it.

Built without the `package` feature (`cargo build`), it is the plain binary, also released
as `sapphire-bridge-desktop-<os>-<arch>`; `update` then refuses and says how to update. The
command line alone, for servers, is [`sapphire-bridge`](../headless/README.md).
