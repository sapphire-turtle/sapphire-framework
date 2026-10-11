# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.15.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-gui-v0.14.0...sapphire-framework-gui-v0.15.0) - 2026-10-11

### Added

- one workspace per app server ([#215](https://github.com/sapphire-turtle/sapphire-framework/pull/215))
- *(bridge)* measure availability and rank equal-priority candidates by it ([#190](https://github.com/sapphire-turtle/sapphire-framework/pull/190))
- external devices — keys checked through the bridge, managed in the CLIs and the sync panel; KeyStore removed ([#199](https://github.com/sapphire-turtle/sapphire-framework/pull/199))
- *(server)* the primary device backfills vectors, gently, and reports progress ([#188](https://github.com/sapphire-turtle/sapphire-framework/pull/188))
- *(sync,bridge-api)* a model's full identity, and no conflict copies of synced vectors ([#187](https://github.com/sapphire-turtle/sapphire-framework/pull/187))
- *(gui)* an Embedding screen in the sync panel ([#186](https://github.com/sapphire-turtle/sapphire-framework/pull/186))
- *(bridge-api)* 2.2.0 — embed.info and embed.embed
- *(gui)* device priority editor, role badges, star in the workspace badge
- *(bridge-api)* 2.1.0 — device priority, elected roles in peers
- *(gui)* a Copy button for the command an install failure says to run
- *(gui)* find a CJK fallback among the system fonts instead of bundling one
- *(gui)* workgroup, device and workspace views, and SyncPanel
- *(gui)* pure view logic — badges, remote-only filter, device and name rules
- *(gui)* an egui-free client that snapshots the bridge and the app server

### Fixed

- *(gui)* keep a priority edit only while it differs from the bridge
- *(gui)* no second invite while one runs, and a late ticket is not lost
- *(gui)* give service install five minutes, not the 30 s command limit
- *(gui)* an install failure with nothing on stderr still says something
- *(gui)* show and open workspace paths without the verbatim prefix
- *(gui)* never turn the picked folder itself into a synced workspace
- *(gui,bridge)* join with the invited name, and keep the ticket on a typo
- *(gui)* prefer regular-weight CJK faces, add a merging API, honour the any-face fallback
- *(gui)* route device outcomes by origin, keep retire confirm open on failure, show busy spinners
- *(gui)* time-box every refresh and command so a silent process cannot freeze the client

### Other

- release 0.15.0
- version the public APIs by their own crates ([#218](https://github.com/sapphire-turtle/sapphire-framework/pull/218))
- Merge branch 'feat/p2p-sync-iroh' into feat/single-workspace-server
- Merge pull request #211 from sapphire-turtle/feat/issue-199-external-devices
- rename designated / backup devices to primary / secondary ([#198](https://github.com/sapphire-turtle/sapphire-framework/pull/198))
- Merge branch 'feat/p2p-sync-iroh' into feat/bridge-embed
- *(gui)* the hung listener needs mut only on Windows
- *(gui)* round trips for ServiceInstall and WorkgroupJoin
- [**breaking**] remove the HTTP sync stack

## [0.14.0](https://github.com/fluo10/sapphire-framework/compare/sapphire-framework-gui-v0.13.0...sapphire-framework-gui-v0.14.0) - 2026-09-08

### Added

- *(gui)* shared sapphire-framework-gui crate — WorkspaceManager ([#86](https://github.com/fluo10/sapphire-framework/pull/86))

### Other

- *(release)* resume publishing at 0.13.0, with every crate in the framework group ([#123](https://github.com/fluo10/sapphire-framework/pull/123))
- *(deps)* Bump rfd from 0.15.4 to 0.17.2 ([#120](https://github.com/fluo10/sapphire-framework/pull/120))
- *(deps)* Update egui requirement from 0.34 to 0.36
- *(deps)* land the actionable dependabot updates, and get CI green first ([#107](https://github.com/fluo10/sapphire-framework/pull/107))
