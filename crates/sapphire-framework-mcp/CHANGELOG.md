# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.22.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-mcp-v0.21.0...sapphire-framework-mcp-v0.22.0) - 2026-10-25

### Added

- *(mcp)* answer subscriptions/listen in the framework's handler ([#270](https://github.com/sapphire-turtle/sapphire-framework/pull/270))
- *(mcp)* Subscriptions, what a handler offers on subscriptions/listen ([#270](https://github.com/sapphire-turtle/sapphire-framework/pull/270))
- *(mcp)* announce changes through ToolContext ([#270](https://github.com/sapphire-turtle/sapphire-framework/pull/270))

### Fixed

- *(mcp)* server_info promises no listChanged to handlers that do not listen ([#270](https://github.com/sapphire-turtle/sapphire-framework/pull/270))

### Other

- *(mcp)* subscriptions/listen over HTTP ([#270](https://github.com/sapphire-turtle/sapphire-framework/pull/270))

## [0.18.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-mcp-v0.17.0...sapphire-framework-mcp-v0.18.0) - 2026-10-13

### Fixed

- *(mcp)* build against rmcp 2 (Content is ContentBlock)

### Other

- *(deps)* go straight to rmcp 3.5

## [0.17.0](https://github.com/sapphire-turtle/sapphire-framework/compare/sapphire-framework-mcp-v0.16.0...sapphire-framework-mcp-v0.17.0) - 2026-10-13

### Added

- *(mcp)* shared MCP tools over stdio and HTTP /mcp
