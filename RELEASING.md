# Releasing

Releases for this repository are automated with [release-plz](https://release-plz.dev/).

## Normal flow

1. When changes are pushed to `main`, release-plz automatically creates or updates a release PR.
2. Review the release PR (version, `CHANGELOG.md`, `Cargo.toml`).
3. Merging the PR creates the `v{version}` tag, the GitHub release, publishes to crates.io, and builds the `sapphire-bridge` binaries (Linux x86_64 / aarch64, macOS aarch64, Windows x86_64) and attaches them to the release. To (re)upload them for an existing tag, run the "Release sapphire-bridge binaries" workflow manually with that tag.

## sapphire-sync

sapphire-sync (`sync/`, #394) is released with the framework, under the same `v{version}` tag and GitHub release.

| Crate | Version | Published |
| --- | --- | --- |
| `sapphire-sync-headless` (the `sapphire-sync` binary, no GUI) | framework | crates.io, release files |
| `sapphire-sync-desktop` (the GUI, which runs the same commands) | framework | release files and Velopack package only (`publish = false`) |
| `sapphire-sync-cli`, `-server`, `-core` | framework | crates.io |
| `sapphire-sync-api` | its own | crates.io |

When the release PR's merge publishes `sapphire-sync-headless`, "Release sapphire-sync binaries" (`release-sync.yml`) builds both editions' binaries (`sapphire-sync-headless-<os>-<arch>`, `sapphire-sync-desktop-<os>-<arch>`) and the desktop app's Velopack package and attaches them to the release, beside the bridge's. The package uses its own Velopack channels (`sync-linux`, `sync-osx`, `sync-win`): the channel names the feed file (`releases.<channel>.json`), and the bridge's package already has `linux`, `osx` and `win` on the same release. To (re)upload them for an existing tag, run the workflow manually with that tag.

`sapphire-sync-api` is the sync server's own API, versioned the way `sapphire-framework-bridge-api` and `-server-api` are: its major is the `app_api` the server reports in the IPC handshake (`API_VERSION`).

Releases up to 0.14.1 were made in the old sapphire-turtle/sapphire-sync repository, which keeps their tags and files. The crates.io name `sapphire-sync` was the app's binary from 0.13.0 to 0.14.1 (and sapphire-workspace's git sync library up to 0.12.1); the binary is `sapphire-sync-headless` since.

## Version bump rules

release-plz infers the version from conventional commits (assuming pre-1.0 `0.x.y`).

- `feat:` / `fix:` / `chore(deps):` and other non-breaking changes → patch bump
- `feat!:` / `chore!:` or a `BREAKING CHANGE:` footer → minor bump
- Public API changes you make yourself (function signatures, public struct fields, etc.) → minor bump (mark the commit with `!`)

When an error type re-exposes a foreign crate via `#[from] some_crate::Error`, a major bump of that dependency is technically breaking, but the practical impact is usually nil. Patch is generally fine — this is the pragmatic stance most of the Rust ecosystem takes.

## Special case: `-sys` crate updates

A major bump of a `-sys` crate (`libgit2-sys`, `libsqlite3-sys`, `openssl-sys`, anything with `links = "..."` in its Cargo.toml) **must** become a minor bump on our side.

Reason: Cargo refuses to resolve a dependency graph that contains two different majors of the same `links`-bearing crate. If we ship a patch release that pulls in a new `-sys` major, downstream crates that pin a different `-sys` major fail to build. This is a hard error, not an API compatibility break, and the user cannot work around it.

The [Cargo SemVer guide's `-sys` section](https://doc.rust-lang.org/cargo/reference/semver.html) explicitly treats this as a major change.

### Dependencies to watch for

`-sys` crates currently reachable from this workspace:

- `git2` → `libgit2-sys`
- `rusqlite` → `libsqlite3-sys`
- `lancedb` may transitively pull in additional `-sys` crates through `arrow-*`

### How to handle it

When a release PR includes a dependency update, check the CHANGELOG. If any of the above are involved:

1. Push to the release PR branch and change `workspace.package.version` in `Cargo.toml` (and the matching internal dependency versions) from patch to minor.
2. Update the version heading (and the compare URL) in `CHANGELOG.md` (and any per-crate `CHANGELOG.md`) to match.
3. Add a one-line note to the CHANGELOG entry explaining why (e.g. "pulls in a new major of `libgit2-sys`").

## Required secrets

- `RELEASE_PLZ_TOKEN`: fine-grained PAT with Contents + Pull requests read/write.
- `CARGO_REGISTRY_TOKEN`: used for crates.io publishing.
