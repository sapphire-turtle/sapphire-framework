# Theme Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The theme (preference, contrast, accent) lives in the settings registry as four
bridge-owned workgroup keys, overridable per device; the GUI follows them and the Appearance
page edits them per scope.

**Architecture:** The bridge defines `gui.theme.*` and stores them in its workgroup / device
`config.toml`. The GUI client's poll reads their effective values through the app server's
relayed `config.get`; `Settings` installs the theme when those values change; the Appearance
page writes through part 4's `Command::ConfigWrite`.

**Tech Stack:** Rust, egui, `sapphire-framework-settings`, tokio.

**Spec:** `docs/superpowers/specs/2026-10-23-theme-sync-design.md`

## Global Constraints

- Keys, kinds, defaults exactly as the spec's table: `gui.theme.preference` Enum
  `system|light|dark` default `system`; `gui.theme.contrast` Enum `low|medium|high` default
  `medium`; `gui.theme.accent_hue` Int 0–360 default 255; `gui.theme.accent_chroma` Int 0–250
  default 150. All `scopes: [Workgroup]`, `overridable: true`, `Apply::Live`, section
  `"Appearance"`, `process: Bridge`.
- corundum-gui does not change.
- Never log setting values.
- Build with the worktree's own target dir; run `cargo fmt` and
  `cargo clippy --all-targets -- -D warnings` on touched crates before each commit.
- Commit with `git add <paths>` only (never `-A`); end messages with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. A hand-edited synced file with `accent_hue = 999` or `contrast = "loud"` — the GUI must
   fall back to that field's default, not panic or ignore the whole theme (Task 3 test).
2. Dragging a slider while the poll answers — the preview must not snap back mid-drag
   (Task 4 test: unchanged snapshot installs nothing).
3. Editing the Workgroup tab while the device overrides that key — the look must end on the
   device's value (Task 5 test).
4. A bridge older than this change (`unknown setting`) — `ServerState` stays `Up`, `gui` is
   `None` (Task 2 test).
5. An existing device or workgroup `config.toml` gaining a `[gui]` table — the bridge must
   still start and validate (Task 1 test).

---

### Task 1: Bridge keys

**Files:**
- Modify: `crates/sapphire-framework-bridge-api/src/settings.rs` (new `pub mod gui`)
- Modify: `crates/sapphire-framework-bridge/src/settings.rs` (defs, reserved `gui`)
- Modify: `crates/sapphire-framework-bridge/src/config_file.rs` (`DeviceConfig`,
  `WorkgroupConfig` accept `[gui]`)
- Modify: `crates/sapphire-framework-server/src/settings.rs:53` (`BRIDGE_NAMES`)

**Interfaces:**
- Produces: `sapphire_bridge_api::settings::gui::{THEME_PREFERENCE, THEME_CONTRAST,
  THEME_ACCENT_HUE, THEME_ACCENT_CHROMA}: &str` and `THEME_KEYS: [&str; 4]` in that order.
  `BRIDGE_NAMES: [&str; 3] = ["net", "embed", "gui"]`.

- [ ] **Step 1: Failing tests** in the bridge's `settings.rs` tests:
  - `gui_theme_keys_list_their_defaults`: `list(&reg, None)` has the four keys with
    `Value::String("system")`, `"medium"`, `Value::Int(255)`, `Value::Int(150)`, origin
    `Default`.
  - `a_device_gui_value_overrides_the_workgroups`: set `gui.theme.contrast=high` in
    `Workgroup`, `low` in `Device` → `get` is `low` from `Device`; unset on `Device` → `high`
    from `Workgroup`.
  - `gui_values_out_of_range_are_rejected`: `accent_hue=361`, `accent_chroma=251`,
    `contrast="loud"` each fail `apply`.
  - `a_gui_table_in_both_files_validates`: with `[gui.theme] contrast = "high"` written by
    hand into the device file, `apply` of an unrelated key succeeds.
  - In the server's settings tests: an app owner defining `gui.x` fails with `Error::Reserved`.
