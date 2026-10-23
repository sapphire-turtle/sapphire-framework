# corundum-gui: the look shared by sapphire and asteria

Issue: #318. Related: project-asteria's design (`docs/superpowers/specs/2026-10-22-asteria-design.md`
in sapphire-turtle/project-asteria, §5.6).

## Why

project-asteria (images and music) has egui desktop and mobile clients too, and the GUI
details both projects are about to work on — a rail that works on desktop and phone,
system fonts, SVG icons, light/dark and contrast, a dyslexia-friendly font — are the same
for both. Today the look and the sapphire-specific screens share one crate,
`sapphire-framework-gui`, and the Lucide icons and their SVG loader are copied into each
app. Fixing accessibility in two frameworks the same way would drift.

So the look moves into a new crate, **`corundum-gui`** (corundum: the mineral of sapphire
and ruby; `corundum` alone is taken on crates.io, and the `-gui` suffix leaves room for
other `corundum-*` crates). Both sapphire-framework-gui and asteria depend on it.

## Scope of this spec

The issue is split into parts, each with its own spec, plan and PR:

1. **Foundation** — create the crate; move the fonts, the icons and `Shell` (decoupled from
   sapphire's `Settings`); switch sapphire-framework-gui to it; add iOS and Android CI.
   *(This spec.)*
2. **Theme** — light / dark / follow the system, contrast levels, colour tokens.
   *(Own spec.)*
3. **Mobile fonts** — the system font lookup on iOS and Android (see "Known gaps").
   *(Own spec.)*

A dyslexia-friendly font is deferred with no part assigned: the free ones (OpenDyslexic and
similar) cover Latin only, and there seems to be no free CJK one, so switching to it would
leave CJK text on the fallback anyway.

Moving each app's icons onto corundum-gui is a PR in that app's repository, after this one
is released.

## Decisions

1. **The crate lives in this repository, `crates/corundum-gui`.** A separate repository was
   considered so that iOS and Android could be CI targets from the start. They can be here:
   the crate depends on egui alone, so `cargo clippy -p corundum-gui --target <mobile>`
   builds it without the rest of the workspace, and a path filter keeps those jobs off
   every other pull request. One repository keeps the move and sapphire-framework-gui's
   switch in one PR. Splitting it out can be revisited once asteria and corundum settle.
2. **Licence MIT OR Apache-2.0**, as the rest of this workspace, so asteria
   (GPL-3.0-or-later) and sapphire can both use it.
3. **egui only.** Dependencies: `egui`, `egui_extras` (`svg`), `fontdb`, `ttf-parser`.
   `pub use egui;` as sapphire-framework-gui does, so apps and components agree on egui.
   No `eframe` feature yet: nothing moved here needs it. It is added when something
   window-level moves (YAGNI) — done in `2026-10-23-corundum-gui-eframe-design.md`.
4. **Versioned on its own, starting at 0.1.0**, outside `version_group = "framework"`:
   asteria should not see a new corundum version with every sapphire release.
5. **sapphire-framework-gui keeps its public API.** Its `Shell` becomes a thin wrapper over
   corundum-gui's, and its `fonts` / `icons` modules re-export corundum-gui's, so no app
   has to change in this PR.

## Design

### corundum-gui

```
crates/corundum-gui/
  Cargo.toml
  assets/icons/*.svg, LICENSE-Lucide
  src/lib.rs     pub use egui; pub mod fonts; pub mod icons; mod shell; pub use shell::*;
  src/fonts.rs   moved unchanged from sapphire-framework-gui
  src/icons.rs   the loader, `image`, the icon set
  src/shell.rs   Shell, Destination, Action, COMPACT_WIDTH
```

**`fonts`** — `sapphire-framework-gui/src/fonts.rs` moved as is (`system_cjk_font`,
`add_system_cjk_fallback`, `install_system_cjk_fallback`, `PREFERRED_FAMILIES`,
`SystemFont`), with its tests.

**`icons`** — `install_loaders(ctx)`, and `image(ui, src, size)` made `pub` (it is
`pub(crate)` today; apps draw their own icons the same way). One function per icon,
returning `egui::ImageSource<'static>`. The set is the generic ones the apps already use,
which asteria needs as well:

`settings, arrow_left, x, plus, trash_2, pencil_line, refresh_cw, info, ellipsis,
chevron_right, chevron_down, circle_check, circle_x, folder, file_text`

The SVGs are copied from sapphire-framework-gui and sapphire-journal (the same Lucide
files, stroked in white so `tint` colours them). App-specific icons (wallet, timer, …)
stay in the apps.

**`shell`** — the frame, knowing nothing about sapphire:

```rust
pub const COMPACT_WIDTH: f32 = 600.0;

pub struct Destination<D> { pub id: D, pub icon: ImageSource<'static>, pub label: String }
pub struct Action { pub id: &'static str, pub icon: ImageSource<'static>, pub label: String }

pub struct Shell<D> { … }

impl<D: Copy + PartialEq> Shell<D> {
    /// `destinations` on the rail, the first open.
    pub fn new(destinations: Vec<Destination<D>>) -> Shell<D>;
    /// Buttons under the destinations.
    pub fn with_actions(self, actions: Vec<Action>) -> Shell<D>;
    /// Destinations at the rail's foot (the bar's end, narrow): where an app puts Settings.
    pub fn with_footer(self, footer: Vec<Destination<D>>) -> Shell<D>;
    pub fn take_action(&mut self) -> Option<&'static str>;
    pub fn current(&self) -> Option<D>;   // None only when there are no destinations at all
    pub fn open(&mut self, id: D);

    /// Draw the frame and the open destination.
    pub fn ui(&mut self, ui: &mut Ui, content: impl FnOnce(&mut Ui, D));
    /// As `ui`, with `foot` drawn next to the footer (a status dot, …). If `foot` returns
    /// a destination, it is opened.
    pub fn ui_with_foot(
        &mut self,
        ui: &mut Ui,
        foot: impl FnOnce(&mut Ui) -> Option<D>,
        content: impl FnOnce(&mut Ui, D),
    );
}
```

Layout is today's: wide, a 48-point rail on the left (destinations, a separator, actions;
footer and foot slot bottom-up); narrower than `COMPACT_WIDTH`, a bottom bar with one cell
per destination and action, and a last cell holding the footer and the foot slot. The width
decides, not the platform. `ui` installs the image loaders on its first frame, as today.

