# The bridge's own window, and the apps that send you to it

Issue: #363 (part B of three). Related: #357 (distribution), #364 (API keys, done), #381
(service migration, part A). Part C, the bridge as a Velopack package, builds on this and on
A, and has its own spec.

## Why

`sapphire-bridge` is the host's one sync daemon: workgroup, devices, embedding. It has no
window. Its settings are reached from every app's GUI, through the app server's relay, and
from three command lines (the bridge's and each app's). Every app shows the same
workgroup, devices and embedding screens, so it is unclear where to change them, and an
app that lags the bridge shows the bridge's settings as an older bridge had them.

The bridge is about to be installed as an app of its own (#357). It should own its screens,
and the apps should send the user there.

Success means:

- `sapphire-bridge-desktop` opens a window with the bridge's status, workgroup, devices,
  embedding, appearance, logs and About; closing it leaves the bridge running;
- a desktop app's Settings shows a summary of the bridge and buttons that open the bridge's
  window on the right page;
- a phone app, whose bridge runs in its own process, keeps the screens as today;
- the apps' command lines no longer manage the workgroup, devices or embedding; the
  bridge's does.

## Decisions

1. **Two binaries, as the apps have.** `bridge/headless` (package `sapphire-bridge-headless`,
   binary `sapphire-bridge`) is today's command line, for servers. `bridge/desktop`
   (package `sapphire-bridge-desktop`, binary `sapphire-bridge-desktop`, not published to
   crates.io) is a superset: bare it opens the window, given a subcommand it runs the
   command line's own code, so the service manager can start it with `serve`.
2. **The resident bridge is still `serve`, a separate process.** The window is a client of
   it over IPC, as an app's window is of its server. Closing the window stops nothing.
3. **The window reaches the bridge directly.** `FrameworkClient` gains a bridge-only mode;
   the screens are the ones `sapphire-framework-gui` already has.
4. **Desktop apps send the user to the bridge; phones keep the screens.** The screens stay
   in `sapphire-framework-gui`. A desktop app's Settings drops the Devices, Workgroup and
   Embedding pages for a summary row with buttons; a phone app shows them as today.
5. **Apps open the bridge's window on a page.** The app runs
   `sapphire-bridge-desktop --page <page>`; a window already open is brought to the front
   and shown that page, through corundum's single-instance activation, which gains a
   message (corundum-framework 0.6). No URL scheme.
6. **The apps' `workgroup`, `device` and `embedding` subcommands are removed.** No alias:
   `sapphire-bridge` has all three. `config` stays, and still reaches the bridge's
   settings through the app server.
7. **The relay is kept whole.** A phone app's screens use it, and a desktop app's summary
   reads `bridge.status` and `bridge.peers` through it.

## corundum-framework 0.6: a message with the activation

Additive, not breaking.

- `platform::instance`:
  - `AlreadyRunning::activate_with(&self, message: &str) -> io::Result<()>` sends the
    message with the activation.
  - `Instance::on_activate_with(&mut self, f: impl Fn(Option<&str>) + Send + 'static)`
    receives it. `activate` sends none, and `on_activate`'s callback ignores it.
  - A window of an older version that receives a message comes to the front as before.
  - The message is opaque to corundum and at most 1 KiB.
- `gui::window`: `WindowOptions::on_activate(f: impl Fn(Option<String>) + Send + 'static)`
  hands it to the app, beside `single_instance`. corundum still brings the window to the
  front and repaints.
- `platform::instance::acquire_or_activate_with(app_name, message: Option<&str>)` is the
  start that opens a window and carries the message when it is second.
- The wire stays one line: `activate\n`, or `activate <message>\n`. A start whose message
  gets no answer (an older window, which does not know the form) asks again without it, so
  the older window still comes to the front.
- `corundum-framework-service`: `registered_program(spec, env, manager) ->
  Result<Option<PathBuf>>`, the program a registered service starts (what `state` already
  reads from the unit, the plist or the task), for an app to find another program's
  executable by its service.

Each lands in corundum-framework's next release; release-plz picks its number (additive:
0.5.x or 0.6.0), and this repository depends on that release.

## The bridge's binaries