- [ ] **Step 2:** `cargo test -p sapphire-framework-bridge settings` → the new tests fail.
- [ ] **Step 3:** Add `pub mod gui` with the constants; append the four defs to
  `bridge_defs()` (a second closure or `overridable: true` set after `def(...)`), Enum choices
  in the table's order, doc lines: "Light, dark, or the system's", "How far apart text and
  background are", "The accent's hue, in degrees", "The accent's chroma, in thousandths".
  Add `("gui", 0)` to `registry()`'s reserved list; add `#[serde(default)] pub gui:
  toml::Table` to `DeviceConfig` and `WorkgroupConfig` (unread); `BRIDGE_NAMES` gains `"gui"`.
- [ ] **Step 4:** `cargo test -p sapphire-framework-bridge -p sapphire-framework-server -p
  sapphire-framework-bridge-api` → pass (fix any test that counts the bridge's defs).
- [ ] **Step 5:** Commit `feat(bridge): the theme's settings, workgroup-wide and overridable (#327)`.

### Task 2: The client reads the theme in its poll

**Files:**
- Modify: `crates/sapphire-framework-gui/src/client/types.rs` (`GuiValues`, `ServerState.gui`)
- Modify: `crates/sapphire-framework-gui/src/client/conn.rs` (`fetch_server`)
- Modify: every `ServerState { .. }` literal (`settings.rs`, `views/banner.rs`, `health.rs`, tests)
- Test: `crates/sapphire-framework-gui/tests/client.rs`

**Interfaces:**
- Produces: `pub type GuiValues = BTreeMap<String, Value>;` and
  `ServerState { .., pub gui: Option<GuiValues> }`.

- [ ] **Step 1: Failing tests** in `tests/client.rs`:
  - `the_theme_reaches_the_snapshot`: bridge + server + relay up; `ConfigWrite { scope:
    Workgroup, set: [gui.theme.contrast = "high"], unset: [] }`; `wait` until
    `snapshot.server.up().gui` has `contrast == "high"` and `accent_hue == Int(255)`.
  - `a_device_override_is_what_the_snapshot_shows`: then write `low` to `Device` → `gui`
    shows `low`.
  - `no_bridge_leaves_gui_none_and_the_server_up`: server only → `server` is `Up`,
    `gui == None`.
- [ ] **Step 2:** `cargo test -p sapphire-framework-gui --test client theme` → fail to compile.
- [ ] **Step 3:** In `fetch_server`, after `update`, one `CONFIG_GET` (`ConfigGetParams`) per
  `THEME_KEYS`, collecting `EntryValue::Value(v)` into `GuiValues`; any error (no bridge,
  unknown key) makes `gui` `None` without failing the rest (`.ok()`, as `update` does).
- [ ] **Step 4:** Run the GUI tests (lib and `client`) → pass.
- [ ] **Step 5:** Commit `feat(gui): the client's poll reads the theme's settings (#327)`.

### Task 3: Converting between the keys and `ThemeSettings`

**Files:**
- Create: `crates/sapphire-framework-gui/src/views/appearance.rs` (registered in `views/mod.rs`)

**Interfaces:**
- Consumes: `GuiValues`, `gui::*` constants.
- Produces: `pub(crate) fn theme_settings(values: &GuiValues) -> ThemeSettings` and
  `pub(crate) fn gui_entries(theme: &ThemeSettings) -> Vec<ConfigEntry>` (the four keys, in
  `THEME_KEYS` order).

- [ ] **Step 1: Failing tests** in the new module:
  - `defaults_round_trip`: `theme_settings(&from(gui_entries(&ThemeSettings::default())))
    == ThemeSettings::default()`; empty map → default.
  - `each_key_maps_to_its_field`: `preference=dark` → `ThemePreference::Dark`;
    `contrast=low` → `Contrast::Low`; `accent_hue=30` → `30.0`; `accent_chroma=200` → `0.2`.
  - `a_bad_value_takes_the_defaults_field`: `accent_hue=999`, `contrast="loud"`,
    `accent_chroma=String("x")` → those fields default, the others kept.
  - `the_accent_rounds`: hue `30.6` → `Int(31)`, chroma `0.1234` → `Int(123)`.