The first destination opens; with no destinations, the first footer destination does
(sapphire's "Settings is all there is").

### sapphire-framework-gui

- `Cargo.toml`: depends on `corundum-gui = { version = "0.1.0", path = "../corundum-gui" }`;
  drops `fontdb` and `ttf-parser`; keeps `egui` / `egui_extras` (its views still use them).
- `fonts`: `pub use corundum_gui::fonts;`. `icons`: `pub use corundum_gui::icons;`, its own
  SVGs and `LICENSE-Lucide` removed.
- `COMPACT_WIDTH`, `Destination`, `Action`: re-exported from corundum-gui; `settings.rs`
  imports `COMPACT_WIDTH` from there.
- `Shell<D>` keeps its signature (`new(destinations, settings)`, `with_actions`,
  `take_action`, `current`, `open`, `open_settings`, `settings`, `ui`) and becomes:

  ```rust
  enum Open<D> { Screen(D), Settings }
  pub struct Shell<D> { frame: corundum_gui::Shell<Open<D>>, settings: Settings }
  ```

  The app's destinations are mapped to `Open::Screen`; Settings is the one footer
  destination (`icons::settings()`, "Settings"). `ui` polls `settings`, draws the
  `Health::Problem` status panel on top, then calls `frame.ui_with_foot` with the status
  dot as the foot (clicking it calls `settings.open_about()` and returns
  `Some(Open::Settings)`), and draws `settings.ui` for `Open::Settings`.

The status-panel order matters: today the rail/bar is added before the top status panel;
the wrapper keeps that order by drawing the status panel inside `content` before the
screen, so the panels nest as they do now.

### Releases

`release-plz.toml` gets a plain `[[package]] name = "corundum-gui"` (no `version_group`),
like the API crates. As a new, independently versioned crate it is published at 0.1.0 when
this PR merges; sapphire-framework-gui's next release depends on it. Its CHANGELOG follows
the API crates' practice.

### Mobile CI

`.github/workflows/mobile-ci.yml`, on `pull_request` with
`paths: [crates/corundum-gui/**, .github/workflows/mobile-ci.yml]`:

- **ios** (`macos-latest`): `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`;
  `cargo clippy -p corundum-gui --target <each> --locked -- -D warnings`.
- **android** (`ubuntu-latest`, the runner's preinstalled NDK): `cargo install cargo-ndk`;
  `rustup target add aarch64-linux-android x86_64-linux-android`;
  `cargo ndk -t arm64-v8a -t x86_64 clippy -p corundum-gui --locked -- -D warnings`.

Build only: running tests on a simulator or emulator is later work. The crate's tests run
on the desktops in the existing CI (`--workspace`). The jobs are not required checks, so
the path filter never leaves a required check pending.

## Testing

- corundum-gui: `Shell`'s current tests, rewritten without `Settings` (opens the first
  destination, both layouts, actions taken once, footer opens, foot slot returning a
  destination opens it, footer-only shell); the fonts tests moved; one test that every
  icon decodes through the SVG loader.
- sapphire-framework-gui: its `Shell` tests kept as they are (they now exercise the
  wrapper), plus one that clicking the status dot opens About.
- `cargo publish --workspace --dry-run --all-features` before opening the PR.
- The mobile jobs green on the PR.

## Known gaps

- ~~`fonts::PREFERRED_FAMILIES` has no iOS or Android branch~~ — fixed by #321: iOS asks
  CoreText for the font files (its sandbox refuses to list the font directories), Android
  reads `/system/fonts` and `/product/fonts`; each has its own family list, and the tests
  run on a simulator and an emulator. Still unchecked: whether an app on a real iPhone may
  open the files CoreText names (the simulator has no sandbox; rust-fontconfig reports it
  can), and the family names on device makers' own Android builds.
