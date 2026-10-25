# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.22.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-bridge-v0.21.0...sapphire-framework-bridge-v0.22.0) - 2026-10-25

### Added

- *(bridge)* define gui.locale, the language of the apps' windows
- *(gui)* store the accent as a Tint: gui.theme.accent ([#327](https://github.com/sapphire-turtle/sapphire-framework/pull/327))
- *(bridge)* the theme's settings, workgroup-wide and overridable ([#327](https://github.com/sapphire-turtle/sapphire-framework/pull/327))
- *(settings)* a rejected batch names its keys on the wire ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge-api)* [**breaking**] drop StatusResult::config_dry_run; bridge-api goes to 3.0 ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge)* config.set and config.unset can check without writing ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(settings)* check-only batches, a setting's process, and dry_run on the wire ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge)* restart the bridge in place, from the CLI and over IPC ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge)* sapphire-bridge config get/set/unset/list ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge)* config.* methods to read and write settings by key ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge)* [**breaking**] migrate net.toml and embedding.toml into config.toml (format 2) ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge)* [**breaking**] read and write the bridge's settings in one config.toml per scope ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge)* describe the bridge's settings as a registry owner ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(embed, retrieve)* quantize the local model, embed images, ANN search
- *(session)* lock per step and give up on no progress, not total time
- host the bridge and an app server in-process

### Fixed

- keep setting values out of error messages ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge)* keep the listeners across an in-place restart ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge)* final review fixes for the settings registry ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(session)* never let a read loop wait on its own writer

### Other

- take the app directories and the settings screen from corundum-framework
- Merge remote-tracking branch 'origin/main' into feat/theme-sync
- *(bridge)* serve every workgroup session the fixture's peer receives ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- Merge origin/main into feat/settings-registry
- the bridge's config.toml and the config command ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
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
