# Shared CI

The CI, release and repository setup every sapphire-turtle Rust repository shares. The
workflows live in this repository's [`.github/workflows/`](../.github/workflows/) as
`shared-*.yml`; the files under [`templates/`](templates/) are what a repository copies in to
use them.

## Reusable workflows

Call them as `sapphire-turtle/sapphire-framework/.github/workflows/<file>@main`.

| Workflow | What it does |
|---|---|
| [`shared-rust-ci.yml`](../.github/workflows/shared-rust-ci.yml) | Pull request CI: `fmt`, then clippy and the tests on Linux x86_64 (`test`) and on Linux aarch64, macOS and Windows (`test-platforms`) |
| [`shared-release-plz.yml`](../.github/workflows/shared-release-plz.yml) | release-plz `release` and `release-pr`, with `RELEASE_PLZ_TOKEN`; outputs which packages were released and their tags |
| [`shared-release-binary.yml`](../.github/workflows/shared-release-binary.yml) | Builds one binary for the four release platforms and attaches `<asset>-<os>-<arch>[.exe]` to a release |
| [`shared-cargo-deny.yml`](../.github/workflows/shared-cargo-deny.yml) | `cargo deny check` against the repository's `deny.toml` |

Each one's inputs are documented at the top of its file.

Callers follow `@main`, so a change here reaches every repository at once. Keep changes
backward compatible: a new input gets a default that preserves today's behaviour. A change
that cannot be, lands together with the pull requests that adapt the callers.

## Conventions

- **Toolchain**: every repository pins the same `rust-toolchain.toml` channel and they are
  bumped together. CI installs nothing else.
- **Required checks**: each repository's `main` ruleset requires `ci / fmt` and `ci / test`
  (the caller job is named `ci`).
- **Tags**: an application's main binary is tagged `v{version}`, its desktop app
  `desktop-v{version}` (`git_only = true`, `publish = false` in `release-plz.toml`). Library
  crates make no tags.
- **Release files**: `<binary>-linux-x86_64`, `-linux-aarch64`, `-macos-aarch64`,
  `-windows-x86_64.exe`, the names `sapphire-framework-update` and the install script look
  for.
- **cargo binstall**: every crate with a released binary carries
  `[package.metadata.binstall]` pointing at those files, so `cargo binstall` installs the
  release instead of building it: copy the block from
  [`bridge/Cargo.toml`](../bridge/Cargo.toml), with the desktop app's `desktop-v` tag prefix.
  A crate not on crates.io (the desktop apps) installs with
  `cargo binstall --git https://github.com/sapphire-turtle/<repo> <crate>`.
- **Licenses**: libraries, CLIs and MCP servers are `MIT OR Apache-2.0`. An application's
  desktop and mobile apps (`<app>-desktop`, `<app>-mobile`) and the GUI library they share
  (`<app>-gui`) are `GPL-3.0-or-later` with an app store exception (sapphire-journal's
  `desktop/LICENSE.App-Store-Exception` is the model): the mobile apps are sold, and the
  desktop app shares much of their code. `sapphire-framework-gui` is a framework library and
  stays `MIT OR Apache-2.0`. So an application repository has no
  license file at its root; each component's directory carries its own, and the README lists
  them. This repository is all libraries and keeps `LICENSE-MIT` / `LICENSE-APACHE` at the root.
- **Secrets**: `RELEASE_PLZ_TOKEN` and, for a repository that publishes to crates.io,
  `CARGO_REGISTRY_TOKEN`.
- **Dependabot**: Cargo patch updates are grouped into one `cargo-patch` pull request. Dependabot
  pull requests are reviewed and merged by hand; nothing auto-merges them.
- **Labels**: `release` (release PRs), and `dependencies`, `github_actions`, `rust`
  (Dependabot).

## Setting up a repository

Copy from [`templates/`](templates/):

| File | Notes |
|---|---|
| `.github/workflows/ci.yml` | Set `linux-packages`, and `packages` / `post-test` if needed |
| `.github/workflows/cargo-deny.yml` | As is |
| `.github/workflows/release-plz.yml`, `release-binaries.yml` | For an application with a CLI and a desktop app; replace `<app>` |
| `.github/dependabot.yml` | As is |
| `rust-toolchain.toml` | As is |
| `deny.toml` | Add the repository's own `ignore` and `exceptions` entries, including the desktop app's GPL one |
| `.gitignore` | Append repository-specific entries below the marker |

Then create the `main` ruleset (no deletion, no force push, pull request required, the
required checks above).
