# corundum-gui: theme

Issue: #318 (part 2, "Theme"). Follows `2026-10-22-corundum-gui-design.md`.

## Why

The apps draw with egui's default dark look and a handful of hard-coded colours
(`Color32::GREEN`, `YELLOW`, `LIGHT_RED`, …). Those were picked against a dark background:
in a light theme yellow text on white is unreadable. Users want light / dark / follow the
system, a choice of contrast, and an accent colour of their own.

Every colour — the accent and the status colours alike — should take its lightness from
where it is used: text colours from the text lightness, fills from the background
lightness. Then switching light/dark or contrast carries every colour with it, and a
user-picked accent can never end up unreadable.

User-written themes (TOML) and Base16 import are later work. This design keeps them
possible: they become other ways of building the same `Theme`.

## Scope

In: the `theme` module in corundum-gui (colour model, built-in light and dark at three
contrast levels, accent, egui `Visuals`, a settings widget); `Shell` installing the default
theme; sapphire-framework-gui's Appearance page and its hard-coded colours.

Out: user theme files, Base16 import, per-cell overrides, a brown swatch (Base16's base0F),
the apps' own hard-coded colours (a PR per app repository, after the release).

## Decisions

1. **A grid, not a list of colours.** A theme is a ladder of lightness levels (`Tone`) and a
   set of hue/chroma pairs (`Swatch`); every colour is `color(tone, swatch)`. Light/dark and
   contrast pick the ladder; the accent is one swatch the user sets.
2. **OKLCH, written in the crate.** OKLCH's L follows perceived lightness, so one L gives
   about the same contrast whatever the hue — HSV's V does not (yellow and blue at one V
   look far apart). The conversion is a few dozen lines; no colour crate is added.
3. **The theme lives in the egui context.** `install` stores it in `ctx.data`; components
   read it with `current(ctx)`. The framework's views take only `&mut Ui`, so nothing has
   to be threaded through. Both the light and the dark theme are built up front, so a
   system switch needs no rebuild.
4. **The crate stores nothing.** The app persists `ThemeSettings`; an optional `serde`
   feature derives the traits.

## Design

### Colour model (`theme/color.rs`)

- `Oklch { l, c, h }` → OKLab → linear sRGB → sRGB `Color32`.
- Gamut mapping: when the linear sRGB is outside `[0, 1]`, keep L and H and binary-search C
  down to the largest value that fits.
- `contrast_ratio(a, b)`: the WCAG 2 ratio from relative luminance.

### Tones

Nine, from the background side to the text side:

| Tone | Used for |
|---|---|
| `Sunken` | text edit background |
| `Base` | panel and window fill |
| `Raised` | inactive buttons, code background, tinted bands |
| `Hover` | hovered widget fill |
| `Pressed` | pressed and selected fill |
| `Border` | separators, window and widget strokes |
| `Muted` | weak text |
| `Text` | body text |
| `Strong` | text of hovered and active widgets |

Each tone has an OKLCH lightness and a chroma scale: background tones scale the swatch's
chroma down (about 0.3–0.5) so tinted fills stay quiet; text tones use it in full.

There are six ladders, constants: {light, dark} × {low, medium, high}. Dark runs the same
order from dark to light. The ladders are tuned to these WCAG targets:

| Pair | Low | Medium | High |
|---|---|---|---|
| `Text` on `Base` | ≥ 4.5 | ≥ 7 | ≥ 12 |
| `Muted` on `Base`, and every swatch's `Text` on `Base` | ≥ 3 | ≥ 4.5 | ≥ 7 |

Low still meets AA for body text.

### Swatches

`Neutral, Accent, Red, Orange, Yellow, Green, Cyan, Blue, Purple` — each an OKLCH (C, H).
`Neutral` has C = 0. `Accent` comes from the settings (default: a blue). Aliases:
`Swatch::SUCCESS = Green`, `WARNING = Orange`, `DANGER = Red`, `INFO = Blue`.
Red through Purple line up with Base16's base08–base0E for a later import.

### `Theme`

```rust
pub struct Theme { /* the 9 × 9 table, computed when built */ }

impl Theme {
    pub fn new(dark: bool, contrast: Contrast, accent: Tint) -> Theme;
    pub fn is_dark(&self) -> bool;
    pub fn color(&self, tone: Tone, swatch: Swatch) -> Color32;
    pub fn text(&self, swatch: Swatch) -> Color32;   // color(Text, swatch)
    pub fn fill(&self, swatch: Swatch) -> Color32;   // color(Raised, swatch)
    pub fn visuals(&self) -> egui::Visuals;
}
```

