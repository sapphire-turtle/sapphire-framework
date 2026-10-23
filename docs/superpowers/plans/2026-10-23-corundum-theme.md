# corundum-gui Theme Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Light / dark / system themes with three contrast levels and a user accent in corundum-gui, used by sapphire-framework-gui in place of its hard-coded colours.

**Architecture:** A `theme` module in corundum-gui builds a 9 × 9 table of colours (tone ladder × swatch) from OKLCH, converts it to egui `Visuals`, and stores the light and dark themes in the egui context. Components read colours with `theme::current(ctx)`. sapphire-framework-gui adds an Appearance page and swaps its hard-coded colours for theme colours.

**Tech Stack:** Rust, egui 0.36, optional serde.

**Spec:** `docs/superpowers/specs/2026-10-23-corundum-theme-design.md`

## Global Constraints

- No new colour crate: OKLCH conversion is written in `theme/color.rs`.
- corundum-gui depends on egui only (plus optional `serde`); no eframe.
- Feature `serde = ["dep:serde", "egui/serde"]` on corundum-gui; `serde` from the workspace (`serde.workspace = true, optional = true`).
- WCAG targets — `Text` on `Base`: Low ≥ 4.5, Medium ≥ 7, High ≥ 12; `Muted` on `Base` and every swatch's `Text` on `Base`: Low ≥ 3, Medium ≥ 4.5, High ≥ 7.
- Defaults: `ThemePreference::System`, `Contrast::Medium`, accent a blue.
- `Swatch::SUCCESS = Green`, `WARNING = Orange`, `DANGER = Red`, `INFO = Blue`.
- `visuals()` starts from `Visuals::dark()` / `Visuals::light()` and changes colours only.
- Match the surrounding doc-comment style (full sentences, `[`links`]`).
- Run tests with `cargo test -p <crate>` from the worktree root; the worktree's `target/` is on disk.

## Review Focus

- Accent chroma 0 (a grey accent): any hue gives the neutral grey, no NaN — test in Task 2.
- Accent hue outside 0–360 (360, -30, 720): treated modulo 360 — test in Task 2.
- Selected text: `selection.stroke` on `selection.bg_fill` must stay readable (≥ the `Muted` target) — test in Task 3.
- `ThemePreference::System` before the OS has reported a theme: `current` still returns a theme matching `ctx.theme()` — test in Task 4.
- Saved settings: a `ThemeSettings` round-trips through serde (JSON) unchanged — test in Task 4 (behind the feature).

---

### Task 1: OKLCH colour maths

**Files:**
- Create: `crates/corundum-gui/src/theme/mod.rs` (only `mod color;` for now, plus the module doc)
- Create: `crates/corundum-gui/src/theme/color.rs`
- Modify: `crates/corundum-gui/src/lib.rs` (`pub mod theme;`)

**Interfaces:**
- Produces (crate-private, `pub(crate)`):
  - `struct Oklch { l: f32, c: f32, h: f32 }` (h in degrees)
  - `fn to_color32(c: Oklch) -> egui::Color32` — gamut-mapped
  - `fn from_color32(c: egui::Color32) -> Oklch` — for tests and later Base16 import
  - `fn contrast_ratio(a: egui::Color32, b: egui::Color32) -> f32` — WCAG 2, ≥ 1

- [ ] **Step 1: Write failing tests in `color.rs`**
  - `round_trips_white_black_and_primaries`: for `WHITE, BLACK, from_rgb(255,0,0), (0,255,0), (0,0,255)`, `to_color32(from_color32(c))` differs from `c` by ≤ 1 per channel.
  - `white_is_l1_black_is_l0`: `from_color32(WHITE).l ≈ 1.0`, `BLACK ≈ 0.0` (±0.001).
  - `out_of_gamut_keeps_lightness_and_hue`: `Oklch { l: 0.3, c: 0.4, h: 95.0 }` → result's `from_color32` has `l` within 0.01 of 0.3 and `h` within 2° of 95.
  - `contrast_extremes`: white/black = 21 (±0.05); a colour with itself = 1; order does not matter.
