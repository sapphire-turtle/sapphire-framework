# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.28.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-v0.27.1...sapphire-framework-v0.28.0) - 2026-11-06

### Other

- update Cargo.toml dependencies

## [0.27.1](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-v0.27.0...sapphire-framework-v0.27.1) - 2026-11-04

### Fixed

- *(deps)* corundum 0.9.1, so a Windows update installs and restarts the app ([#410](https://github.com/sapphire-turtle/sapphire-framework/pull/410))

## [0.27.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-v0.26.1...sapphire-framework-v0.27.0) - 2026-11-04

### Changed

- *(deps)* [**breaking**] corundum 0.9, whose Windows service needs no administrator ([#408](https://github.com/sapphire-turtle/sapphire-framework/pull/408))

## [0.26.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-v0.25.0...sapphire-framework-v0.26.0) - 2026-11-03

### Added

- [**breaking**] corundum 0.8, the desktop packages on cargo-packager ([#403](https://github.com/sapphire-turtle/sapphire-framework/pull/403))

## [0.25.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-v0.24.1...sapphire-framework-v0.25.0) - 2026-11-02

### Added

- *(gui)* gui::package, a desktop app as a Velopack package

## [0.22.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-v0.21.0...sapphire-framework-v0.22.0) - 2026-10-25

### Added

- *(server)* [**breaking**] <app> config, and dispatch takes the app's settings and build function ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- let applications add metrics of their own
- serve Prometheus metrics at /metrics when server.toml turns them on
- *(gui)* host an app on Android

### Other

- say /metrics is the one route without a token

## [0.21.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-v0.20.0...sapphire-framework-v0.21.0) - 2026-10-16

### Added

- *(gui)* start a desktop app's window with `app::run` (eframe feature)

## [0.20.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-v0.19.0...sapphire-framework-v0.20.0) - 2026-10-14

### Added

- verified self-update for app CLIs and sapphire-bridge
- *(server)* look for newer releases and show them in status and the GUI

### Other

- deprecate client-side workspace selection and the old registry

## [0.18.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-v0.17.0...sapphire-framework-v0.18.0) - 2026-10-13

### Added

- *(frontmatter)* YAML front matter for Markdown files ([#237](https://github.com/sapphire-turtle/sapphire-framework/pull/237))

## [0.17.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-v0.16.0...sapphire-framework-v0.17.0) - 2026-10-13

### Added

- *(mcp)* shared MCP tools over stdio and HTTP /mcp

## [0.14.0](https://github.com/fluo10/sapphire-framework/compare/sapphire-framework-v0.13.0...sapphire-framework-v0.14.0) - 2026-09-08

### Added

- *(registry)* device table, whose ids get written into content
- *(registry)* user table with the key file's own discipline
- *(remote-server)* [**breaking**] identify API keys by UUID again
- *(remote-server)* [**breaking**] identify API keys by grain-id instead of UUID
- *(gui)* shared sapphire-framework-gui crate — WorkspaceManager ([#86](https://github.com/fluo10/sapphire-framework/pull/86))
- *(backend)* shared workspace registry + selection resolver
- add sapphire-framework facade crate skeleton ([#95](https://github.com/fluo10/sapphire-framework/pull/95))

### Fixed

- *(registry,remote-server)* address code-review findings A-J
- *(remote-server)* the minors from the whole-branch review

### Other

- *(release)* resume publishing at 0.13.0, with every crate in the framework group ([#123](https://github.com/fluo10/sapphire-framework/pull/123))
- Merge pull request #115 from fluo10/feat/api-key-rotation
