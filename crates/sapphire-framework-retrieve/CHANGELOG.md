# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.15.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-retrieve-v0.14.0...sapphire-framework-retrieve-v0.15.0) - 2026-10-11

### Added

- *(server)* the primary device backfills vectors, gently, and reports progress ([#188](https://github.com/sapphire-turtle/sapphire-framework/pull/188))
- *(retrieve,workspace)* vector files per content and profile, read before embedding ([#187](https://github.com/sapphire-turtle/sapphire-framework/pull/187))
- *(retrieve)* [**breaking**] embedding moves to the bridge — trait only, configure_vectors, stop on a provider outage
- *(retrieve)* [**breaking**] one document per file — drop the chunker, results carry a snippet
- *(retrieve)* snippet helpers and a character cap for REST embedder input

### Fixed

- *(bridge,workspace)* status reports embedding; outage is audible; bound the embed.info probe
- *(retrieve)* a provider outage stops embed_pending but keeps the run's progress; drop wrong-size vectors
- *(retrieve)* CJK-safe REST cap, size-split requests, non-sticky embed failures
- *(retrieve)* drop an old-schema tantivy index when stamping an empty store

### Other

- *(retrieve)* name the REST request fn type for clippy
- *(retrieve)* FTS snippet for a Japanese match
- *(retrieve)* bound the FTS snippet source to 256 KiB
- drop remaining chunk wording; merge API_VERSION 3 changelog entry
- cargo fmt
- *(retrieve)* stop at the limit, extract pick_snippet, note the wipe policy
- *(architecture)* describe the framework as it now is
