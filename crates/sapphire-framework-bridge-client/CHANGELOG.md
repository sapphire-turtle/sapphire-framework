# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.22.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-bridge-client-v0.21.0...sapphire-framework-bridge-client-v0.22.0) - 2026-10-25

### Added

- *(settings)* a rejected batch names its keys on the wire ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge-api)* [**breaking**] drop StatusResult::config_dry_run; bridge-api goes to 3.0 ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge)* config.set and config.unset can check without writing ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(settings)* check-only batches, a setting's process, and dry_run on the wire ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(server)* relay the bridge's client-facing methods ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge-api)* the relay's not-running code and a relayed BridgeClient ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(bridge)* config.* methods to read and write settings by key ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))
- *(embed, retrieve)* quantize the local model, embed images, ANN search

### Fixed

- *(bridge-client)* a relayed client starts no announcement forwarder ([#314](https://github.com/sapphire-turtle/sapphire-framework/pull/314))

### Other

- Merge origin/main into feat/settings-registry

## [0.19.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-bridge-client-v0.18.0...sapphire-framework-bridge-client-v0.19.0) - 2026-10-14

### Added

- *(server)* reconnect to sapphire-bridge on its own ([#244](https://github.com/sapphire-turtle/sapphire-framework/pull/244))