- [ ] **Step 2:** `cargo test -p corundum-gui theme::color` — FAIL (not defined).
- [ ] **Step 3: Implement.** OKLab matrices from Björn Ottosson's reference; sRGB transfer function both ways. Gamut mapping: if any linear channel is outside `[-1e-4, 1 + 1e-4]`, binary-search `c` in `[0, c]` (20 iterations) for the largest in-gamut chroma, then clamp channels. `h` normalised with `rem_euclid(360.0)`.
- [ ] **Step 4:** `cargo test -p corundum-gui theme::color` — PASS.
- [ ] **Step 5: Commit** `feat(corundum-gui): OKLCH colour conversion and WCAG contrast`

### Task 2: Tones, swatches and `Theme`

**Files:**
- Create: `crates/corundum-gui/src/theme/palette.rs`
- Modify: `crates/corundum-gui/src/theme/mod.rs` (types below, `pub use palette::{Theme, Tone, Swatch};`)

**Interfaces:**
- Consumes: Task 1's `Oklch`, `to_color32`, `contrast_ratio`.
- Produces (public, in `theme`):
  - `enum Contrast { Low, Medium, High }` — `Default = Medium`; derive `Clone, Copy, PartialEq, Eq, Debug`.
  - `struct Accent { pub chroma: f32, pub hue: f32 }` — `Default = { chroma: 0.15, hue: 255.0 }`; `Clone, Copy, PartialEq, Debug`; `pub const MAX_CHROMA: f32 = 0.25`.
  - `enum Tone { Sunken, Base, Raised, Hover, Pressed, Border, Muted, Text, Strong }` with `Tone::ALL: [Tone; 9]`.
  - `enum Swatch { Neutral, Accent, Red, Orange, Yellow, Green, Cyan, Blue, Purple }` with `Swatch::ALL: [Swatch; 9]` and consts `SUCCESS, WARNING, DANGER, INFO`.
  - `struct Theme` with `new(dark: bool, contrast: Contrast, accent: Accent) -> Theme`, `is_dark(&self) -> bool`, `color(&self, Tone, Swatch) -> Color32`, `text(&self, Swatch)`, `fill(&self, Swatch)`.

Starting values (tune only the L values, keeping each ladder monotonic, until the tests pass):

| Tone | chroma scale | Dark M (L) | Light M (L) |
|---|---|---|---|
| Sunken | 0.2 | 0.16 | 0.99 |
| Base | 0.2 | 0.21 | 0.97 |
| Raised | 0.35 | 0.27 | 0.92 |
| Hover | 0.45 | 0.32 | 0.88 |
| Pressed | 0.6 | 0.38 | 0.84 |
| Border | 0.4 | 0.42 | 0.78 |
| Muted | 1.0 | 0.70 | 0.50 |
| Text | 1.0 | 0.88 | 0.28 |
| Strong | 1.0 | 0.96 | 0.18 |

Low pulls the text tones toward Base, High pushes them away (and Base toward black / white); start Low/High from Medium ± 0.06 on the text side.

Swatches (C, H): Red (0.17, 25), Orange (0.15, 55), Yellow (0.14, 95), Green (0.14, 145), Cyan (0.12, 200), Blue (0.15, 255), Purple (0.15, 305), Neutral (0, 0). Accent from the argument.

- [ ] **Step 1: Write failing tests in `palette.rs`**
  - `every_ladder_meets_its_targets`: for dark in {false, true}, each `Contrast`, default accent: `contrast_ratio(color(Text, Neutral), color(Base, Neutral))` ≥ the Text target; `Muted`/`Neutral` and `text(s)` for every `s` in `Swatch::ALL` ≥ the Muted target.
  - `ladders_are_monotonic`: background → text order, the neutral column's `from_color32(..).l` strictly increases (dark) / decreases (light).
  - `accent_extremes_meet_the_swatch_target`: hue 0..360 step 10, chroma `MAX_CHROMA`, each ladder: `text(Accent)` on Base ≥ the Muted target.
  - `grey_accent_is_neutral`: `Accent { chroma: 0.0, hue: h }` for h in {0, 123, 300}: `color(t, Accent) == color(t, Neutral)` for every tone.
  - `hue_wraps`: accents with hue 360 / -30 / 720 equal those with 0 / 330 / 0 in every cell.
  - `aliases`: `Swatch::DANGER == Swatch::Red`, etc.
