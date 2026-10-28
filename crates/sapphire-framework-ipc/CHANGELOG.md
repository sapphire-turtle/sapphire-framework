# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.23.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-ipc-v0.22.0...sapphire-framework-ipc-v0.23.0) - 2026-10-28

### Other

- update Cargo.toml dependencies

## [0.22.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-ipc-v0.21.0...sapphire-framework-ipc-v0.22.0) - 2026-10-25

### Added

- *(ipc)* an in-process carrier, chosen per process

### Fixed

- *(ipc)* a cancelled accept on Windows keeps its waiting pipe instance ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))

### Other

- take the app directories and the settings screen from corundum-framework
- Merge pull request #288 from sapphire-turtle/feat/mobile-android
- *(deps)* Bump windows-sys from 0.59.0 to 0.61.2

## [0.20.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-ipc-v0.19.0...sapphire-framework-ipc-v0.20.0) - 2026-10-14

### Other

- *(ipc)* retry the final reachability probe in probe_race

## [0.19.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-ipc-v0.18.0...sapphire-framework-ipc-v0.19.0) - 2026-10-14

### Added

- *(server)* reconnect to sapphire-bridge on its own ([#244](https://github.com/sapphire-turtle/sapphire-framework/pull/244))

### Fixed

- *(ipc)* probe never removes a socket file; bind clears a dead server's
- *(ipc,server)* a probe during startup no longer strands a server; stop background sync work on shutdown ([#235](https://github.com/sapphire-turtle/sapphire-framework/pull/235))

## [0.17.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-ipc-v0.16.0...sapphire-framework-ipc-v0.17.0) - 2026-10-13

### Added

- *(mcp)* shared MCP tools over stdio and HTTP /mcp

## [0.16.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-ipc-v0.15.0...sapphire-framework-ipc-v0.16.0) - 2026-10-12

### Other

- update Cargo.toml dependencies
