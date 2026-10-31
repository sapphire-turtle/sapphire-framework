# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.24.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-bridge-v0.23.0...sapphire-bridge-v0.24.0) - 2026-10-31

### Other

- call the shared workflows from sapphire-turtle/.github@v1

## [0.23.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-bridge-v0.22.0...sapphire-bridge-v0.23.0) - 2026-10-28

### Other

- set the brand colour of the README icon to #4F7FD9
- *(bridge)* the filled app icon
- *(bridge)* add sapphire-bridge's icon and show it in its README

## [0.22.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-bridge-v0.21.0...sapphire-bridge-v0.22.0) - 2026-10-25

### Added

- *(embed, retrieve)* quantize the local model, embed images, ANN search
- *(update)* recognise cargo binstall, and let binstall install the bridge

### Other

- take the app directories and the settings screen from corundum-framework
- Merge origin/main into feat/settings-registry
- the bridge's config.toml and the config command ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))

## [0.20.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-bridge-v0.19.0...sapphire-bridge-v0.20.0) - 2026-10-14

### Added

- *(install)* one install script for every app, with the bridge
- [**breaking**] print the help on a bare invocation instead of starting the server
- verified self-update for app CLIs and sapphire-bridge

### Other

- Merge pull request #259 from sapphire-turtle/feat/bare-invocation-help
- *(update)* drop minisign; trust the GitHub release over HTTPS

## [0.16.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-bridge-v0.15.0...sapphire-bridge-v0.16.0) - 2026-10-12

### Other

- move apps/sapphire-bridge to bridge/ at the repo root
