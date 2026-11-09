# sapphire-sync-headless

The headless edition of [sapphire-sync](https://github.com/sapphire-turtle/sapphire-framework/tree/main/sync) — peer-to-peer
file sync over [sapphire-framework](https://github.com/sapphire-turtle/sapphire-framework) — for hosts
without a GUI. Its one binary, `sapphire-sync`, is the app server (`sapphire-sync serve`) and the CLI
that talks to it. The desktop app's binary runs the same commands and is put on PATH under the same
name.

## Installation

### Install script

```sh
# Linux, macOS
curl -fsSL https://raw.githubusercontent.com/sapphire-turtle/sapphire-framework/main/install/install.sh | sh -s -- bridge sync
```

```powershell
# Windows
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/sapphire-turtle/sapphire-framework/main/install/install.ps1))) bridge sync
```

This installs `sapphire-sync` and `sapphire-bridge`, which it syncs through, into
`~/.local/bin`, checking each download's SHA-256 and signature first.

### From crates.io

```sh
cargo binstall sapphire-sync-headless   # the release file
cargo install sapphire-sync-headless    # or build it
```

Either installs the `sapphire-sync` binary. The desktop app's binary runs the same commands; install
only one of the two.

## Usage

See [sapphire-sync's README](https://github.com/sapphire-turtle/sapphire-framework/tree/main/sync#quick-start).

## License

MIT OR Apache-2.0.
