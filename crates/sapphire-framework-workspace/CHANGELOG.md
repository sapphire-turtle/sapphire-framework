# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.15.0](https://github.com/sapphire-turtle/sapphire-framework/compare/v0.14.0...v0.15.0) - 2026-10-11

### Added

- *(server)* the primary device backfills vectors, gently, and reports progress ([#188](https://github.com/sapphire-turtle/sapphire-framework/pull/188))
- *(server)* embed in the background, once per file across the workgroup ([#187](https://github.com/sapphire-turtle/sapphire-framework/pull/187))
- *(retrieve,workspace)* vector files per content and profile, read before embedding ([#187](https://github.com/sapphire-turtle/sapphire-framework/pull/187))
- *(sync,bridge-api)* a model's full identity, and no conflict copies of synced vectors ([#187](https://github.com/sapphire-turtle/sapphire-framework/pull/187))
- *(workspace)* follow the bridge's embedding model without a restart ([#186](https://github.com/sapphire-turtle/sapphire-framework/pull/186))
- *(bridge-api)* embedding settings types, methods and CLI pieces — 2.3.0 ([#186](https://github.com/sapphire-turtle/sapphire-framework/pull/186))
- *(workspace)* [**breaking**] embed through the bridge; apps no longer configure embedding
- *(retrieve)* [**breaking**] embedding moves to the bridge — trait only, configure_vectors, stop on a provider outage
- *(workspace)* [**breaking**] index every file whole; protocol 3 for snippet results
- *(workspace)* shared app logging — subscriber at init, rotating app.log per app
- *(backend)* define the workspace.* wire types
- *(registry)* [**breaking**] drop users; every device belongs to one person
- *(workspace)* WorkspaceArgs + clap/serde/dirs re-exports; facade passthrough ([#128](https://github.com/sapphire-turtle/sapphire-framework/pull/128))
- *(workspace)* AppContext::init — per-kind layout, migration, unified env resolution ([#129](https://github.com/sapphire-turtle/sapphire-framework/pull/129))
- *(workspace)* app_dirs module — per-kind layout, migration, env-name rule (#129, #128)

### Fixed

- *(workspace)* clear the track store in place instead of deleting its file ([#195](https://github.com/sapphire-turtle/sapphire-framework/pull/195))
- *(bridge,workspace)* status reports embedding; outage is audible; bound the embed.info probe
- *(workspace)* re-index after the retrieve store is reset
- *(workspace)* final-review findings — AppKind prelude export, init panic messages, keys.toml doc, test hygiene
- *(workspace)* app_dirs move fallback — remove_file for file sources, else-if collapse, error context
- *(workspace)* app_dirs review fixes — cross-device move fallback, once-per-uuid keys guard, AppKind::as_str by value

### Other

- release 0.15.0
- version the public APIs by their own crates ([#218](https://github.com/sapphire-turtle/sapphire-framework/pull/218))
- Merge branch 'feat/p2p-sync-iroh' into feat/bridge-embed
- Merge pull request #196 from sapphire-turtle/feat/remove-chunker
- one document per file in retrieve; breaking notes
- cargo fmt
- *(track,workspace)* compare paths against the same canonicalization the code under test applies (issue #162)
- *(workspace)* [**breaking**] undo the per-kind directory split

## [0.14.0](https://github.com/fluo10/sapphire-framework/compare/v0.13.0...v0.14.0) - 2026-09-08

### Fixed

- *(track)* store mtimes in nanoseconds and compare file size too ([#118](https://github.com/fluo10/sapphire-framework/pull/118))
