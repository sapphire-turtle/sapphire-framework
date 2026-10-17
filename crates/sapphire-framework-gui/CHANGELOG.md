# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.21.1](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-gui-v0.21.0...sapphire-framework-gui-v0.21.1) - 2026-10-17

### Added

- *(gui)* host an app on Android
- *(gui)* a read-only file browser with text preview
- host the bridge and an app server in-process
- *(gui)* let apps put action buttons on the Shell's rail

### Fixed

- *(gui)* one start per process, no JNI local leaks, no stale refocus text
- *(gui)* typing, the system bars and the activity on Android
- *(gui)* file browser icons from egui's icon font

### Other

- Merge pull request #288 from sapphire-turtle/feat/mobile-android
- *(deps)* Bump fontdb from 0.23.0 to 0.24.0

## [0.21.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-gui-v0.20.0...sapphire-framework-gui-v0.21.0) - 2026-10-16

### Added

- *(gui)* a window frame for every app — Shell, Settings and Health
- *(gui)* start a desktop app's window with `app::run` (eframe feature)

## [0.20.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-gui-v0.19.0...sapphire-framework-gui-v0.20.0) - 2026-10-14

### Added

- *(server)* look for newer releases and show them in status and the GUI

### Fixed

- map a workgroup workspace without writing config.toml first

### Other

- deprecate client-side workspace selection and the old registry

## [0.16.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-gui-v0.15.0...sapphire-framework-gui-v0.16.0) - 2026-10-12

### Added

- move or remove the current workspace from the GUI and CLI

## [0.14.0](https://github.com/fluo10/sapphire-framework/compare/sapphire-framework-gui-v0.13.0...sapphire-framework-gui-v0.14.0) - 2026-09-08

### Added

- *(gui)* shared sapphire-framework-gui crate — WorkspaceManager ([#86](https://github.com/fluo10/sapphire-framework/pull/86))

### Other

- *(release)* resume publishing at 0.13.0, with every crate in the framework group ([#123](https://github.com/fluo10/sapphire-framework/pull/123))
- *(deps)* Bump rfd from 0.15.4 to 0.17.2 ([#120](https://github.com/fluo10/sapphire-framework/pull/120))
- *(deps)* Update egui requirement from 0.34 to 0.36
- *(deps)* land the actionable dependabot updates, and get CI green first ([#107](https://github.com/fluo10/sapphire-framework/pull/107))
