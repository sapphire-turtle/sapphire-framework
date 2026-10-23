# corundum-gui Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create `crates/corundum-gui` holding the fonts, the Lucide icons and a sapphire-free `Shell`, switch sapphire-framework-gui onto it without changing its public API, and build corundum-gui for iOS and Android in CI.

**Architecture:** corundum-gui depends on egui only. Its `Shell<D>` knows destinations, actions, footer destinations and a foot slot; sapphire-framework-gui's `Shell<D>` wraps `corundum_gui::Shell<Open<D>>`, putting Settings in the footer and the health dot in the foot slot. sapphire-framework-gui re-exports corundum-gui's `fonts`, `icons`, `Destination`, `Action`, `COMPACT_WIDTH`.

**Tech Stack:** Rust 1.99.0 (rust-toolchain.toml), egui / egui_extras 0.36 (`svg`), fontdb 0.24, ttf-parser 0.25, release-plz, GitHub Actions, cargo-ndk.

**Spec:** `docs/superpowers/specs/2026-10-22-corundum-gui-design.md`

## Global Constraints

- Work in the worktree `../sapphire-framework-wt-corundum`, branch `feat/corundum-gui`. Set `CARGO_TARGET_DIR` to a directory on disk (not under `/tmp`, which is RAM tmpfs).
- Never `git add -A` or `git stash`; add files by path.
- corundum-gui: `license = "MIT OR Apache-2.0"`, `version = "0.1.0"` (not `version.workspace`), edition / repository from the workspace.
- corundum-gui dependencies: `egui = "0.36"`, `egui_extras = { version = "0.36", default-features = false, features = ["svg"] }`, `fontdb = "0.24"`, `ttf-parser = "0.25"`. Nothing else, no features, no eframe.
- `pub use egui;` at corundum-gui's root.
- corundum-gui is in `release-plz.toml` with no `version_group`.
- sapphire-framework-gui's public API is unchanged: every item exported from `src/lib.rs` today still resolves at the same path with the same signature.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` and `cargo fmt --all -- --check` pass after every task.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` and reference `(#318)`.

## Review Focus

1. A narrow window (< 600 pt) with a footer: the footer destination and the foot slot both sit in the bar's last cell and stay clickable — tested in Task 2 (`footer_and_foot_are_in_both_layouts`).
2. The health dot clicked while an app screen is open: Settings opens *on About in the same frame*, not one frame later — tested in Task 3 (`clicking_the_dot_opens_about`).
3. A `Health::Problem` status panel still sits above the screen and does not displace the rail — covered by keeping today's draw order (rail first, status panel inside content); checked by rendering both sizes in Task 3's kept tests with no panic.
4. An app calling `sapphire_framework_gui::icons::settings()` / `fonts::install_system_cjk_fallback` keeps compiling — Task 3 Step 5 builds the doc examples and `app/mod.rs`, which use these paths.
5. An icon SVG that egui_extras cannot parse would only fail at runtime on a user's screen — tested in Task 1 (`every_icon_decodes`).

---

### Task 1: The crate, its fonts and its icons

**Files:**
- Create: `crates/corundum-gui/Cargo.toml`, `crates/corundum-gui/src/lib.rs`, `crates/corundum-gui/src/icons.rs`
- Move: `crates/sapphire-framework-gui/src/fonts.rs` → `crates/corundum-gui/src/fonts.rs` (copy now; the original is deleted in Task 3)
- Create: `crates/corundum-gui/assets/icons/{settings,arrow-left,x,plus,trash-2,pencil-line,refresh-cw,info,ellipsis,chevron-right,chevron-down,circle-check,circle-x,folder,file-text}.svg` and `LICENSE-Lucide`
- Modify: root `Cargo.toml` (`members`), `release-plz.toml`