- [ ] **Step 2:** `cargo test -p corundum-gui theme::palette` — FAIL.
- [ ] **Step 3: Implement** `Theme::new` computing all 81 cells into `[[Color32; 9]; 9]` (cell = `to_color32(Oklch { l: tone.l, c: swatch.c * tone.chroma_scale, h: swatch.h })`); tune the L table until Step 1 passes. If a hue cannot meet the target at `MAX_CHROMA`, lower `MAX_CHROMA` rather than the targets, and note it in the PR.
- [ ] **Step 4:** `cargo test -p corundum-gui theme::palette` — PASS.
- [ ] **Step 5: Commit** `feat(corundum-gui): theme tones, swatches and colour table`

### Task 3: egui `Visuals`

**Files:**
- Create: `crates/corundum-gui/src/theme/visuals.rs`

**Interfaces:**
- Consumes: Task 2's `Theme`.
- Produces: `impl Theme { pub fn visuals(&self) -> egui::Visuals }` — mapping exactly as the spec's table.

- [ ] **Step 1: Write failing tests**
  - `visuals_take_the_theme_colours`: for a dark and a light theme: `dark_mode == is_dark()`, `panel_fill == color(Base, Neutral)`, `extreme_bg_color == color(Sunken, Neutral)`, `widgets.inactive.weak_bg_fill == color(Raised, Neutral)`, `selection.bg_fill == color(Pressed, Accent)`, `hyperlink_color == text(Accent)`, `error_fg_color == text(DANGER)`, `warn_fg_color == text(WARNING)`, `weak_text_color == Some(color(Muted, Neutral))`.
  - `non_colour_fields_are_eguis`: `window_corner_radius` and `window_shadow.offset` equal `Visuals::dark()`'s (dark) / `light()`'s (light).
  - `selected_text_is_readable`: every ladder, default accent and accent hue 0..360 step 30 at `MAX_CHROMA`: `contrast_ratio(selection.stroke.color, selection.bg_fill)` ≥ the Muted target. If it fails, set `selection.stroke` to `(Strong, Accent)` or lower Pressed's chroma scale — not the target — and update the spec's table to match.
- [ ] **Step 2:** `cargo test -p corundum-gui theme::visuals` — FAIL.
- [ ] **Step 3: Implement** per the spec's table.
- [ ] **Step 4:** `cargo test -p corundum-gui theme::visuals` — PASS.
- [ ] **Step 5: Commit** `feat(corundum-gui): build egui visuals from the theme`

### Task 4: Settings, `install`, `current`, serde

**Files:**
- Modify: `crates/corundum-gui/src/theme/mod.rs`
- Modify: `crates/corundum-gui/Cargo.toml` (`serde` optional dep + feature; `serde_json` dev-dependency from the workspace)

**Interfaces:**
- Consumes: Tasks 2–3.
- Produces:
  - `struct ThemeSettings { pub preference: egui::ThemePreference, pub contrast: Contrast, pub accent: Accent }` — `Default`, `Clone, PartialEq, Debug`.
  - `fn install(ctx: &egui::Context, settings: &ThemeSettings)`
  - `fn is_installed(ctx: &egui::Context) -> bool`
  - `fn current(ctx: &egui::Context) -> std::sync::Arc<Theme>`
  - With `serde`: `Serialize, Deserialize` on `ThemeSettings`, `Contrast`, `Accent`.

Storage: one `Installed { light: Arc<Theme>, dark: Arc<Theme> }` under a fixed `egui::Id::new("corundum_theme")` in `ctx.data_mut(|d| d.insert_temp(..))`.