```
bridge/
  headless/   sapphire-bridge-headless → bin sapphire-bridge     (was bridge/)
  desktop/    sapphire-bridge-desktop  → bin sapphire-bridge-desktop
```

- `headless` is today's `bridge/` moved: `src/main.rs`, `src/embed.rs`, its README, icons
  and binstall metadata (`pkg-url` unchanged: the files are still
  `sapphire-bridge-<os>-<arch>`).
- `desktop`:
  - Its command line is `BridgeCommand` as an optional subcommand, and
    `--page <status|workgroup|devices|embedding|appearance|logs|about>` (default
    `status`).
  - With a subcommand it runs `BridgeCommand::dispatch_with`, as `sapphire-bridge` does.
  - Without one it opens the window with `app::run_bridge`.
  - If the window cannot open (no display), it prints why and the help, and exits 1.
- `release-bridge.yml` builds and attaches `sapphire-bridge-desktop-<os>-<arch>` as well.
- release-plz:
  - `sapphire-bridge-headless` joins the framework's version group in place of
    `sapphire-bridge`.
  - `sapphire-bridge-desktop` is not published, as the apps' desktop packages are not.
- The crate `sapphire-bridge` on crates.io stops at its last version. Its README's last
  release says where the binary went, and `cargo binstall sapphire-bridge-headless`
  replaces `cargo binstall sapphire-bridge`.

## The window (`sapphire-framework-gui`)

### `app::run_bridge`

```rust
pub struct BridgeWindowOptions {
    pub title: String,               // "Sapphire Bridge"
    pub version: &'static str,
    pub page: BridgePage,            // from --page
}
pub fn run_bridge(options: BridgeWindowOptions) -> eframe::Result<()>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BridgePage { Status, Workgroup, Devices, Embedding, Appearance, Logs, About }
impl BridgePage {
    pub fn parse(s: &str) -> Option<BridgePage>;   // the --page and activation values
    pub fn as_str(self) -> &'static str;
}
```

- `run_bridge` takes the single-instance lock under the name `sapphire-bridge-desktop`.
  - When another window holds it, the start activates that window with `page.as_str()`
    and exits 0.
  - When this window holds it, an activation's message opens that page; a message that
    is not a page leaves the page as it is.
- The window is corundum's `Shell` with `Settings` in bridge mode. Its pages:

