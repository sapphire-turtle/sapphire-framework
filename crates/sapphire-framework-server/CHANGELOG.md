# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


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