- [ ] **Step 1: Write failing tests**
  - `install_follows_the_egui_theme`: install defaults with preference `Dark`; `current(&ctx).is_dark()` and `ctx.style().visuals.panel_fill == current(&ctx).color(Base, Neutral)` after one frame; `ctx.set_theme(Light)` → `!is_dark()`, visuals follow.
  - `system_preference_still_gives_a_theme`: install with `System` on a fresh context; `current(&ctx).is_dark() == (ctx.theme() == egui::Theme::Dark)`.
  - `reinstall_replaces_both`: install, then install with `Contrast::High` and accent hue 30; both the light and dark `current` (via `set_theme`) equal `Theme::new(_, High, that accent)` in `color(Text, Accent)`.
  - `before_install_current_is_the_default`: fresh context: `!is_installed`, `current` equals `Theme::new(ctx.theme() == Dark, Medium, Accent::default())` in every cell; `ctx.style().visuals` unchanged.
  - `#[cfg(feature = "serde")] settings_round_trip`: non-default settings → `serde_json` → back, equal.
- [ ] **Step 2:** `cargo test -p corundum-gui --all-features theme::tests` — FAIL.
- [ ] **Step 3: Implement.** `Theme` needs `PartialEq` for the comparisons (derive it).
- [ ] **Step 4:** `cargo test -p corundum-gui --all-features` — PASS; also `cargo test -p corundum-gui` (no features) — PASS.
- [ ] **Step 5: Commit** `feat(corundum-gui): install and read the theme through the egui context`

### Task 5: `settings_ui` and `Shell` installing the defaults

**Files:**
- Create: `crates/corundum-gui/src/theme/ui.rs`
- Modify: `crates/corundum-gui/src/shell.rs` (in `ui_with_foot`, beside the loaders: `if !theme::is_installed(ctx) { theme::install(ctx, &ThemeSettings::default()) }`)
- Modify: `crates/corundum-gui/src/lib.rs` (crate doc: mention `theme`)

**Interfaces:**
- Consumes: Task 4.
- Produces: `pub fn settings_ui(ui: &mut egui::Ui, settings: &mut ThemeSettings) -> bool` (re-exported from `theme`).

Layout: a grid with rows "Appearance" (Light / Dark / System radio), "Contrast" (Low / Medium / High radio), "Accent hue" (slider 0–360, suffix "°"), "Accent strength" (slider 0–`MAX_CHROMA`), and a 24 × 24 preview square filled with `fill(Accent)` beside `text(Accent)` sample text, both from `Theme::new` of the edited settings for the current `ctx.theme()`.

- [ ] **Step 1: Write failing tests**
  - In `ui.rs`, `settings_ui_renders`: `render([600.0, 400.0], |ui| { settings_ui(ui, &mut s); })` with no panic, returns false with no input.
  - `changes_reinstall`: a helper that mutates `settings.contrast = High` and calls the same internal `apply(ctx, &settings)` the widget calls on change; `current(ctx)` reflects High. (Clicking a radio in a headless test is brittle; test the change path the widget uses.)
  - In `shell.rs`: `the_shell_installs_the_default_theme` (after `render`, `theme::is_installed`) and `the_shell_keeps_an_installed_theme` (install `Contrast::High` first, run the shell over the same context, `current` still High). These need a variant of `render` taking a context: add `render_in(ctx: &egui::Context, size, ui)` to `test_support.rs` and make `render` call it.
- [ ] **Step 2:** `cargo test -p corundum-gui` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** `cargo test -p corundum-gui` — PASS.
- [ ] **Step 5: Commit** `feat(corundum-gui): theme settings widget; the shell installs the default theme`

### Task 6: sapphire-framework-gui Appearance page

**Files:**
- Modify: `crates/sapphire-framework-gui/src/settings.rs`
- Modify: `crates/sapphire-framework-gui/src/shell.rs` (pass-through)
- Modify: `crates/sapphire-framework-gui/src/lib.rs` (`pub use corundum_gui::theme;`)

