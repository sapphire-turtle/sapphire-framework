# The theme in the settings registry, synced across the workgroup

#327. It builds on the theme (#318 part 2, `2026-10-23-corundum-theme-design.md`) and on the
GUI's settings pages (#314 part 4, `2026-10-23-gui-settings-design.md`).

## Goal

A user's theme — light / dark / system, the contrast, the accent — is the same on every
device of their workgroup and in every sapphire app, without the apps saving anything, and a
device can keep its own where it wants to. corundum-gui stays free of storage: it is used by
asteria too, and only ever hands `ThemeSettings` values in and out.

Success means:

- changing the theme on one device changes it on the others once the workgroup has synced,
  in apps that are open, without a restart;
- a device can override any of the theme's settings for itself, and drop the override again;
- `<app> config` reads and writes the theme like any other setting;
- an app built on `Shell` / `Settings` needs no code to save or load the theme;
- corundum-gui does not change.

## Decisions

1. **The bridge owns the keys, in the workgroup scope.** The workgroup's `config.toml` is the
   one file every device of the workgroup syncs, and it is the bridge's. The bridge defines
   and stores the keys and never reads them. One theme for every sapphire app of the
   workgroup.
2. **Every key is synced and overridable per device.** Home `Workgroup`, `overridable: true`.
3. **The Appearance page keeps the theme widget**, with the settings pages' scope tabs above
   it, instead of the generic form (which would make the accent two number fields).
4. **A change is written at once**, without Apply: the page previews live, as it does today,
   and a theme is cheap to change back. This page alone differs from the settings pages'
   draft-and-Apply.
5. **The GUI follows the effective theme through its poll**, and installs it whenever it
   differs from what is installed. Start-up, a sync from another device and a `config set`
   from the CLI all take the same path.
6. **No workspace scope** (see Out of scope).

## Keys

Defined by the bridge in `sapphire-framework-bridge/src/settings.rs`, section `"Appearance"`,
all `Kind` as below, `scopes: [Workgroup]`, `overridable: true`, `Apply::Live`:

| Key | Kind | Default | `ThemeSettings` field |
|---|---|---|---|
| `gui.theme.preference` | Enum `system`, `light`, `dark` | `system` | `preference` |
| `gui.theme.contrast` | Enum `low`, `medium`, `high` | `medium` | `contrast` |
| `gui.theme.accent_hue` | Int 0–360 | 255 | `accent.hue` (degrees) |
| `gui.theme.accent_chroma` | Int 0–250 | 150 | `accent.chroma` × 1000 |

`Kind` has no float, so the accent is kept in whole degrees and thousandths of chroma
(`Accent::MAX_CHROMA` is 0.25). The defaults are `ThemeSettings::default()`'s. Hue 360 is
accepted because the theme widget keeps 360 itself.

`gui` becomes a reserved top-level name of the bridge's registry (`("gui", 0)` beside `net`
and `embed`) and of the app server's `BRIDGE_NAMES`, so no app can define a
`gui.*` key. The bridge's `validate` does not look at `[gui]`; the registry's kind checks are
enough.

A device not in a workgroup stores them in its device file, as every workgroup key does
there today (`BridgeOwner::file`), and they move with nothing when it joins one: the
workgroup's values win only where the device has not set its own — the existing
`overridable` resolution.

The key names are constants in `sapphire-framework-bridge-api`
(`settings::gui::THEME_PREFERENCE` and the others; a new module, so a minor release), shared by
the bridge's definitions and the GUI.

## Reading the theme: the poll

`ServerState` (the snapshot's app-server part) gains `gui: Option<GuiValues>`, where
`GuiValues` is the effective value of each `gui.theme.*` key (`BTreeMap<String, Value>`).
`fetch_server` reads it with one `config.get` per key, relayed by the app server to the
bridge, after the calls it makes today. `None` when the bridge is not running or does not
know the keys (an older bridge answers `unknown setting`); the rest of `ServerState` is
unaffected by that failure.

The client stays egui-free: it holds the raw values. The conversion lives beside the views:

- `theme_settings(&GuiValues) -> ThemeSettings`: each key that is missing or out of range
  takes the default's field (the bridge checks kinds on write, but a synced file may have
  been written by hand);
- `gui_entries(&ThemeSettings) -> Vec<ConfigEntry>`: the four keys, rounding the accent to
  whole degrees and thousandths.

### Installing

`Settings::follow_theme(ctx)` (which the shell calls every frame, after `poll`) installs
`theme_settings` of the snapshot's `gui` when it differs from the `gui` it last saw — on a
change of the values, not of the installed look — so a drag's live preview on the
Appearance page is never undone by a poll that brings nothing new. A failed write on the
page forgets the last seen values, so the next poll restores the stored theme. With
`gui == None` nothing is installed: the theme stays as it is (the defaults the shell
installs at start, until the first answer).

