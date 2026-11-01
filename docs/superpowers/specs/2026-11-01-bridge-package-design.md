# Sapphire Bridge as a Velopack package

Issue: #363 (part C of three), with #381 step 3 (the first-run service page and About's
service row) and #357 (distribution). Builds on part B
(`2026-10-31-bridge-desktop-design.md`: `sapphire-bridge-desktop`, its window) and part A
(#381: the service code is corundum-framework-service's). The runtime is corundum's
(`corundum-framework-package`, `gui::first_run`, `gui::update`; corundum's
`docs/superpowers/specs/2026-10-29-velopack-runtime-design.md` and
`2026-10-31-service-registration-design.md`).

## Why

Sapphire Bridge is the host's one sync daemon and now has a window, but it is installed by
script or `cargo binstall` and updated by `<app> update`. Someone who is not a developer
needs to install it like any app, have it keep running, and have it update itself. It is the
first sapphire program packaged with Velopack; the apps follow under #357.

Success means:

- an installer per desktop OS (Windows Setup, macOS `.pkg`, Linux AppImage) installs Sapphire
  Bridge, puts `sapphire-bridge` on `PATH`, and its first start asks to keep it running in
  the background;
- it finds, downloads and applies its own updates, with its service stopped before and
  started after;
- the apps neither replace a packaged bridge nor lose the bare-binary path for servers.

## Decisions

1. **One package with the desktop binary only.** Package id `sapphire-bridge`, title
   "Sapphire Bridge". It holds `sapphire-bridge-desktop`, which is a superset of the command
   line; corundum's `cli_path` puts it on `PATH` as `sapphire-bridge`. The headless
   `sapphire-bridge` binary stays as it is, for servers (install script, `cargo binstall`).
2. **Published with the framework's release.** `release-bridge.yml` packs after release-plz
   tags `v<version>` and uploads the Velopack files to that GitHub release, beside the bare
   binaries. The updater reads `sapphire-turtle/sapphire-framework`'s latest release.
3. **The first start asks to keep it running,** with corundum's first-run flow and
   `ServicePage`, Yes by default. No service is registered at install.
4. **On Windows, a GUI binary and a console shim.** With the feature, `sapphire-bridge-desktop`
   is built for the Windows GUI subsystem (no console window behind the window or the
   service), and a shim binary, `sapphire-bridge-console`, packed as `sapphire-bridge.exe`,
   is the command on `PATH`: it runs the desktop binary with its arguments and streams
   (corundum's `package::console::forward`), as corundum's demo does.
5. **Velopack is a build feature.** `bridge/desktop`'s `velopack` feature, on in the release
   workflow only, as corundum's demo does; `cargo build` makes the plain binary.

## The package (`bridge/desktop`, feature `velopack`)

```rust
// bridge/desktop/src/package.rs
pub static PACKAGE: PackageSpec = PackageSpec {
    app_name: "sapphire-bridge",
    on_installed: None,
    on_uninstalling: None,
    before_apply: None,
    cli: Some(CliSpec { command: "sapphire-bridge" }),
    services: Services::new(&[bridge_service]),
};

/// `bridge_service_spec` for this binary's version (Services takes `fn() -> ServiceSpec`).
fn bridge_service() -> ServiceSpec { bridge_service_spec(env!("CARGO_PKG_VERSION")) }

pub const REPO: &str = "https://github.com/sapphire-turtle/sapphire-framework";
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
```

`main` with the feature:

1. `package::start(&PACKAGE)` first: Velopack's install, update and uninstall hooks start
   the executable and expect it to exit at once (`Start::Done` → exit 0).
2. A subcommand (`serve`, `status`, …) runs as without the feature: no lock, no updater.
3. A bare start takes the single-instance lock (`acquire_or_activate_with`, as part B), then
   starts the updater: `Updater::new(&PACKAGE, Source::Github { repo: REPO, prerelease:
   false })`, `apply_pending_at_start()` (which restarts into the new version), and
   `run_in_background(CHECK_EVERY)`. Only the instance does this, so a second start never
   applies an update under the first.
4. Outside a package (`package::current()` is `None`) the updater is not made, and the
   window is part B's.

`run_bridge` gains a `BridgeWindowOptions::package: Option<PackageWindow>` that carries the
updater, the services and the first-run pages to the window, so the gui crate does not depend
on the desktop crate.

## First run (bridge window)

On the window's first start in a package (corundum's `store::Stored.first_run` unset), the
window shows `gui::first_run::FirstRun` before Settings:

1. A welcome page (the bridge's own `FirstRunPage`): "Sapphire Bridge syncs your apps'
   workspaces between your devices, and keeps the device's settings for them."
2. `ServicePage::new(vec![bridge_service], "Keep Sapphire Bridge running in the
   background?", "Sync and the apps need it while their windows are closed.")` — Yes by
   default.

Outside a package (and in a cargo build, which `ServicePage` already recognises) the flow is
skipped.

## About

In a package, About shows, below the version and the services' banner:

- a `settings::ServiceRow` for the bridge's service: its state, Stop / Start, Remove, Repair;
- an `update::UpdatesPanel` with `Target::this_app(updater)`: the latest release, Download,
  Restart to update.

## `update`

- `sapphire-bridge-desktop update` in a package: `--check` asks the package's updater; plain
  `update` downloads and applies it (the package restarts). This replaces part B's refusal
  there; outside a package the refusal stays.
- An app's `<app> update` updates the bridge by running `sapphire-bridge update` from
  `PATH` (`update_bridge`). For a packaged bridge that is the package's command, so the
  package's updater does it; nothing changes in the apps.

## Release workflow

`release-bridge.yml` gains a `pack` job after `build-desktop`, a matrix as corundum's
`demo-release.yml`:

| OS | runner | pack | extra |
|---|---|---|---|
| Linux | ubuntu-22.04 | AppImage | `--categories Network` |
| macOS | macos-latest | `.pkg` | `--bundleId org.sapphire-turtle.sapphire-bridge`, signed and notarised when the secrets are set, else a warning |
| Windows | windows-latest | Setup | — |

- It builds `sapphire-bridge-desktop --features velopack --release`, renders the icon from
  `bridge/headless/assets/app-icon.svg`, runs `vpk pack --packId sapphire-bridge
  --packVersion <version> --packTitle "Sapphire Bridge" --mainExe sapphire-bridge-desktop`,
  and uploads with `vpk upload github --releaseName v<version> --tag v<version> --merge`.
- `VPK_VERSION` is pinned as corundum's is.
- A smoke step runs the Linux AppImage with `--version`.

## Errors

- No release with Velopack files yet, or no network: the updater's row says so; nothing else
  changes.
- The service cannot be registered on the first-run page: `ServicePage`'s error and its
  "try again" / "continue without it".
- An update fails to apply: the package keeps the running version (Velopack's behaviour);
  the row shows the error.

## Testing

- Unit:
  - `PACKAGE` has id `sapphire-bridge`, command `sapphire-bridge`, one service whose args
    are `["serve"]`.
  - The first-run pages are the welcome page, then the service page.
  - `sapphire-bridge-desktop update` in a package reports and applies through the updater
    (`Updater::for_tests`).
  - `sapphire-bridge-desktop update` outside a package still refuses (part B's test).
- CI: `cargo build -p sapphire-bridge-desktop --features velopack` on each desktop OS; the
  `pack` job on a workflow_dispatch run; the AppImage `--version` smoke step.
- By hand after the first release, on each OS: install from the installer, the first-run
  page, the service running after a reboot, `sapphire-bridge` on `PATH`, an update from the
  previous version with the service stopped and started again, uninstall removing the
  service.

## Out of scope

- Packaging the apps (#357's children).
- A Windows code-signing certificate.
- Moving the headless binary into the package.