- [ ] **Step 2:** `cargo test -p sapphire-framework-gui appearance` → fail.
- [ ] **Step 3:** Implement; ranges as the Global Constraints.
- [ ] **Step 4:** Tests pass.
- [ ] **Step 5:** Commit `feat(gui): the theme's settings as ThemeSettings and back (#327)`.

### Task 4: `Settings` follows the theme

**Files:**
- Modify: `crates/sapphire-framework-gui/src/settings.rs`
- Modify: `crates/sapphire-framework-gui/src/shell.rs` (call it every frame)

**Interfaces:**
- Produces: `pub fn follow_theme(&mut self, ctx: &egui::Context)` on `Settings`; field
  `seen_gui: Option<GuiValues>`; `pub(crate) fn forget_seen_theme(&mut self)` (sets
  `seen_gui = None`, used by Task 5 on a failed write).

Edge-triggered: install `theme_settings(gui)` only when the snapshot's `gui` is `Some` and
differs from `seen_gui`, then store it. `None` installs nothing and leaves `seen_gui`. This
is what keeps a drag's preview: an unchanged snapshot never reinstalls. `Shell::ui` calls it
right after `settings.poll()`; `Settings::ui` calls it when `own_banner` (standalone).

- [ ] **Step 1: Failing tests** (`settings.rs` tests, an `egui::Context::default()`):
  - `a_new_theme_in_the_snapshot_is_installed`: `test_snapshot` with `gui` contrast `high`
    → after `follow_theme`, `theme::current(&ctx)` is the High theme (compare with a context
    where `theme::install` was called with that `ThemeSettings`, by `visuals()` equality or
    `Theme` fields the theme exposes).
  - `an_unchanged_snapshot_installs_nothing`: follow once, then `theme::install` something
    else on `ctx`, follow again → the other theme stays.
  - `no_gui_leaves_the_theme`: `gui: None` → `theme::is_installed(&ctx)` stays false.
- [ ] **Step 2:** Run → fail.
- [ ] **Step 3:** Implement and wire into `Shell::ui`.
- [ ] **Step 4:** Tests pass (whole GUI lib).
- [ ] **Step 5:** Commit `feat(gui): install the theme the settings hold, and follow it (#327)`.

### Task 5: The Appearance page edits per scope

**Files:**
- Modify: `crates/sapphire-framework-gui/src/views/appearance.rs` (`AppearancePage`)
- Modify: `crates/sapphire-framework-gui/src/settings.rs` (draw it; join `"Appearance"`;
  route its writes)

**Interfaces:**
- Consumes: part 4's `ConfigState { effective, by_scope }`, `Command::ConfigWrite`,
  `ensure_loaded`; Tasks 3–4.
- Produces: `pub(crate) struct AppearancePage { tab: Scope, working: ThemeSettings, editing:
  bool, error: Option<String> }` with
  `fn ui(&mut self, ui, state: Option<&ConfigState>, gui: Option<&GuiValues>, unavailable:
  Option<&str>) -> Option<Command>` and `fn on_outcome(&mut self, result) -> bool` (true on
  failure, so `Settings` calls `forget_seen_theme`). `Origin::Appearance` in `Settings`.

