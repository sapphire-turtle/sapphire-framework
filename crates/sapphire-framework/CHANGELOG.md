# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.15.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-v0.14.0...sapphire-framework-v0.15.0) - 2026-10-11

### Added

- one workspace per app server ([#215](https://github.com/sapphire-turtle/sapphire-framework/pull/215))
- external devices — keys checked through the bridge, managed in the CLIs and the sync panel; KeyStore removed ([#199](https://github.com/sapphire-turtle/sapphire-framework/pull/199))
- *(workspace)* [**breaking**] embed through the bridge; apps no longer configure embedding
- *(server)* provide the flat FrameworkCommand for app CLIs
- *(service)* decide the scope and the target user
- *(session)* frame replication messages and content
- *(bridge)* lay out the bridge directory and keep it to one process
- *(backend)* add IpcBackend so a UI can leave the cache to the server
- *(server)* keep one backend per workspace root, closing cold ones
- *(ipc)* add the crate scaffold, errors and the JSON-RPC envelope
- *(registry)* [**breaking**] drop users; every device belongs to one person
- *(registry)* [**breaking**] drop users; every device belongs to one person
- *(sync)* scaffold sapphire-framework-sync with value types
- *(workspace)* WorkspaceArgs + clap/serde/dirs re-exports; facade passthrough ([#128](https://github.com/sapphire-turtle/sapphire-framework/pull/128))

### Fixed

- *(workspace)* final-review findings — AppKind prelude export, init panic messages, keys.toml doc, test hygiene

### Other

- release 0.15.0
- Merge remote-tracking branch 'origin/main' into chore/merge-main
- *(bridge-embed)* ship the embedding component with the framework; facade feature bridge-embed
- *(plan)* finish the app command system sweep
- *(facade)* replace the remote features with ipc, server and bridge
- [**breaking**] remove the HTTP sync stack
- *(keys)* [**breaking**] lift KeyStore out of the remote server crate ([#103](https://github.com/sapphire-turtle/sapphire-framework/pull/103))
- *(server)* prove concurrent clients no longer contend for the cache
- *(registry)* record the per-device file format
- unify the tagline — human-agent collaboration, drop the fossil tagline ([#135](https://github.com/sapphire-turtle/sapphire-framework/pull/135))
- *(registry)* record the per-device file format

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