**Interfaces:**
- Consumes: `corundum_gui::theme::{ThemeSettings, settings_ui}`.
- Produces:
  - `Settings::with_theme(self, theme: ThemeSettings) -> Settings`
  - `Settings::take_theme_changed(&mut self) -> Option<ThemeSettings>`
  - `Shell::take_theme_changed(&mut self) -> Option<ThemeSettings>` (calls the settings')
  - `Page::Appearance`, listed after the app's sections and before the "Sync" heading, label "Appearance", under its own heading "General"; `first_page` unchanged.

`Settings` gains `theme: ThemeSettings` (default) and `theme_changed: bool`; the page calls `settings_ui(ui, &mut self.theme)` and sets `theme_changed` when it returns true.

- [ ] **Step 1: Write failing tests in `settings.rs`**
  - `appearance_is_listed_and_drawn`: open `Page::Appearance`, render wide and narrow; `drawn == Some(Page::Appearance)`.
  - `a_theme_change_is_taken_once`: set `theme.contrast = High` and `theme_changed = true` as the page would (through a `#[cfg(test)]` helper that runs the same code path as a `settings_ui` change); `take_theme_changed()` returns it once, then `None`.
  - `with_theme_is_what_the_page_edits`: `with_theme(s)` → the page's settings equal `s`; `take_theme_changed()` is `None`.
- [ ] **Step 2:** `cargo test -p sapphire-framework-gui settings` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** `cargo test -p sapphire-framework-gui` — PASS.
- [ ] **Step 5: Commit** `feat(sapphire-framework-gui): Appearance page for the theme`

### Task 7: Replace the hard-coded colours

**Files:**
- Modify: `crates/sapphire-framework-gui/src/views/devices.rs:185-187`, `views/workspaces.rs:66-68`, `views/embedding.rs:52`, `views/files/view.rs:135,193`, `views/mod.rs:42`, `manager.rs:112,171`, `shell.rs:156`

**Interfaces:**
- Consumes: `theme::{current, Tone, Swatch}`.

Mapping: `GREEN`, `from_rgb(80, 180, 100)` → `t.text(Swatch::SUCCESS)`; `YELLOW`, `from_rgb(230, 160, 40)` → `t.text(Swatch::WARNING)`; `LIGHT_RED` → `t.text(Swatch::DANGER)`; `GRAY` → `t.color(Tone::Muted, Swatch::Neutral)`, with `let t = theme::current(ui.ctx());`.

- [ ] **Step 1:** `grep -rnE "Color32::(GREEN|YELLOW|LIGHT_RED|GRAY)|from_rgb" crates/sapphire-framework-gui/src crates/corundum-gui/src` — lists exactly the sites above.
- [ ] **Step 2: Replace them**; drop `Color32` imports left unused.
- [ ] **Step 3:** the grep from Step 1 prints nothing (except `theme/` test code in corundum-gui).
- [ ] **Step 4:** `cargo test -p sapphire-framework-gui` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` — PASS.
- [ ] **Step 5: Commit** `refactor(sapphire-framework-gui): take status colours from the theme`

### Task 8: Release checks

- [ ] **Step 1:** `cargo fmt --all --check` — clean.
- [ ] **Step 2:** `cargo test --workspace --all-features` — PASS.
- [ ] **Step 3:** `cargo publish --workspace --dry-run --all-features` — succeeds (corundum-gui's new API is unpublished, so a failure that names only sapphire-framework-gui needing the newer corundum-gui is expected; note it in the PR, as for the API crates).
- [ ] **Step 4:** If `aarch64-linux-android` is installed locally, `cargo clippy -p corundum-gui --all-features --target aarch64-linux-android -- -D warnings`; otherwise leave it to the mobile CI jobs.
- [ ] **Step 5:** Update the spec's "Known gaps"/tables if Tasks 2–3 changed `MAX_CHROMA` or the selection stroke; commit `docs(spec): record the tuned theme values`.
