# Install scripts

> 日本語: [README.ja.md](README.ja.md)

These scripts install the headless edition of
[`sapphire-bridge`](../bridge/headless/README.md) and [`sapphire-sync`](../sync/README.md)
(the CLI and the server in one binary) from this repository's GitHub releases. They are meant
for servers and other headless machines. Each sapphire application has scripts like these in
its own repository (`https://github.com/sapphire-turtle/sapphire-<app>/tree/main/install`);
they do not install the bridge, so a host that syncs installs it from here.

The desktop apps are not installed by these scripts. Each ships as an installer on its
releases, which carries the CLI as well and keeps both up to date.

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
| `bridge` | `sapphire-bridge` |
| `sync` | `sapphire-sync` |

| `install.sh` | `install.ps1` | |
|---|---|---|
| `--dir <dir>` | `-Dir <dir>` | Install into `<dir>` instead of `~/.local/bin` |
| `--require-signature` | `-RequireSignature` | Fail, rather than warn, when minisign is not on `PATH` |

- Releases exist for Linux x86_64 / aarch64, macOS aarch64 and Windows x86_64.
- They install the latest version: the highest `x.y.z` among the repository's releases with
  the application's tag prefix, the same rule as the `update` command. To change versions
  later, run the binary's own `update`.
- Each download is checked before it is installed:
  - against the SHA-256 GitHub records for the release file (`sha256sum` or `shasum` on
    Linux and macOS, `Get-FileHash` on Windows);
  - when [minisign](https://jedisct1.github.io/minisign/) is on `PATH`, against its signature
    (`<file>.sig`) by the organization's key, `F4A73F39AA17C8EB`
    (`sapphire_framework_update::UPDATE_PUBKEY`, the key `update` checks), which must also
    name the file it was made for.

  Without minisign the scripts warn and check the SHA-256 alone; with `--require-signature`
  they stop instead. A download that fails either check, or has no SHA-256 or signature to
  check, is never installed.
- A binary already installed by `cargo install` or `cargo binstall` (or, on Linux and macOS,
  by a package manager) is left alone; update it the same way.
- An application whose desktop app is already installed is skipped: the desktop package
  includes the same CLI and server, and a second copy would run a second server. It is
  recognised by `%LocalAppData%\sapphire-<app>-desktop\Update.exe` on Windows (the
  package's id; `%LocalAppData%\sapphire-<app>` is the app's data), `Sapphire <App>.app` in
  `~/Applications` or `/Applications` on macOS, and a `~/.local/bin/sapphire-<app>` that
  points to an AppImage on Linux. On Linux the desktop app makes that link only when asked
  (Settings › About › Command line), so an AppImage that never made it is not recognised:
  install either the AppImage or the CLI on a host, not both.
- `install.ps1` adds the directory to the user `PATH`; `install.sh` tells you when it is not on
  `PATH`.
- These scripts used to install every application, and the bridge with each. Asked for an
  application that has its own scripts now (`journal`, say), they stop and print that
  script's command; `--no-bridge` is accepted and does nothing, and `--version` is gone.

## With cargo

Where `~/.cargo/bin` comes first on `PATH`, as on a machine you develop on, install the
releases there with [`cargo binstall`](https://github.com/cargo-bins/cargo-binstall) instead:
it downloads the same release files rather than building them, and `update` tells you to run
it again.

```sh
cargo binstall sapphire-bridge-headless sapphire-sync-headless
```

## The scripts of each repository

These scripts are made from
[sapphire-turtle/.github's template](https://github.com/sapphire-turtle/.github/tree/main/templates/install),
as are the applications'. A change to how they work goes to the template first, and then to
each repository; only the tables at the top differ. The release files must follow
[the shared naming](https://github.com/sapphire-turtle/.github#conventions).