`visuals()` starts from `Visuals::dark()` / `light()` (so radii, shadows and spacing stay
egui's) and sets the colours:

| egui | Colour |
|---|---|
| `panel_fill`, `window_fill`, `noninteractive.{bg_fill, weak_bg_fill}` | `(Base, Neutral)` |
| `extreme_bg_color` | `(Sunken, Neutral)` |
| `faint_bg_color`, `code_bg_color` | `(Raised, Neutral)` |
| `inactive` / `hovered` / `active` / `open` fills | `Raised` / `Hover` / `Pressed` / `Hover`, `Neutral` |
| `noninteractive.fg_stroke`, `inactive.fg_stroke` | `(Text, Neutral)` |
| `hovered.fg_stroke`, `active.fg_stroke`, `open.fg_stroke` | `(Strong, Neutral)` |
| `window_stroke`, `noninteractive.bg_stroke`, `hovered.bg_stroke` | `(Border, Neutral)` |
| `selection.bg_fill` / `selection.stroke` | `(Pressed, Accent)` / `(Text, Accent)` |
| `hyperlink_color` | `(Text, Accent)` |
| `warn_fg_color` / `error_fg_color` | `text(WARNING)` / `text(DANGER)` |
| `weak_text_color` | `Some((Muted, Neutral))` |
| `dark_mode` | `is_dark()` |

### Settings and installation

```rust
pub enum Contrast { Low, Medium, High }          // default Medium
// The accent is a `Tint` (see 2026-10-23-corundum-tint-picker-design.md); this design
// first had `Accent { chroma, hue }`, replaced before release.

pub struct ThemeSettings {
    pub preference: egui::ThemePreference,       // default System
    pub contrast: Contrast,
    pub accent: Option<Tint>,                    // None: the default, Blue
}

pub fn install(ctx: &egui::Context, settings: &ThemeSettings);
pub fn is_installed(ctx: &egui::Context) -> bool;
pub fn current(ctx: &egui::Context) -> Arc<Theme>;
pub fn settings_ui(ui: &mut egui::Ui, settings: &mut ThemeSettings) -> bool;
```

- `install` builds the light and the dark `Theme`, calls `set_visuals_of` for each and
  `set_theme(preference)`, and stores both (`Arc`) in `ctx.data`. Calling it again replaces
  them.
- `current` returns the one for `ctx.theme()`; before any `install`, a theme built from the
  default settings (egui's visuals untouched).
- `settings_ui`: light / dark / system; low / medium / high; an accent hue slider (0–360°),
  a chroma slider, and a preview swatch. On a change it calls `install` at once (a slider
  being dragged previews live) and returns `true` once the change is made — a click, or the
  end of a drag — so an app saving then saves once per drag. An accent outside the sliders'
  range is first brought in as the theme reads it (hue modulo 360, chroma clamped), which is
  not reported as a change.
- `Shell::ui` installs the default settings on its first frame if nothing is installed yet,
  as it installs the image loaders. An app that installed its saved settings first is left
  alone.
- Feature `serde = ["dep:serde", "egui/serde"]` derives `Serialize` / `Deserialize` on
  `ThemeSettings`, `Contrast` and `Tint`.

### Module layout

```
crates/corundum-gui/src/theme/
  mod.rs       ThemeSettings, Contrast, install, is_installed, current
  color.rs     OKLCH, gamut mapping, contrast ratio
  palette.rs   Tone, Swatch, the six ladders, the swatch constants, Theme
  visuals.rs   Theme::visuals
  ui.rs        settings_ui
```

### sapphire-framework-gui

- **Appearance page** in `Settings`, between the app's sections and Sync, drawing
  `settings_ui`.

  ```rust
  /// Start from the settings the app saved.
  pub fn with_theme(self, theme: ThemeSettings) -> Settings;
  /// The settings, once, after the user changed them: the app saves these.
  pub fn take_theme_changed(&mut self) -> Option<ThemeSettings>;
  ```

  `with_theme` only records the settings; the app (or `Shell`'s first frame) installs them.
  `Shell` in sapphire-framework-gui gains a pass-through `take_theme_changed`. The
  `theme` module is re-exported as `sapphire_framework_gui::theme`.
- **Hard-coded colours** become theme colours:

  | Now | Becomes |
  |---|---|
  | `GREEN`, `from_rgb(80, 180, 100)` | `text(SUCCESS)` |
  | `YELLOW`, `from_rgb(230, 160, 40)` | `text(WARNING)` |
  | `LIGHT_RED` | `text(DANGER)` |
  | `GRAY` | `color(Muted, Neutral)` |

  In `views/devices.rs`, `views/workspaces.rs`, `views/embedding.rs`, `views/files/view.rs`,
  `views/mod.rs`, `manager.rs`, `shell.rs`.

### Releases

corundum-gui's next version (0.2.0 by release-plz, a feature) carries the theme;
sapphire-framework-gui's next release depends on it.

## Testing

- `color`: white, black and the sRGB primaries round-trip through OKLCH within 1/255;
  out-of-gamut inputs come back in gamut with L and H kept.
- Every cell of every ladder × swatch is in gamut, and meets the contrast targets above.
- Accent extremes (chroma at the slider's top, every hue in 10° steps) stay in gamut and
  meet the swatch text target.
- `install` then `set_theme(Dark)` / `set_theme(Light)`: `current` and `ctx.style().visuals`
  follow.
- `Shell` installs the defaults on its first frame, and does not replace an installed
  theme.
- `settings_ui` renders; changing a value through it reinstalls (`current` reflects it).
- sapphire-framework-gui: the Appearance page is listed and drawn; a change made there
  comes out of `take_theme_changed` once.
- `cargo publish --workspace --dry-run --all-features`; the mobile CI jobs green.
