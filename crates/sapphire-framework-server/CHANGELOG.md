# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.20.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-server-v0.19.0...sapphire-framework-server-v0.20.0) - 2026-10-14

### Added

- [**breaking**] print the help on a bare invocation instead of starting the server
- *(server)* let an application read the runtime sync_via_bridge mounts ([#250](https://github.com/sapphire-turtle/sapphire-framework/pull/250))
- *(server)* an app's update also updates sapphire-bridge
- verified self-update for app CLIs and sapphire-bridge
- *(server)* look for newer releases and show them in status and the GUI

### Fixed

- map a workgroup workspace without writing config.toml first
- *(server)* pick the newest release by tag prefix, not releases/latest

### Other

- Merge pull request #266 from sapphire-turtle/refactor/single-workspace-cleanup
- deprecate client-side workspace selection and the old registry
- Merge remote-tracking branch 'origin/main' into feat/bare-invocation-help
- Merge pull request #254 from sapphire-turtle/fix/workspace-init-sync
- *(update)* drop minisign; trust the GitHub release over HTTPS

## [0.19.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-server-v0.18.0...sapphire-framework-server-v0.19.0) - 2026-10-14

### Added

- *(server)* reconnect to sapphire-bridge on its own ([#244](https://github.com/sapphire-turtle/sapphire-framework/pull/244))

### Fixed

- *(server,session)* push what a new live session's peer missed while it started
- *(server)* wait for the aborted sync loops before shutdown returns
- *(ipc,server)* a probe during startup no longer strands a server; stop background sync work on shutdown ([#235](https://github.com/sapphire-turtle/sapphire-framework/pull/235))

## [0.18.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-server-v0.17.0...sapphire-framework-server-v0.18.0) - 2026-10-13

### Added

- *(cli)* `workspace select --sync` and `workspace sync enable|disable|status`

## [0.17.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-server-v0.16.0...sapphire-framework-server-v0.17.0) - 2026-10-13

### Added

- *(mcp)* shared MCP tools over stdio and HTTP /mcp

## [0.16.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-server-v0.15.0...sapphire-framework-server-v0.16.0) - 2026-10-12

### Added

- move or remove the current workspace from the GUI and CLI

### Fixed

- *(server)* end background dials with the server
- *(server)* answer sync.enable without waiting on offline peers
- *(server)* match watcher events under a root's resolved path too

### Other

- Merge pull request #230 from sapphire-turtle/fix/sync-enable-offline-peer
- Merge pull request #226 from sapphire-turtle/ci/test-release-platforms
- *(server)* delete the host tests' scratch tree under the env lock
