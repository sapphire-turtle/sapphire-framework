# corundum-gui Tint Picker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A `Tint` (hue + chroma, or a theme swatch) with random and hashed constructors, and a `TintPicker` widget (palette, hue/chroma disk, read-only preview) that the accent setting uses.

**Architecture:** `theme/tint.rs` holds the type and its constructors; `Theme::tint` resolves one at a tone; the accent becomes an `Option<Tint>`; `theme/picker.rs` draws chips and a mesh disk and reports changes the way `settings_ui` does.

**Tech Stack:** Rust, egui 0.36 (`Mesh`, `Painter`), optional serde.

**Spec:** `docs/superpowers/specs/2026-10-23-corundum-tint-picker-design.md`

## Global Constraints

- No new dependencies: FNV-1a 64 written in the crate; randomness from `std::collections::hash_map::RandomState`.
- `Tint::MAX_CHROMA = 0.25`; `from_unit`: chroma `√u1 · MAX_CHROMA`, hue `360 · u2`, inputs clamped to `[0, 1)`.
- `from_bytes`: FNV-1a 64 (offset `0xcbf29ce484222325`, prime `0x100000001b3`); u1 from the high 32 bits, u2 from the low 32 bits, each `/ 2^32`.
- Default accent `Tint::Swatch(Swatch::Blue)`; `Swatch(Accent)` as the accent resolves to Blue.
- Palette order: `Default, Neutral, Red, Orange, Yellow, Green, Cyan, Blue, Purple, Random`; chips 24 pt, gap 4 pt; disk `min(160, width)`, 48 sectors × 8 rings, hue 0° at the right, counter-clockwise.
- Icons are Lucide SVGs stroked `#ffffff` like the existing ones.
- Run tests with `cargo test -p corundum-gui --all-features` from the worktree root.

## Review Focus

- A `Custom` tint with NaN / infinite chroma or hue: reads as the default accent, never black — test in Task 2.
- A drag that starts on the disk and leaves it: the value clamps to the rim and keeps following — covered by the clamp in the pointer mapping; test in Task 4 (`pointer_outside_clamps_to_rim`).
- The picker narrower than the palette: chips wrap, nothing overflows — test in Task 4 (renders at width 120).
- The value is `Some(Swatch(Accent))` in a label picker: drawn as the accent, marker at the accent's position — test in Task 4.
- Saved `ThemeSettings` with `accent: None`: round-trips and resolves to Blue — test in Task 2.

---

### Task 1: `Tint` and its constructors

**Files:**
- Create: `crates/corundum-gui/src/theme/tint.rs`
- Modify: `crates/corundum-gui/src/theme/mod.rs` (`mod tint; pub use tint::Tint;`)
- Modify: `crates/corundum-gui/src/theme/palette.rs` (serde derives on `Swatch`)

**Interfaces:**
- Produces: `enum Tint { Swatch(Swatch), Custom { chroma: f32, hue: f32 } }` (`Clone, Copy, PartialEq, Debug`, serde under the feature), `Tint::MAX_CHROMA`, `from_unit(f32, f32) -> Tint`, `from_bytes(&[u8]) -> Tint`, `random() -> Tint`; crate-private `fn fnv1a(bytes: &[u8]) -> u64`.