| Page | Contents |
|---|---|
| Status | Version and state, this device's name, the workgroup's name, the devices with connected ones marked, the synced workspaces and their roles; Restart (`Command::Restart(Process::Bridge)`) |
| Workgroup | `WorkgroupView` |
| Devices | `DeviceList` |
| Embedding | `EmbeddingView` and its settings section |
| Appearance | The theme and the language, as an app's Appearance page edits them |
| Logs | The last 500 lines of the newest file in `BridgeDir::log_dir()`, read again every 2 s while the page is shown; a "Open the folder" button |
| About | The version, and `UpdatesPanel` with one target, the running `sapphire-bridge-desktop` (the GitHub-release updater; part C adds the package's row) |

- When no bridge answers, the status banner says so and offers Start. Start uses the OS
  service when one is registered (its start), else runs this executable with `serve`,
  detached.

### `FrameworkClient` in bridge-only mode

- `ClientConfig::bridge_only(version)` connects with `BridgeClient::connect_running`
  instead of through an app server.
- The snapshot's `server` is `Conn::Absent` and its `bridge` is filled as today.
- Bridge commands go to the bridge's client directly. A command that needs the app server
  answers an error, and no bridge-mode page sends one.
- `ClientConfig::for_app` (today's) is unchanged.

## The apps (desktop)

### Settings › Sync

`Settings` gains `bridge_pages(BridgePages)`:

- `BridgePages::Linked` is the desktop default (`app::run`).
- `BridgePages::InApp` is what `mobile` sets, and is today's behaviour.

With `Linked`, Settings › Sync is:

- **Workspace**: as today, sync on and off included.
- **Sapphire Bridge**: a summary row and buttons, no pages.
  - The summary is the workgroup's name or "No workgroup", "N of M devices connected",
    and the bridge's version and state (running, not running, incompatible).
  - The buttons are "Open Sapphire Bridge", "Workgroup…", "Devices…" and "Embedding…".
    Each runs `sapphire-bridge-desktop --page <page>` as a detached process.
  - With no executable found, the buttons give way to:
    - "Get Sapphire Bridge", a link to the release page;
    - a line naming the command-line equivalent, `sapphire-bridge workgroup …`.

The Health banner and its actions are unchanged.

With `Linked`, the client fetches only what the summary needs (`bridge.status`,
`bridge.peers`), not `embed.settings` or `bridge.workspaces`.

### Finding `sapphire-bridge-desktop`

`platform::bridge_desktop_exe() -> Option<PathBuf>`, the first that exists of:

1. Beside the executable the bridge's OS service is registered with (corundum's
   `registered_program` for the service named `sapphire-bridge`): that executable if it is
   `sapphire-bridge-desktop`, else `sapphire-bridge-desktop` in its directory.
2. `sapphire-bridge-desktop` beside the running app's executable.
3. `sapphire-bridge-desktop` on `PATH`.

`bridge_desktop_exe_with(service_exe, own_exe, path_var)` is the same, given those, for
the tests.

### Command line

`FrameworkCommand::{Workgroup, Device, Embedding}` and their dispatch are removed; clap
then refuses them as unknown subcommands. `WorkgroupCommand` and `DeviceCommand` remain
the bridge's, and `EmbeddingCommand` remains in bridge-api.

## Errors

- No bridge: the window's banner, and the app's summary saying "not running".
- No `sapphire-bridge-desktop`: the app's "Get Sapphire Bridge" row.
- The launch failing (permissions, a broken file): an error line under the buttons with
  the OS's message.
- No display for `sapphire-bridge-desktop`: the reason and the help on stderr, exit 1.
- A log file that cannot be read: the Logs page says so, with the path.

## Testing

- **corundum:**
  - `activate_with` delivers its message to `on_activate_with`.
  - `activate` delivers `None`.
  - A message over 1 KiB is refused.
  - `on_activate` still fires for an activation that has a message.
- **The client in bridge-only mode,** against an in-process test bridge (`test-util`):
  - the snapshot has the bridge's status, peers, ledger and embedding;
  - a workgroup command reaches the bridge with no app server running;
  - with no bridge, the snapshot says so.
- **`BridgePage`:** `parse` and `as_str` round-trip, and `parse("nope")` is `None`.
- **The bridge window's Settings** (the render helper):
  - it lists the bridge's pages;
  - setting a page from an activation shows it;
  - an unknown message leaves the page as it is.
- **An app's Settings with `Linked`:**
  - no Devices, Workgroup or Embedding page;
  - the summary shows the workgroup's name and the connected count from a snapshot;
  - each button calls an injected launcher with the right `--page`;
  - with no executable, the "Get Sapphire Bridge" row shows.
- **An app's Settings with `InApp`:** the pages as today.
- **`bridge_desktop_exe_with`:** each of the three sources in order, on tempdir fixtures,
  and `None` when none has it.
- **`bridge/desktop`:** `--page` parses, and `sapphire-bridge-desktop status` runs the
  command line's code (a process test, as `cli_config`).
- **App command line:** `workgroup`, `device` and `embedding` are refused by clap, and
  `config list` still runs.

## Rollout

1. corundum-framework 0.6: the activation's message.
2. This repository, in one release:
   - the binaries' move and the new crate name;
   - the desktop binary in `release-bridge.yml`;
   - the window and the client's bridge-only mode;
   - `Settings`' bridge pages and the summary row;
   - the subcommands removed.

   The CHANGELOG's Breaking section (hand-written on the release PR) says:
   - the apps' `workgroup`, `device` and `embedding` moved to `sapphire-bridge`;
   - desktop Settings send you to Sapphire Bridge;
   - the crate is now `sapphire-bridge-headless`.

   The new crate name meets crates.io's new-crate rate limit; re-run the release job.
3. The apps update the framework. Desktop Settings change by themselves (`Linked` is the
   default); a phone app's `mobile::run` sets `InApp`.

## Out of scope

- Part A (#381): the service code moves to corundum.
- Part C: the bridge as a Velopack package, its first-run "keep the bridge running" page,
  its command on `PATH`, and the package's update row.
- A URL scheme, and opening a page from a browser.
