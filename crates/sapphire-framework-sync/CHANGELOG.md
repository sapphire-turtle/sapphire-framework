# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.22.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-sync-v0.21.0...sapphire-framework-sync-v0.22.0) - 2026-10-21

### Added

- *(sync)* remove staged content nothing needs any more ([#298](https://github.com/sapphire-turtle/sapphire-framework/pull/298))
- *(sync)* list the content a replica wants written and does not have ([#297](https://github.com/sapphire-turtle/sapphire-framework/pull/297))
- *(sync)* [**breaking**] locate content by path; 1 GiB default limit
- *(sync)* stream-hash content into a verified staging writer

### Fixed

- *(sync)* keep content already staged when replacing it fails ([#300](https://github.com/sapphire-turtle/sapphire-framework/pull/300))
- *(sync)* make staged content and placed files durable before they count ([#299](https://github.com/sapphire-turtle/sapphire-framework/pull/299))
- *(session)* batch content lookups and bound the final writer wait
- *(sync)* give every staging writer a partial file of its own
- *(sync)* copy an on-disk conflict loser regardless of the size limit

### Other

- *(sync)* commit path states once per page instead of once per path

## [0.16.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-sync-v0.15.0...sapphire-framework-sync-v0.16.0) - 2026-10-12

### Added

- move or remove the current workspace from the GUI and CLI