At start the defaults show until the first poll answers, usually well under a second with
the services running. There is no local cache of the theme.

## The Appearance page

The page joins the settings pages of part 4: the schema's `"Appearance"` section is claimed
by it (as `"Embedding"` is by the Embedding screen), so it is not also listed under
"Configuration", and its keys are drawn by the theme widget, not by the generic form.

- **Tabs**: Workgroup, then Device (the scopes the keys allow, in the order part 4 uses).
  Their contents come from the `ConfigState` that part 4's `ConfigLoad` reads, loaded when
  the page opens and after each write.
- **Workgroup tab**: `theme::settings_ui` over the workgroup's values (the defaults for keys
  it does not set). Where the device overrides a key, a weak line above the widget names the
  overridden settings — `This device uses its own contrast` — and the look follows the
  device's value once a change is written.
- **Device tab**: `theme::settings_ui` over the effective values. Keys this device sets are
  marked as its own; the others show `from workgroup` (or `default`). A change writes only
  the keys it changed, to the device. A button, `Use the workgroup's`, shown while the device
  sets any key, unsets all four on the device.
- **Writes**: when `settings_ui` returns `true` (a click, or the end of a drag), the page
  sends part 4's `Command::ConfigWrite` with the tab's scope and the keys whose value
  differs from the tab's loaded one, and installs the resulting effective theme at once. On
  failure the error shows above the widget, the page reloads, and the poll restores the
  installed theme.
- **Unavailable**: with no app server, part 4's `unavailable` line. With `gui == None` (no
  bridge, or one that does not know the keys), the widget is drawn disabled over the
  installed theme, with `sapphire-bridge is not running` or `this sapphire-bridge does not
  know the appearance settings; update it`.
- The restart banner never lists these keys: they are `Live`.

## The app-facing API

The framework saves the theme now, so:

- `Settings::with_theme` and `Settings::take_theme_changed`, and `Shell::take_theme_changed`,
  become `#[deprecated]`: `with_theme` does nothing, `take_theme_changed` always answers
  `None`. They go in the next breaking release.
- corundum-gui's `theme` module, its `serde` feature and `settings_ui` are unchanged.

## Order

Part 4 (`feat/gui-settings`) does not yet contain #330's Appearance page; it merges main
first (its `Page` gains `Appearance` back beside `Config`). This work then branches from
part 4, or from main once part 4 is merged, and touches:

- `sapphire-framework-bridge-api`: the `settings::gui` key constants;
- `sapphire-framework-bridge`: the four definitions and the reserved `gui`;
- `sapphire-framework-server`: `gui` in `BRIDGE_NAMES`;
- `sapphire-framework-gui`: `GuiValues` in the poll, the conversion, installing in `poll`,
  the Appearance page's tabs and writes, the deprecations.

## Tests

- Bridge: the four keys are listed with their defaults; a workgroup value is the effective
  one; a device value overrides it and `unset` on the device restores it; out-of-range values
  and unknown choices are rejected; an app owner defining `gui.x` fails as reserved.
- Conversion (no rendering): defaults round-trip; each key maps to its field; a missing or
  out-of-range value takes the default's field; the accent rounds to whole degrees and
  thousandths.
- `Settings::poll`: a changed `gui` in the snapshot installs the new theme; an unchanged one
  installs nothing; `None` leaves the installed theme; no install during a drag.
- Appearance page, rendered headless (`test_support::render`): both tabs; the override line
  on the Workgroup tab; `from workgroup` and `Use the workgroup's` on the Device tab; the
  disabled states; a change sends one `ConfigWrite` with only the changed keys and the
  tab's scope.
- Client integration (real bridge and app server, `tests/client.rs`): a `config set` of
  `gui.theme.contrast` on the workgroup reaches the snapshot's `gui`; with a device override
  the snapshot shows the device's value; with no bridge `gui` is `None` and `server` is still
  up.
- The deprecated methods: `take_theme_changed` answers `None` after a change on the page.

## Out of scope

- **A workspace scope for the theme.** The registry resolves a key from the device (when
  overridable), then its home scope, then the default — never a third layer — and a key has
  one owner, while the workgroup file is the bridge's and a workspace's file is the app
  server's. A per-workspace theme would need resolution through several scopes and the app
  server layering its workspace file over a bridge key; it is left for a later issue if it
  is wanted.
- The language setting (#329). It is meant to follow the same path — a bridge-owned
  `gui.locale` in the workgroup scope, overridable — but its keys and page belong to #329.
- A local cache of the theme for the first frames before the services answer.
- A per-app theme.
