# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.22.0](https://github.com/sapphire-turtle/sapphire-framework/compare/v0.21.0...v0.22.0) - 2026-10-25

### Added

- *(embed, retrieve)* quantize the local model, embed images, ANN search
- *(workspace)* read and write binary files; skip non-UTF-8 content when indexing

### Other

- take the app directories and the settings screen from corundum-framework

## [0.21.0](https://github.com/sapphire-turtle/sapphire-framework/compare/v0.20.0...v0.21.0) - 2026-10-16

### Added

- *(gui)* start a desktop app's window with `app::run` (eframe feature)

## [0.20.0](https://github.com/sapphire-turtle/sapphire-framework/compare/v0.19.0...v0.20.0) - 2026-10-14

### Other

- deprecate client-side workspace selection and the old registry

## [0.17.0](https://github.com/sapphire-turtle/sapphire-framework/compare/v0.16.0...v0.17.0) - 2026-10-13

### Added

- *(mcp)* shared MCP tools over stdio and HTTP /mcp

### Fixed

- *(logging)* print the console log to stderr, not stdout ([#232](https://github.com/sapphire-turtle/sapphire-framework/pull/232))

## [0.16.0](https://github.com/sapphire-turtle/sapphire-framework/compare/v0.15.0...v0.16.0) - 2026-10-12

### Other

- deprecate current-directory marker discovery

## [0.14.0](https://github.com/fluo10/sapphire-framework/compare/v0.13.0...v0.14.0) - 2026-09-08

### Fixed

- *(track)* store mtimes in nanoseconds and compare file size too ([#118](https://github.com/fluo10/sapphire-framework/pull/118))
