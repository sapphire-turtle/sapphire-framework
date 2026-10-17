# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.21.1](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-bridge-v0.21.0...sapphire-framework-bridge-v0.21.1) - 2026-10-17

### Added

- host the bridge and an app server in-process

### Other

- Merge pull request #288 from sapphire-turtle/feat/mobile-android
- *(deps)* Bump windows-sys from 0.59.0 to 0.61.2

## [0.21.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-bridge-v0.20.0...sapphire-framework-bridge-v0.21.0) - 2026-10-16

### Fixed

- *(bridge)* bound the data-plane header read and time it out

## [0.20.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-bridge-v0.19.0...sapphire-framework-bridge-v0.20.0) - 2026-10-14

### Added

- [**breaking**] print the help on a bare invocation instead of starting the server
- verified self-update for app CLIs and sapphire-bridge

### Other

- *(update)* drop minisign; trust the GitHub release over HTTPS

## [0.19.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-bridge-v0.18.0...sapphire-framework-bridge-v0.19.0) - 2026-10-14

### Added

- *(server)* reconnect to sapphire-bridge on its own ([#244](https://github.com/sapphire-turtle/sapphire-framework/pull/244))

### Fixed

- *(ipc,server)* a probe during startup no longer strands a server; stop background sync work on shutdown ([#235](https://github.com/sapphire-turtle/sapphire-framework/pull/235))

## [0.17.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-bridge-v0.16.0...sapphire-framework-bridge-v0.17.0) - 2026-10-13

### Fixed

- *(logging)* print the console log to stderr, not stdout ([#232](https://github.com/sapphire-turtle/sapphire-framework/pull/232))

## [0.16.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-bridge-v0.15.0...sapphire-framework-bridge-v0.16.0) - 2026-10-12

### Other

- update Cargo.toml dependencies
