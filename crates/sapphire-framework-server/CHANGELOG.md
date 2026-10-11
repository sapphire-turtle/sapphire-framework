# Changelog

All notable changes to `sapphire-workspace` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [0.15.0](https://github.com/sapphire-turtle/sapphire-framework/releases/tag/sapphire-framework-server-v0.15.0) - 2026-10-11

### Added

- one workspace per app server ([#215](https://github.com/sapphire-turtle/sapphire-framework/pull/215))
- *(bridge)* measure availability and rank equal-priority candidates by it ([#190](https://github.com/sapphire-turtle/sapphire-framework/pull/190))
- external devices — keys checked through the bridge, managed in the CLIs and the sync panel; KeyStore removed ([#199](https://github.com/sapphire-turtle/sapphire-framework/pull/199))
- *(server)* the primary device backfills vectors, gently, and reports progress ([#188](https://github.com/sapphire-turtle/sapphire-framework/pull/188))
- *(server)* embed in the background, once per file across the workgroup ([#187](https://github.com/sapphire-turtle/sapphire-framework/pull/187))
- *(bridge)* embedding settings — workgroup models, device switches, key; applied in place ([#186](https://github.com/sapphire-turtle/sapphire-framework/pull/186))
- *(bridge-api)* 2.2.0 — embed.info and embed.embed
- *(cli)* device priority, and roles in device list
- *(server)* sync through the designated and backup devices when elected
- *(workspace)* shared app logging — subscriber at init, rotating app.log per app
- *(bridge-api)* 2.1.0 — device priority, elected roles in peers
- *(server)* workspace list shows this host's workspaces from the server
- *(server)* workspace.list and workspace.forget; serve restores synced workspaces
- *(server)* a host workspace registry in the app's config directory
- *(backend)* [**breaking**] API 2 — workspace.list/forget types; StatusReport moves to the protocol
- *(server)* workgroup create and device retire go through the running bridge
- *(bridge-api)* [**breaking**] version 2.0.0 is API_VERSION 2; add workgroup_create and device_retire
- *(server)* workspace commands on the app server; workgroup/device on the bridge
- *(server)* a typed status report shared by CLI and IPC
- *(server)* stop on SIGTERM/SIGINT; remove start-on-demand
- *(server)* provide the flat FrameworkCommand for app CLIs
- *(service)* add service install to the app servers and the bridge
- *(service)* decide the scope and the target user
- *(bridge)* optionally run a relay for a workgroup
- *(bridge)* merge relay configuration from the host and its workgroup
- *(server)* propagate commits live, and through a host in the middle
- *(bridge)* add device invite, workgroup join and sync.map
- *(bridge)* [**breaking**] join a workgroup, and find this host's record by node id
- *(server)* scan a workspace when something changes it from outside
- *(server)* expose sync.enable, sync.disable and sync.status
- *(server)* hold a replica per synced workspace and answer the bridge
- *(server)* give each workspace an identity that survives across devices
- *(server)* refuse start-on-demand for a privilege-separated app
- *(server)* run the privilege separation sequence
- *(server)* fork the lower-privileged helper over a socketpair
- *(server)* drop privileges permanently and verify the drop
- *(server)* resolve the two users privilege separation needs
- *(server)* add the server run/status/stop subcommands
- *(server)* listen, shut down on request, and exit when idle
- *(server)* push workspace events to subscribed clients
- *(server)* serve the workspace.* namespace over IPC
- *(server)* keep one backend per workspace root, closing cold ones

### Fixed

- keep workspace dev-dependencies path-only, so publishing strips them
- *(server)* keep the sessions a star would close until a hub answers ([#192](https://github.com/sapphire-turtle/sapphire-framework/pull/192))
- *(server)* connect to the bridge before asking for an API key ([#186](https://github.com/sapphire-turtle/sapphire-framework/pull/186))
- *(server)* a star needs a designated device; read the cached device id only
- *(server)* restore re-checks each row; registry docs say what is canonical
- *(server)* forget tears down sync for a workspace whose root is gone
- *(sync)* ignore Access events in the watchers, and bound the debounce
- *(ipc)* gate the handshake on the API version, not the crate version
- *(sync)* a watcher scan must not re-record content the store already knows
- *(server)* reap per-connection serve tasks and live sessions on shutdown
- *(server,ipc)* portable SIGTERM select arm, test-server teardown, drop dead lock_path
- *(server)* close inherited fds in the helper child before exec

### Other

- release 0.15.0
- version the public APIs by their own crates ([#218](https://github.com/sapphire-turtle/sapphire-framework/pull/218))
- Merge branch 'feat/p2p-sync-iroh' into feat/single-workspace-server
- Merge pull request #213 from sapphire-turtle/feat/issue-190-availability
- Merge pull request #211 from sapphire-turtle/feat/issue-199-external-devices
- rename designated / backup devices to primary / secondary ([#198](https://github.com/sapphire-turtle/sapphire-framework/pull/198))
- Merge branch 'feat/p2p-sync-iroh' into feat/bridge-embed
- Merge branch 'feat/p2p-sync-iroh' into feat/designated-device
- Merge branch 'feat/p2p-sync-iroh' into feat/workspace-logging
- describe when the star forms, ties and non-preemption
- cargo fmt
- *(server)* star tests prove the hub path, the pair election and no preemption strictly
- *(server)* star topology end to end — relay, takeover, no preemption, mesh fallbacks
- *(server)* replace source-grep test with integration tests for CLI bridge commands
- handshake with the bridge at API_VERSION
- *(sync)* cover the live-session re-record recovery
- *(service)* one per-user unit kind — scope, run-as and privilege machinery removed (issue #145)
- *(server,bridge)* drop privilege separation from the app server and the bridge CLI (issue #145)
- *(server)* drop the live-connection counter whose reader the idle exit took with it
- *(server)* make the live-loop test wait for quiet instead of a fixed window
- *(bridge-api)* pin the invite and workspaces wire shapes
- *(server)* two hosts converge end to end
- *(server)* pin that a later registration never clears the app's routes
- *(server)* prove privilege separation works, as root, in CI
- *(server)* prove concurrent clients no longer contend for the cache
- *(server)* fix a broken intra-doc link in subscribe_method
