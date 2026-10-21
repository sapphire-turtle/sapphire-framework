# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.22.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-session-v0.21.0...sapphire-framework-session-v0.22.0) - 2026-10-21

### Added

- *(session)* lock per step and give up on no progress, not total time
- *(sync)* [**breaking**] locate content by path; 1 GiB default limit
- *(session)* [**breaking**] chunked content frames, two-queue writer, receive into staging

### Fixed

- *(session)* ask again for content earlier sessions never got ([#297](https://github.com/sapphire-turtle/sapphire-framework/pull/297))
- *(session)* a stream cut under the writer ends the session as gone
- *(session)* never send or ask for content over the receiver's limit
- *(session)* batch content lookups and bound the final writer wait
- *(session)* never let a read loop wait on its own writer

### Other

- Merge remote-tracking branch 'origin/feat/chunked-blob-transfer' into fix/rerequest-unavailable-content
- *(session)* rustfmt writer.rs

## [0.19.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-session-v0.18.0...sapphire-framework-session-v0.19.0) - 2026-10-14

### Fixed

- *(server,session)* push what a new live session's peer missed while it started
