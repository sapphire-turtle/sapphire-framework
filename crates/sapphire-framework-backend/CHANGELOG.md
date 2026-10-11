# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.15.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-backend-v0.14.0...sapphire-framework-backend-v0.15.0) - 2026-10-11

### Added

- one workspace per app server ([#215](https://github.com/sapphire-turtle/sapphire-framework/pull/215))
- *(server)* the primary device backfills vectors, gently, and reports progress ([#188](https://github.com/sapphire-turtle/sapphire-framework/pull/188))
- *(workspace)* [**breaking**] index every file whole; protocol 3 for snippet results
- *(server)* sync through the designated and backup devices when elected
- *(backend)* [**breaking**] API 2 — workspace.list/forget types; StatusReport moves to the protocol
- *(server)* workspace commands on the app server; workgroup/device on the bridge
- *(server)* stop on SIGTERM/SIGINT; remove start-on-demand
- *(bridge)* add device invite, workgroup join and sync.map
- *(server)* expose sync.enable, sync.disable and sync.status
- *(backend)* add IpcBackend so a UI can leave the cache to the server
- *(backend)* define the workspace.* wire types

### Fixed

- keep workspace dev-dependencies path-only, so publishing strips them
- *(ipc)* gate the handshake on the API version, not the crate version
- *(backend)* log a warning when the event pump drops a malformed notification

### Other

- release 0.15.0
- version the public APIs by their own crates ([#218](https://github.com/sapphire-turtle/sapphire-framework/pull/218))
- rename designated / backup devices to primary / secondary ([#198](https://github.com/sapphire-turtle/sapphire-framework/pull/198))
- Merge pull request #196 from sapphire-turtle/feat/remove-chunker
- *(ipc)* Error::Spawn is Error::NotRunning — nothing starts servers any more
- [**breaking**] remove the HTTP sync stack
