# corundum-gui: tint picker

Issue: #328. Builds on the theme (`2026-10-23-corundum-theme-design.md`, #330).

## Why

The theme gives every colour its lightness from where it is used, so a colour the user picks
is a hue and a chroma only — a *tint*. Today the accent is the only one, picked with two
sliders. Apps will let users colour their own things too (labels, as GitHub's issue labels;
categories), and those need the same: a picker with no lightness, a random pick, and a
default derived from the thing's name.

## Scope

In: the `Tint` type (replacing `Accent`); deriving a tint from random numbers and from
bytes; `Theme::tint`; the `TintPicker` widget (palette, hue/chroma disk, read-only preview);
`settings_ui` using it for the accent; two Lucide icons.

Out: storing or syncing tints (#327); label UIs in the apps.

## Decisions

1. **A palette pick is remembered by name.** `Tint::Swatch(Swatch::Red)` follows the theme's
   red, which a later Base16 import may change; only disk, random and hash picks store
   numbers (`Tint::Custom`).
2. **Unset is a value.** Pickers edit an `Option<Tint>`; `None` shows the caller's default
   (the accent's blue, a label's name hash). "Default" sets `None`, so a default derived
   from a name follows the name.
3. **Random and hashed tints are uniform over the disk's area.** Chroma's density grows
   with the radius, so vivid tints are common and greys rare (under 4% within the inner
   fifth of the radius) — and it is what poking the disk at random would give.
4. **The hash is FNV-1a 64, written in the crate.** std's `DefaultHasher` may change between
   Rust releases; a tint from a label's name must be the same on every device and version.
   Random numbers come from std's `RandomState`: no `rand` dependency.
5. **The caller picks the tone the picker draws in** (default `Tone::Text`): an accent is
   seen as text, a label as a fill.
6. **The lightness preview is a miniature of real use**, not a row of swatches, so it does
   not look selectable.

## Design

### `Tint` (`theme/tint.rs`)

```rust
#[derive(Clone, Copy, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Tint {
    Swatch(Swatch),
    Custom { chroma: f32, hue: f32 },   // OKLCH C, H in degrees
}

impl Tint {
    pub const MAX_CHROMA: f32 = 0.25;           // the disk's rim (was Accent::MAX_CHROMA)
    pub fn from_unit(u1: f32, u2: f32) -> Tint; // chroma √u1 · MAX_CHROMA, hue 360 · u2
    pub fn from_bytes(bytes: &[u8]) -> Tint;    // FNV-1a 64, high and low 32 bits → u1, u2
    pub fn random() -> Tint;                    // a RandomState hash → from_unit
}
```

Inputs to `from_unit` are clamped to `[0, 1)`. `Swatch` also gets the serde derives. JSON:
`{"Swatch":"Red"}`, `{"Custom":{"chroma":0.2,"hue":140.0}}`.

### Theme

- `Theme::tint(&self, tone: Tone, tint: Tint) -> Color32`: a `Swatch` reads the table; a
  `Custom` is computed (chroma scaled by the tone, gamut-mapped, hue modulo 360, chroma
  clamped to `MAX_CHROMA`; a non-finite chroma or hue reads as the default accent).
- `Theme::new(dark, contrast, accent: Tint)`. The `Accent` column is the resolved accent;
  `Tint::Swatch(Swatch::Accent)` as the accent resolves to `Swatch::Blue`.
- `ThemeSettings.accent: Option<Tint>`; `None` is `Tint::Swatch(Swatch::Blue)`
  (`ThemeSettings::accent()` returns the resolved one). `Accent` is removed; the change is
  breaking, and no settings in the old shape exist outside #330's unreleased branch.

### `TintPicker` (`theme/picker.rs`)

```rust
pub struct TintPicker<'a> { … }

impl<'a> TintPicker<'a> {
    pub fn new(value: &'a mut Option<Tint>, default: Tint) -> TintPicker<'a>;
    pub fn tone(self, tone: Tone) -> Self;     // default Tone::Text
    pub fn preview(self, on: bool) -> Self;    // default false
    pub fn show(self, ui: &mut egui::Ui) -> bool;
}
```

`show` returns `true` once a change is made: a click, or the end of a drag (the `report`
logic `settings_ui` uses, moved here). During a drag the value changes every frame.

Layout, top to bottom:

1. **Palette**, 24-point chips 4 points apart, wrapping when narrow:
   `Default, Neutral, Red, Orange, Yellow, Green, Cyan, Blue, Purple, Random`. Each chip
   has its name as hover text.
   - Colour chips: a rounded square in `theme.tint(tone, Swatch(s))`; sets
     `Some(Swatch(s))`. The selected one has a 2-point ring in `Text`.
   - **Default**: filled with `default`, the `rotate-ccw` icon on top; sets `None`; shown
     selected while the value is `None`.
   - **Random**: a ring of hues around the `dice-5` icon; sets `Some(Tint::random())`.
2. **Disk**, `min(160, available width)` across: a `Mesh` of 48 hue sectors × 8 rings,
   vertex colours `theme.tint(tone, Custom { chroma: r · MAX_CHROMA, hue })`. Hue 0° at the
   right, counter-clockwise. A click or drag sets `Some(Custom { … })` from the pointer
   (outside the rim clamps to it). The current tint (a swatch's own chroma and hue for a
   `Swatch`) is marked with a black-and-white double ring.
3. **Preview** (if on), small and frameless, `Sense::hover()`: link-style text in
   `tint(Text, t)`, a label (`Raised` fill, `Text` text) and a selected row (`Pressed`
   fill), all with the current tint.

The theme used is `theme::current(ui.ctx())`.

### `settings_ui`

The accent rows become one row holding
`TintPicker::new(&mut settings.accent, Tint::Swatch(Swatch::Blue)).preview(true)`. The
out-of-range normalising moves into `Theme::tint` (a `Custom` outside the range is read the
theme's way and never rewritten), so nothing is rewritten on first draw.

### Icons

`icons::rotate_ccw()` and `icons::dice_5()`, Lucide SVGs stroked in white like the rest.

## Testing

- `from_unit`: `(0, x)` is chroma 0; `(1−ε, 0.25)` is chroma ≈ `MAX_CHROMA`, hue ≈ 90°. A
  100 × 100 grid of inputs puts 25% ± 1% at chroma ≤ `MAX_CHROMA / 2`.
- `from_bytes`: `b"bug"` and `b""` give fixed values (pinned in the test); `b"bug"` and
  `b"feature"` differ.
- `random`: 100 draws, each in range; not all equal.
- `Theme::tint`: `Swatch(s)` equals `color(tone, s)` for every tone and swatch; `Custom`
  with hue -30 equals hue 330; chroma 0 equals Neutral; NaN reads as the default accent.
- Accent: `Swatch(Accent)` and `None` both give Blue's `Accent` column.
- `TintPicker` (positions recorded under `cfg(test)`, clicked as the Shell tests do):
  renders with and without preview; Default → `None`; a colour chip → `Some(Swatch(..))`;
  the disk's centre → `Custom` with chroma < 0.01; Random → `Custom`; a click returns
  `true` once.
- `settings_ui`: a `Custom` accent with hue -30 is drawn and not reported as a change.
- serde: `Tint` both variants round-trip; `ThemeSettings` with `accent: None` round-trips.
- Icons decode through the SVG loader (the existing icon test).
