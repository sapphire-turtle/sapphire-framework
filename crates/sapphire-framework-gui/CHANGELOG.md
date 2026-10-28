# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.23.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-gui-v0.22.0...sapphire-framework-gui-v0.23.0) - 2026-10-28

### Added

- *(sapphire-framework-gui)* [**breaking**] the side pane in the shell, ShellApp, and corundum's pickers

## [0.22.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-gui-v0.21.0...sapphire-framework-gui-v0.22.0) - 2026-10-25

### Added

- *(gui)* keep the theme and the language in effect on the device
- *(sapphire-framework-gui)* [**breaking**] follow and edit the language as the bridge's gui.locale
- *(sapphire-framework-gui)* translate the appearance and settings pages
- *(sapphire-framework-gui)* re-export the tr! macro
- *(sapphire-framework-gui)* choose the language on the Appearance page
- *(sapphire-framework-gui)* [**breaking**] every screen string is translated
- *(sapphire-framework-gui)* [**breaking**] translations; the view models return messages
- *(gui)* store the accent as a Tint: gui.theme.accent ([#327](https://github.com/sapphire-turtle/sapphire-framework/pull/327))
- *(corundum-gui)* [**breaking**] shell labels are message ids, translated when drawn
- *(gui)* [**breaking**] the framework saves the theme; deprecate the app-side API ([#327](https://github.com/sapphire-turtle/sapphire-framework/pull/327))
- *(gui)* the Appearance page writes the theme per scope ([#327](https://github.com/sapphire-turtle/sapphire-framework/pull/327))
- *(gui)* install the theme the settings hold, and follow it ([#327](https://github.com/sapphire-turtle/sapphire-framework/pull/327))
- *(gui)* the theme's settings as ThemeSettings and back ([#327](https://github.com/sapphire-turtle/sapphire-framework/pull/327))
- *(gui)* the client's poll reads the theme's settings ([#327](https://github.com/sapphire-turtle/sapphire-framework/pull/327))
- *(gui)* [**breaking**] settings pages from the schema; Embedding edits through its section ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(gui)* settings widgets, pages and the restart banner ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(gui)* the model behind the settings pages ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(gui)* load, write and restart settings through the client ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(sapphire-framework-gui)* Appearance page for the theme
- *(gui)* [**breaking**] reach the bridge through the app server ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(embed, retrieve)* quantize the local model, embed images, ANN search
- *(gui)* host an app on Android
- *(gui)* a read-only file browser with text preview
- host the bridge and an app server in-process
- *(gui)* let apps put action buttons on the Shell's rail

### Fixed

- *(sapphire-framework-gui)* unset only the device's own appearance keys, merge same-frame writes
- *(sapphire-framework-gui)* call the Configuration page 詳細設定 in Japanese
- Japanese wording (full-width parentheses, About, Set, show folder)
- *(gui)* keep a written theme on the page until the reload; read the rest when one key is unreadable ([#327](https://github.com/sapphire-turtle/sapphire-framework/pull/327))
- *(gui)* clear a written secret's undo state at once; keep a drafted unset ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(gui)* keep an unset Int's text field while it is typed into ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(gui)* forget a typed secret's undo history, and tie errors to their place ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(gui)* load the settings concurrently, and isolate the settings tests ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(gui)* keep the renderer's start-up line in a phone's log
- *(gui)* keep the app connection when only the bridge refused ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(gui)* one start per process, no JNI local leaks, no stale refocus text
- *(gui)* typing, the system bars and the activity on Android
- *(gui)* file browser icons from egui's icon font

### Other

- take the app directories and the settings screen from corundum-framework
- move corundum-gui to sapphire-turtle/corundum-framework
- Merge remote-tracking branch 'origin/main' into feat/corundum-i18n
- Merge remote-tracking branch 'origin/main' into feat/theme-sync
- Merge remote-tracking branch 'origin/main' into feat/gui-settings
- Merge origin/main into feat/gui-settings
- Merge remote-tracking branch 'origin/main' into feat/theme-sync
- Merge remote-tracking branch 'origin/main' into feat/corundum-android
- *(gui)* run the phone's window through corundum-gui ([#326](https://github.com/sapphire-turtle/sapphire-framework/pull/326))
- *(sapphire-framework-gui)* take status colours from the theme
- *(gui)* open the window through corundum-gui
- Merge remote-tracking branch 'origin/main' into feat/corundum-gui
- *(gui)* pin the same-frame About and the narrow footer click; run mobile CI on dependency bumps ([#318](https://github.com/sapphire-turtle/sapphire-framework/pull/318))
- *(gui)* build Shell, fonts and icons on corundum-gui ([#318](https://github.com/sapphire-turtle/sapphire-framework/pull/318))
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