**Interfaces:**
- Produces: `corundum_gui::fonts::*` (unchanged from today's `fonts.rs`); `corundum_gui::icons::{install_loaders(ctx: &egui::Context), image(ui: &egui::Ui, src: egui::ImageSource<'static>, size: f32) -> egui::Image<'static>}` and one `pub fn <name>() -> egui::ImageSource<'static>` per icon, named with underscores (`arrow_left`, `trash_2`, `pencil_line`, `refresh_cw`, `chevron_right`, `chevron_down`, `circle_check`, `circle_x`, `file_text`, …).

- [ ] **Step 1: Scaffold.** Add `"crates/corundum-gui"` to the workspace `members` (before `sapphire-framework-gui`). Write `Cargo.toml` per Global Constraints, with `description = "The look shared by sapphire and asteria apps: egui shell, system fonts, Lucide icons"`, `keywords = ["gui", "egui", "icons", "fonts"]`, `categories = ["gui"]`. `lib.rs`: a module doc saying what the crate is for (sapphire and asteria share it; egui only), `pub use egui;`, `pub mod fonts; pub mod icons;`. In `release-plz.toml`, after the `sapphire-framework-server-api` entry, add a `[[package]] name = "corundum-gui"` with a comment that it is shared with asteria and so is versioned on its own.

- [ ] **Step 2: Copy the assets.** `settings.svg`, `arrow-left.svg`, `LICENSE-Lucide` from `crates/sapphire-framework-gui/assets/icons/`; `folder.svg` via `git -C ../sapphire-sync show origin/main:desktop/assets/icons/folder.svg`; the rest via `git -C ../sapphire-journal show origin/main:desktop/assets/icons/<name>.svg`. Check each has `stroke="white"` or `stroke="#fff"`/`currentColor` as the existing two do; make them match the existing two.

- [ ] **Step 3: Copy `fonts.rs`** unchanged (its tests come with it).

- [ ] **Step 4: Write the failing icon test** in `icons.rs`:

```rust
#[test]
fn every_icon_decodes() {
    let ctx = egui::Context::default();
    install_loaders(&ctx);
    for (name, src) in ALL {   // a #[cfg(test)] const/fn listing all 15 (name, fn) pairs
        let egui::ImageSource::Bytes { uri, bytes } = src() else { panic!("{name}") };
        ctx.include_bytes(uri.clone(), bytes);
        let poll = ctx.try_load_image(&uri, egui::SizeHint::Size { width: 20, height: 20, maintain_aspect_ratio: true });
        assert!(matches!(poll, Ok(egui::load::ImagePoll::Ready { .. })), "{name}: {poll:?}");
    }
}
```

- [ ] **Step 5: Run it, see it fail** — `cargo test -p corundum-gui every_icon_decodes` → compile error (no icon fns).

- [ ] **Step 6: Implement `icons.rs`** from today's `sapphire-framework-gui/src/icons.rs`: same module doc (drop the "the components draw" wording for "the shared icon set"), same `icon!` macro, `image` made `pub`, one fn per icon with a one-line doc of its usual use.

- [ ] **Step 7: Verify** — `cargo test -p corundum-gui` passes (fonts tests + `every_icon_decodes`); clippy and fmt clean.

- [ ] **Step 8: Commit** — `feat(corundum-gui): new crate with the system fonts and the Lucide icons (#318)`.

### Task 2: corundum-gui's Shell

**Files:**
- Create: `crates/corundum-gui/src/shell.rs`, `crates/corundum-gui/src/test_support.rs` (`#[cfg(test)]`, the `render` helper copied from sapphire-framework-gui's `test_support.rs`)
- Modify: `crates/corundum-gui/src/lib.rs` (`mod shell; pub use shell::{Action, COMPACT_WIDTH, Destination, Shell};`)

**Interfaces:**
- Consumes: `crate::icons::{install_loaders, image, settings, arrow_left}`.
- Produces (exactly as in the spec):

```rust
pub const COMPACT_WIDTH: f32 = 600.0;
pub struct Destination<D> { pub id: D, pub icon: egui::ImageSource<'static>, pub label: String }
impl<D> Destination<D> { pub fn new(id: D, icon: egui::ImageSource<'static>, label: impl Into<String>) -> Self }
pub struct Action { pub id: &'static str, pub icon: egui::ImageSource<'static>, pub label: String }
impl Action { pub fn new(id: &'static str, icon: egui::ImageSource<'static>, label: impl Into<String>) -> Self }
impl<D: Copy + PartialEq> Shell<D> {
    pub fn new(destinations: Vec<Destination<D>>) -> Shell<D>;
    pub fn with_actions(self, actions: Vec<Action>) -> Shell<D>;
    pub fn with_footer(self, footer: Vec<Destination<D>>) -> Shell<D>;
    pub fn take_action(&mut self) -> Option<&'static str>;
    pub fn current(&self) -> Option<D>;
    pub fn open(&mut self, id: D);
    pub fn ui(&mut self, ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui, D));
    pub fn ui_with_foot(&mut self, ui: &mut egui::Ui,
        foot: impl FnOnce(&mut egui::Ui) -> Option<D>,
        content: impl FnOnce(&mut egui::Ui, D));
}
```

- [ ] **Step 1: Write the failing tests** in `shell.rs`, porting today's sapphire `shell.rs` tests (no `Settings`, no tokio). `Screen { Entries, Summary, Prefs }`; `shell()` = `Shell::new([Entries, Summary])`. `#[cfg(test)]` rect fields `action_rects` and `footer_rects` (and `foot_rect` for the slot's allocated rect) let tests click.
  - `it_opens_on_the_first_screen_and_draws_it` — as today; `current() == Some(Entries)`; both sizes `[900,600]`, `[400,700]` draw `Entries`.
  - `actions_are_drawn_in_both_layouts_and_report_nothing_unclicked` — as today.
  - `a_clicked_action_is_taken_once_and_opens_nothing` — as today.
  - `footer_and_foot_are_in_both_layouts` — `.with_footer([Prefs])`, `ui_with_foot` whose foot allocates a 12×12 rect; in both sizes, `footer_rects[0].is_positive()` and `foot_rect.is_positive()`.
  - `clicking_a_footer_destination_opens_it` — click `footer_rects[0]` (same 5-frame pattern as the action test) → `current() == Some(Prefs)`, and the next `ui` calls `content` with `Prefs`.
  - `a_destination_the_foot_returns_is_opened` — foot returns `Some(Summary)` → after one frame `current() == Some(Summary)`.
  - `with_only_a_footer_it_opens_on_the_footer` — `Shell::new(vec![]).with_footer([Prefs])` → `current() == Some(Prefs)`.
  - `with_nothing_there_is_nothing_to_draw` — `Shell::<Screen>::new(vec![])`: `current() == None` and `ui` renders without calling `content` (content panics).

- [ ] **Step 2: Run, see them fail** — `cargo test -p corundum-gui shell` → compile errors.

- [ ] **Step 3: Implement `shell.rs`** by generalizing today's sapphire `shell.rs`: `open: Option<D>` replaces `Open<D>`; `footer: Vec<Destination<D>>`; `ui` is `ui_with_foot(ui, |_| None, content)`. Rail: destinations, separator + actions (as today), then bottom-up: `add_space(4.0)`, footer destinations in order, then the foot slot. Bar: `cells = destinations + actions + 1`; the last cell is `ui.horizontal` with the footer destinations then the foot. If `foot` returns `Some(id)`, set `open = Some(id)` before calling `content`. With `open == None`, `content` is not called. Install the loaders on the first frame, as today. Keep the module doc and the `Shell` doc example (rewritten with `corundum_gui::{Destination, Shell, icons}`, no Settings).

- [ ] **Step 4: Verify** — `cargo test -p corundum-gui` passes; clippy, fmt, `cargo test --doc -p corundum-gui` clean.

- [ ] **Step 5: Commit** — `feat(corundum-gui): Shell, the rail and bottom bar without sapphire's Settings (#318)`.

### Task 3: sapphire-framework-gui on corundum-gui

**Files:**
- Modify: `crates/sapphire-framework-gui/Cargo.toml`, `src/lib.rs`, `src/shell.rs`, `src/settings.rs`, `src/views/files/view.rs`
- Delete: `crates/sapphire-framework-gui/src/fonts.rs`, `src/icons.rs`, `assets/icons/` (whole directory)

**Interfaces:**
- Consumes: Task 1 and Task 2's Produces.
- Produces: unchanged public API of sapphire-framework-gui.

- [ ] **Step 1: Write the failing test** in `src/shell.rs` (keeping all its existing tests as they are):

```rust
#[test]
fn clicking_the_dot_opens_about() {
    // shell() as today; record the dot's rect under #[cfg(test)] as `dot_rect`.
    // Two empty frames at [900, 600], then move / press / release on dot_rect.center().
    assert_eq!(shell.current(), None);              // Settings is open
    assert!(shell.settings.about_open());           // on About, in the frame of the click
}
```

Add `#[cfg(test)] pub(crate) fn about_open(&self) -> bool { self.page == Some(Page::About) }` to `Settings`. "In the frame of the click": the test checks `about_open()` right after the release frame, and the wrapper must call `open_about()` before `settings.ui` in that same frame (see Step 4).

- [ ] **Step 2: Run, see it fail** — `cargo test -p sapphire-framework-gui clicking_the_dot` → `dot_rect` / `about_open` missing.

- [ ] **Step 3: Dependencies and re-exports.** Cargo.toml: add `corundum-gui = { version = "0.1.0", path = "../corundum-gui" }`; remove `fontdb`, `ttf-parser`. Keep `egui`, `egui_extras`. `lib.rs`: `pub use corundum_gui::{fonts, icons};` replacing `pub mod fonts; pub mod icons;`; `pub use corundum_gui::{Action, COMPACT_WIDTH, Destination};` and `pub use shell::Shell;`; `pub use settings::{Settings, SettingsSection};`. Delete `COMPACT_WIDTH` from `settings.rs` and import it from `corundum_gui`; same in `views/files/view.rs`. `crate::icons::image(…)` call sites keep working via the re-export. Delete `fonts.rs`, `icons.rs`, `assets/icons/`.

- [ ] **Step 4: Rewrite `shell.rs` as the wrapper.** Fields: `frame: corundum_gui::Shell<Open<D>>`, `settings: Settings`, `#[cfg(test)] dot_rect: egui::Rect`. Keep `enum Open<D> { Screen(D), Settings }` (derive `Clone, Copy, PartialEq, Eq, Debug`). `new(destinations, settings)`: call `settings.hosted()`; map each `Destination<D>` to `Destination<Open<D>>` with `Open::Screen`; footer = `[Destination::new(Open::Settings, icons::settings(), "Settings")]`. `with_actions` / `take_action` delegate. `current()` maps `Some(Open::Screen(d))` → `Some(d)`, otherwise `None`. `open(id)` / `open_settings()` delegate with `Open::Screen(id)` / `Open::Settings`. `ui`:
  1. `self.settings.poll(); let health = self.settings.health();`
  2. `let about = Cell::new(false);`
  3. `frame.ui_with_foot(ui, foot, content)` where `foot` draws the dot (today's `status_dot_ui` body as a free fn `status_dot(ui, health) -> egui::Response`; record its rect under `cfg(test)`) and on click sets `about` and returns `Some(Open::Settings)`; `content` first draws `egui::Panel::top("shell_status")` with `settings.status_ui` when `health == Health::Problem`, then for `Open::Screen(d)` calls the app's `content(ui, d)`, and for `Open::Settings` calls `settings.open_about()` if `about.get()` and then `settings.ui(ui)`.
  Remove `icon_button`, `RAIL_WIDTH`, the loader flag and the `Destination` / `Action` definitions. Keep the module and `Shell` docs (the doc example still compiles: same paths).

- [ ] **Step 5: Verify** — `cargo test -p sapphire-framework-gui --all-features` (all old shell tests + the new one + doc tests) passes; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `cargo check -p sapphire-framework --no-default-features --features eframe --locked`; `cargo fmt --all -- --check`.

- [ ] **Step 6: Commit** — `refactor(gui): build Shell, fonts and icons on corundum-gui (#318)`.

### Task 4: Mobile CI, changelog, release check

**Files:**
- Create: `.github/workflows/mobile-ci.yml`
- Modify: `CHANGELOG.md` (`## [Unreleased]` → `### Added` / `### Changed`)

- [ ] **Step 1: Write `mobile-ci.yml`** per the spec's "Mobile CI": `on: pull_request: paths: [crates/corundum-gui/**, .github/workflows/mobile-ci.yml]`; a header comment on why (corundum-gui is shared with asteria, which ships on phones; build only; not a required check so the path filter never leaves one pending). Job `ios` on `macos-latest`: checkout `actions/checkout@v7`, `cargo --version` step, `Swatinem/rust-cache@v2`, `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`, then for each target `cargo clippy -p corundum-gui --target "$t" --locked -- -D warnings`. Job `android` on `ubuntu-latest`: same first steps, `rustup target add aarch64-linux-android x86_64-linux-android`, `cargo install cargo-ndk --locked`, `cargo ndk -t arm64-v8a -t x86_64 clippy -p corundum-gui --locked -- -D warnings` (the runner's `ANDROID_NDK_HOME` / `ANDROID_NDK_LATEST_HOME` is preinstalled; export `ANDROID_NDK_HOME=$ANDROID_NDK_LATEST_HOME` if cargo-ndk needs it).

- [ ] **Step 2: Check locally what can be checked** — `rustup target add aarch64-linux-android` and, if an NDK is available locally, the android command; otherwise `cargo check -p corundum-gui --target aarch64-linux-android` (no linking needed for an rlib). Note in the PR body which targets were only checked in CI.

- [ ] **Step 3: CHANGELOG.** Under `## [Unreleased]`: Added — new crate `corundum-gui` 0.1.0 (MIT OR Apache-2.0, shared with asteria): `Shell` with footer destinations and a foot slot, the system CJK fallback font, Lucide icon set and loader; iOS / Android build CI for it. Changed — `sapphire-framework-gui` builds its `Shell`, `fonts`, `icons` on corundum-gui; public API unchanged; `fontdb` / `ttf-parser` dependencies moved.

- [ ] **Step 4: Release dry run** — `cargo publish --workspace --dry-run --all-features --locked` succeeds (corundum-gui is new, so sapphire-framework-gui verifies against the local one; that is expected).

- [ ] **Step 5: Commit** — `ci: build corundum-gui for iOS and Android (#318)`.

- [ ] **Step 6: Open the follow-up issue** for the mobile font lookup (spec "Known gaps"): title `fix(corundum-gui): find system fonts on iOS and Android`, linking #318 and the spec. Ask the user before creating it.
