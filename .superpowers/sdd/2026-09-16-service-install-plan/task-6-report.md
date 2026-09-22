# Task 6 Report — Wiring service install into the apps

**Status:** DONE (with one noted pre-existing flake, unrelated to this task)

**Commit:** `56029b5` — `feat(service): add service install to the app servers and the bridge`
(branch `feat/p2p-sync-iroh`, on top of `b636357`)

## What was implemented

**Server (`crates/sapphire-framework-server`):**
- `ServerCommand::Service(ServiceCommand)` variant + dispatch arm calling the shared
  `ServiceCommand::run(&spec, &Environment::detect(), &SystemManager)` runner from
  `sapphire-framework-service` (added in the previous session of this task); error surfaced
  through a new transparent `Error::Service(#[from] sapphire_framework_service::Error)`.
- `AppServer::privileges(PrivilegeConfig)` builder — stores the config only; applying it
  stays `privilege::apply`'s job in `main` (per plan step 5 deviation note).
- `AppServer::service_spec()` — `args = ["server", "run"]`, description
  `"<app> server <version>"`, `system_run_as: RunAs::InvokingUser`, carries `privileges`,
  `post_install: None`.

**Bridge (`crates/sapphire-framework-bridge`):**
- `BridgeCommand::Service(ServiceCommand)` variant + dispatch arm with the same shared
  runner; new transparent `Error::Service(#[from] …)` variant.
- `bridge_service_spec(version)` — `args = ["run"]`, `app_name: "sapphire-bridge"`,
  `system_run_as: RunAs::InvokingUser` (a root bridge would put the bridge directory under
  `/root`), `privileges: None` (the bridge owns no workspace).
- Re-exports `ServiceCommand` and `ServiceSpec` from the bridge lib so the binary needs no
  direct dependency on the service crate.
- Cargo.toml: `sapphire-framework-service` dependency added (Cargo.lock updated).

**Binary (`apps/sapphire-bridge`):** no direct service dependency; doc comment updated to
mention `service install`. All dispatch flows through `BridgeCommand::dispatch`.

**Docs:** `docs/ARCHITECTURE.md` crate table row added verbatim from the plan:
`| sapphire-framework-service | OS のサービスマネージャへの登録（systemd user/system・LaunchAgent・タスクスケジューラ） | ✅ |`
(placed after `sapphire-framework-session`, before `apps/sapphire-bridge`).

## Tests (all pre-written as failing first, then made green)
- Server: `the_service_subcommands_parse`, `the_generated_spec_runs_the_server_not_the_cli`,
  `the_generated_spec_carries_the_apps_privileges`, `the_generated_spec_names_this_application`,
  `the_generated_spec_describes_the_server`,
  `the_generated_spec_runs_a_system_unit_as_the_invoking_user` (in `service_spec_tests`).
  Also fixed a duplicated-test-module mess left by the earlier double-write of `command.rs`
  (merged into one clean `service_spec_tests` module; parse test lives in `tests`).
- Bridge: `the_bridge_service_subcommands_parse`, `the_bridge_service_spec_runs_the_bridge`,
  `the_bridge_service_spec_names_the_bridge`.
- Service crate (previous session): runner tests — `running_an_install_command_activates_the_service`,
  `running_an_uninstall_command_removes_the_unit`, `running_a_status_command_reports_what_the_manager_said`,
  plus `Environment::detect` tests; all green.

## Verification commands + results
- `cargo test -p sapphire-framework-service --all-features` → 51 passed
- `cargo test -p sapphire-framework-server --all-features` → 93 lib + all integration suites passed
- `cargo test -p sapphire-framework-bridge --all-features` → 135 lib + all suites passed (incl. `wake`, twice)
- `cargo test -p sapphire-bridge --all-features --locked` → ok
- **Full suite:** `cargo test --all-features --locked --no-fail-fast` → **exit 0, 64 suites ok,
  752 passed, 0 failed**. One flaky occurrence of `converge`
  (`a_host_that_was_offline_catches_up_when_it_returns`, redb "Database already open") in an
  earlier run; verified **pre-existing** by stashing all changes and reproducing the flake on
  the clean tree (`b636357`, 2 of 6 runs failed) — known flake (server `converge` redb lock),
  unrelated to this task. The final full run was clean.
- `cargo fmt --all -- --check` → clean (after one `cargo fmt` pass)
- `cargo clippy --all-targets --all-features --locked -- -D warnings` → clean

## Files changed
`Cargo.lock`, `apps/sapphire-bridge/src/main.rs`,
`crates/sapphire-framework-bridge/{Cargo.toml,src/command.rs,src/error.rs,src/lib.rs}`,
`crates/sapphire-framework-server/src/{command.rs,error.rs,lib.rs}`,
`crates/sapphire-framework-service/src/{manager.rs,scope.rs}`, `docs/ARCHITECTURE.md`
(12 files, +443/−5).

## Concerns
- None blocking. The `converge` redb-lock flake is pre-existing (documented; reproduced on a
  clean tree) and out of this task's scope.
- TDD note: this session completed implementation + green runs; the failing-test (RED) phase
  for server/bridge happened in the earlier session per the plan's steps and the compressed
  history (tests existed and failed before the dispatch arms/service_spec were written).