- [ ] **Step 1: Failing tests** in `tint.rs`: `from_unit_maps_radius_and_angle` ((0, 0.5) → chroma 0; (0.999999, 0.25) → chroma within 1e-3 of MAX, hue within 0.1 of 90); `from_unit_is_area_uniform` (100×100 grid of cell centres, share with chroma ≤ MAX/2 within 0.24..0.26); `fnv1a_known_values` (`fnv1a(b"") == 0xcbf29ce484222325`, `fnv1a(b"a") == 0xaf63dc4c8601ec8c`); `from_bytes_is_fixed` (pin `from_bytes(b"bug")` to the value the first run prints, as `Custom` with chroma/hue to 1e-6); `from_bytes_differs` (`b"bug"` vs `b"feature"`); `random_is_in_range_and_varies` (100 draws, chroma in [0, MAX], hue in [0, 360), at least 2 distinct); `#[cfg(feature = "serde")] tint_round_trips` (both variants via serde_json).
- [ ] **Step 2:** `cargo test -p corundum-gui --all-features theme::tint` — FAIL (not defined).
- [ ] **Step 3: Implement.** `random`: `RandomState::new().build_hasher()` fed a process-wide `AtomicU64` counter, `finish()` split as in `from_bytes`.
- [ ] **Step 4:** same command — PASS (after pinning the `from_bytes(b"bug")` value).
- [ ] **Step 5: Commit** `feat(corundum-gui): Tint, with random and hashed tints`

### Task 2: The accent becomes a `Tint`; `Theme::tint`

**Files:**
- Modify: `crates/corundum-gui/src/theme/{mod.rs, palette.rs, visuals.rs, ui.rs}`