Behaviour (the spec's "The Appearance page"):
- `pages(..)` gets `&[JOINED_SCREEN.1, "Appearance"]`, so the section is placed on the
  Appearance page and not under "Configuration"; the Appearance page never draws the generic
  form for it.
- Tabs Workgroup, Device. Tab values: Workgroup = `by_scope[Workgroup]` over defaults;
  Device = `effective`. Unless `editing`, `working` is reset to the tab's values each frame;
  `editing` is set when `settings_ui` changed `working` without returning `true`, and cleared
  on `true` or when no pointer button is down.
- Workgroup tab: when `by_scope[Device]` sets any theme key, a weak line `This device uses
  its own {names}` (names: `light/dark`, `contrast`, `accent`).
- Device tab: per key set on the device, nothing; others a weak `from workgroup` /
  `default` line listing them; a `Use the workgroup's` button when the device sets any key →
  `ConfigWrite { scope: Device, set: [], unset: THEME_KEYS }`.
- On `settings_ui` → `true`: `ConfigWrite { scope: tab, set: entries of gui_entries(working)
  that differ from the tab's values, unset: [] }` (nothing sent when none differ); then
  install the expected effective theme: for each key, the device's value where the device
  sets it and the tab is Workgroup, else `working`'s.
- `state == None` or `gui == None`: `settings_ui` inside `add_enabled_ui(false, ..)` over
  `gui.map(theme_settings).unwrap_or_default()`, with `unavailable` text: no app server →
  part 4's unavailable line; bridge down → `sapphire-bridge is not running`; bridge up but
  `gui == None` → `this sapphire-bridge does not know the appearance settings; update it`.
- On a write's failure: `error` shown above the widget, a reload (`load()`), and
  `forget_seen_theme()`. On success: reload.

- [ ] **Step 1: Failing tests** (headless `render`, `test_snapshot`, `sent`):
  - `appearance_draws_both_tabs` at `[900, 600]` and `[400, 700]`.
  - `a_change_writes_only_the_changed_keys`: drive `AppearancePage` directly: set `working`
    contrast High on the Workgroup tab and call the write path → `ConfigWrite { scope:
    Workgroup, set: [gui.theme.contrast="high"], unset: [] }`.
  - `use_the_workgroups_unsets_all_four_on_the_device`.
  - `the_override_line_names_the_overridden`: device sets contrast → drawn text contains
    `This device uses its own contrast`.
  - `an_overridden_key_keeps_the_devices_look`: Workgroup tab, device contrast `low`, change
    workgroup contrast to `high` → installed theme is Low.
  - `the_section_is_not_listed_under_configuration`: schema with an `"Appearance"` section →
    `list()` has no `Page::Config("Appearance")`.
  - `disabled_without_a_bridge` and `disabled_with_an_old_bridge`: drawn text has the
    spec's lines.
  - `a_failed_write_forgets_the_seen_theme`.
- [ ] **Step 2:** Run → fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** GUI lib tests pass.
- [ ] **Step 5:** Commit `feat(gui): the Appearance page writes the theme per scope (#327)`.

### Task 6: Deprecate the app-side theme API; docs

**Files:**
- Modify: `crates/sapphire-framework-gui/src/settings.rs`, `src/shell.rs`, `src/lib.rs` docs
- Modify: `CHANGELOG.md` only if the repo keeps per-PR entries there (check `RELEASING.md`;
  release-plz may write it)

- [ ] **Step 1: Failing test** `take_theme_changed_answers_none_after_a_change` (with
  `#[allow(deprecated)]`); replace main's `a_theme_change_is_taken_once` and
  `with_theme_is_what_the_page_edits`.
- [ ] **Step 2:** Run → fail.
- [ ] **Step 3:** `#[deprecated(note = "the framework saves the theme in the settings
  registry (#327)")]` on `Settings::with_theme` (returns `self` unchanged),
  `Settings::take_theme_changed` and `Shell::take_theme_changed` (answer `None`); drop the
  `theme` / `theme_changed` fields. Update the crate docs to say the theme is saved and
  synced by the framework.
- [ ] **Step 4:** `cargo test` and `cargo clippy --all-targets -- -D warnings` for
  `sapphire-framework-gui`, `-bridge`, `-server`, `-bridge-api`, `corundum-gui` → pass.
- [ ] **Step 5:** Commit `feat(gui)!: the framework saves the theme; deprecate the app-side API (#327)`.
