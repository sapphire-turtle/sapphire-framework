# Sapphire Bridge package: by-hand release checks

CI builds and packs Sapphire Bridge (`.github/workflows/release-bridge.yml`, job `pack`) and
runs the Linux AppImage with `--version`. The rest needs a person at each desktop OS, after
the first release with the package and again whenever the package, its service or its updater
changes (spec: `docs/superpowers/specs/2026-11-01-bridge-package-design.md`).

On each of Windows, macOS and Linux:

1. **Install** from the release's installer (`sapphire-bridge-win-Setup.exe`,
   `sapphire-bridge-osx-Setup.pkg`, `sapphire-bridge.AppImage`).
2. **First run:** the window opens on the welcome page, then asks to keep Sapphire Bridge
   running in the background; Yes registers the service, and About › Service shows it
   running.
3. **After a reboot** the service is running (`sapphire-bridge status` answers) without
   opening the window.
4. **`sapphire-bridge` on `PATH`:** on macOS and Linux, About › Command line › Install
   first (Windows has it from the install). A new terminal runs `sapphire-bridge --version`
   and `sapphire-bridge status` (on Windows, with output in the terminal).
5. **Update** from the previous release: About › Updates finds the new version, downloads it
   and restarts into it; the service was stopped before and runs the new version after
   (`sapphire-bridge status`). Again from the command line with `sapphire-bridge update`:
   the service runs the new version afterwards without opening the window.
6. **Uninstall** removes the service (no unit, LaunchAgent or scheduled task left) and the
   command on `PATH`; the bridge's data directory stays.

If the release workflow's `pack` job failed, re-run it for the tag by hand (Actions ›
Release sapphire-bridge binaries › Run workflow, with the tag).