**Interfaces:**
- Consumes: Task 1.
- Produces: `Theme::new(dark: bool, contrast: Contrast, accent: Tint) -> Theme`; `Theme::tint(&self, tone: Tone, tint: Tint) -> Color32`; `ThemeSettings { preference, contrast, accent: Option<Tint> }`; `ThemeSettings::accent(&self) -> Tint`; `Accent` removed. Crate-private `Swatch::chroma_hue(self) -> (f32, f32)` (no accent argument; `Accent` maps to Blue's values) and `fn resolve(tint: Tint, accent: Tint) -> (f32, f32)` normalising hue modulo 360, chroma into `[0, MAX]`, non-finite → Blue's values.

- [ ] **Step 1: Failing tests** (palette.rs): `tint_of_a_swatch_is_its_cell` (every tone × swatch); `custom_hue_wraps_and_grey_is_neutral` (hue -30 == 330; chroma 0 == Neutral, every tone); `non_finite_reads_as_default_accent` (chroma NaN, hue inf → equals `tint(tone, Swatch(Blue))`); `accent_swatch_accent_is_blue` (`Theme::new(_, _, Swatch(Accent))` Accent column == Blue column). (mod.rs): `none_accent_is_blue` and `#[cfg(feature = "serde")] settings_with_no_accent_round_trip`. Existing tests switch from `Accent { chroma, hue }` to `Tint::Custom { chroma, hue }` and `Accent::MAX_CHROMA` to `Tint::MAX_CHROMA`.
- [ ] **Step 2:** `cargo test -p corundum-gui --all-features theme` — FAIL (compile).
- [ ] **Step 3: Implement.** `ui.rs` keeps its sliders for now, editing a local `(chroma, hue)` taken from `settings.accent()` and writing `Some(Custom{..})` back only when a slider changed (Task 5 replaces them); its normalising code goes (Theme reads out-of-range values itself).
- [ ] **Step 4:** `cargo test -p corundum-gui --all-features` — PASS; `cargo test -p sapphire-framework-gui --lib` — PASS.
- [ ] **Step 5: Commit** `feat(corundum-gui)!: the accent is a Tint; Theme::tint`

### Task 3: Icons

**Files:**
- Create: `crates/corundum-gui/assets/icons/rotate-ccw.svg`, `dice-5.svg` (from `https://unpkg.com/lucide-static@0.460.0/icons/<name>.svg`, stroke set to `#ffffff`, same header as `x.svg`)
- Modify: `crates/corundum-gui/src/icons.rs` (`rotate_ccw()`, `dice_5()`, both in the test's `ALL`)

- [ ] **Step 1:** add both to `ALL` in the test — FAIL (not defined).
- [ ] **Step 2:** add the SVGs and functions; `cargo test -p corundum-gui icons` — PASS.
- [ ] **Step 3: Commit** `feat(corundum-gui): rotate-ccw and dice-5 icons`

### Task 4: `TintPicker`

**Files:**
- Create: `crates/corundum-gui/src/theme/picker.rs`
- Modify: `crates/corundum-gui/src/theme/ui.rs` (move `report` to picker.rs as `pub(super)`)
- Modify: `crates/corundum-gui/src/theme/mod.rs` (`pub use picker::TintPicker;`)

**Interfaces:**
- Consumes: Tasks 1–3, `theme::current`.
- Produces: `TintPicker::new(&'a mut Option<Tint>, default: Tint)`, `.tone(Tone)`, `.preview(bool)`, `.show(&mut Ui) -> bool`. Under `cfg(test)`: `struct PickerRects { chips: Vec<Rect>, disk: Rect }` stored in `ui.data` at `Id::new("corundum_tint_picker_rects")` for the tests.

Pointer mapping (the only algorithm the tests do not pin): `d = pos − centre`; `r = (|d| / radius).min(1)`; `hue = atan2(−d.y, d.x)` in degrees `rem_euclid(360)` (screen y points down, so negate for counter-clockwise); value `Custom { chroma: r · MAX_CHROMA, hue }`.

- [ ] **Step 1: Failing tests** (click helper as in `shell.rs` tests, over a context with the theme installed): `renders_with_and_without_preview` (also at width 120); `default_chip_unsets` (value `Some(Swatch(Red))` → click chip 0 → `None`, `show` returned true once); `colour_chip_sets_swatch` (chip 3 → `Some(Swatch(Orange))`); `disk_centre_is_grey` (→ `Custom` chroma < 0.01); `random_chip_sets_custom` (last chip → `Some(Custom{..})`); `pointer_outside_clamps_to_rim` (unit test of the mapping fn: a point at 2× radius → chroma == MAX); `accent_swatch_marks_the_accent` (unit test of the marker fn: `Swatch(Accent)` → the accent's (chroma, hue)); `report` tests moved from ui.rs.
- [ ] **Step 2:** `cargo test -p corundum-gui --all-features theme::picker` — FAIL.
- [ ] **Step 3: Implement** per the spec's layout (palette in `horizontal_wrapped`; disk via `egui::Mesh` with per-vertex colours; marker as two circles, black 3 pt then white 1.5 pt; random chip as a ring of 24 segment strokes in `tint(tone, Custom{MAX, h})`; preview with `Sense::hover()`).
- [ ] **Step 4:** same command — PASS.
- [ ] **Step 5: Commit** `feat(corundum-gui): TintPicker — palette, hue/chroma disk, preview`

### Task 5: `settings_ui` uses the picker

**Files:**
- Modify: `crates/corundum-gui/src/theme/ui.rs`

- [ ] **Step 1: Failing test** `the_accent_row_is_a_tint_picker`: render `settings_ui`; the picker rects (from `ui.data`) exist. Keep `out_of_range_accents_are_not_reported_as_changes` (now with `Custom` values) and `settings_ui_renders`; drop `a_hue_at_the_slider_end_stays_there` (no slider).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3: Implement:** one "Accent" row with `TintPicker::new(&mut settings.accent, Tint::Swatch(Swatch::Blue)).preview(true)`; drop the sliders and the preview function; the picker's result ORs into the returned bool, other rows report as before.
- [ ] **Step 4:** `cargo test -p corundum-gui --all-features` and `cargo test -p sapphire-framework-gui --lib` — PASS.
- [ ] **Step 5: Commit** `feat(corundum-gui): pick the accent with the TintPicker`

### Task 6: Checks

- [ ] `cargo fmt --all --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `cargo clippy -p corundum-gui --no-default-features --features serde --target aarch64-linux-android -- -D warnings`; `cargo test --workspace --all-features` — all clean.
- [ ] Update the theme spec's `ThemeSettings`/`Accent` text to point at the tint spec; commit `docs(spec): the accent is a Tint`.
