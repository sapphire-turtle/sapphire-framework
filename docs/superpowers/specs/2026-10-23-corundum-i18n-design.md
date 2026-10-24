# corundum-gui: translations

Issue: #329. Follows `2026-10-22-corundum-gui-design.md` and `2026-10-23-corundum-theme-design.md`.

## Why

The strings on screen in corundum-gui and sapphire-framework-gui are English written into
the code ("Appearance", "Light", "Sync", "About", "syncing · 2 peers"). The sapphire apps
and asteria should be able to show Japanese and other languages. The fonts are ready —
`fonts` loads a system CJK font as the fallback — but there is nothing to look a string up
in.

## Scope

In: an `i18n` module in corundum-gui (lookup, the language setting, the system language, a
language picker); `Shell` labels as message ids; the theme widget's strings; every string
sapphire-framework-gui builds for the screen; the language as a bridge setting
(`gui.locale`) edited on its Appearance page; English and Japanese.

Out: languages beyond English and Japanese; translation files loaded at run time; number,
date and size formatting by locale; following a change of the system language while the
app runs; the apps' own strings (a PR per app repository, after the release).

## Decisions

1. **Fluent.** `fluent-bundle` with `.ftl` files. It has CLDR plural rules ("1 peer" /
   "2 peers" from one message), named arguments so a translation can put them in any
   order, and it is pure Rust, so it builds for Android and iOS as it is. Its keys are not
   checked at compile time; a test checks them instead (see Testing).
2. **Embedded per crate, added by the app.** Each crate embeds its own
   `locales/<lang>/<crate>.ftl` with `include_str!`. An app registers its own files the
   same way; a key registered later replaces the same key registered earlier, so an app
   can also reword a framework string. Keys carry the crate's prefix (`corundum-`,
   `sapphire-`; the app picks its own) so they do not collide by accident. Nothing is read
   from disk at run time.
3. **The language lives in the egui context**, as the theme does. `install` stores the
   built bundles in `ctx.data`; lookups read them every frame. Changing the language
   takes effect on the next frame, without a restart. For that, a label held across
   frames is held as a message id, not as text.
4. **The crate stores nothing.** corundum-gui only hands `LanguageSettings` in and out; the
   `serde` feature derives the traits, as for `ThemeSettings`. In the sapphire apps the
   bridge stores the language (see "Saving the language").
5. **Missing means English.** A lookup tries the chosen language, then English, then
   shows the message id itself.
6. **The system language from `sys-locale`, and from Java on Android.** `sys-locale`
   reads the OS's preferred languages on Linux, Windows, macOS and iOS. On Android it
   reads system properties, which miss per-app languages and are not always readable, so
   there the list comes from `android.os.LocaleList.getDefault()` over JNI.

## Design

### Dependencies (corundum-gui)

- `fluent-bundle = "0.16"`, `fluent-langneg = "0.14"`, `unic-langid = "0.9"`,
  `sys-locale = "0.3"`.
- `fluent-syntax = "0.12"`, for `i18n::check` (the entries of the parsed files; already a
  dependency of `fluent-bundle`).
- Android only: `jni = "0.21"`, `ndk-context = "0.1"` (the versions sapphire-framework-gui
  already uses).

### Settings (`i18n/mod.rs`)

```rust
/// What the user chose: what the app saves, and hands to `install` at start.
#[derive(Clone, PartialEq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(default))]
pub struct LanguageSettings {
    pub language: Language,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Language {
    /// The system's preferred languages, in order.
    #[default]
    System,
    /// A BCP 47 tag: "ja", "en-GB".
    Explicit(String),
}
```

### Resources

```rust
/// One crate's translations: `(language tag, .ftl source)` pairs, embedded.
pub struct Resources {
    pub name: &'static str,
    pub files: &'static [(&'static str, &'static str)],
}

/// corundum-gui's own; always registered first.
pub(crate) const CORUNDUM: Resources = Resources {
    name: "corundum-gui",
    files: &[
        ("en", include_str!("../../locales/en/corundum.ftl")),
        ("ja", include_str!("../../locales/ja/corundum.ftl")),
    ],
};

/// Add `resources` after those registered before; a key in them replaces the same key
/// from earlier ones. A name already registered is ignored, so it is cheap to call every
/// frame. If a language is installed, it is rebuilt with them.
pub fn register(ctx: &egui::Context, resources: Resources);
```

The registered list is kept in `ctx.data`. corundum-gui's own resources are always first,
whether or not anything was registered.

### Install and lookup

```rust
/// Use `settings` in `ctx`: choose the language, build its bundle and English's, and store
/// both. Call it at start and whenever the settings change.
pub fn install(ctx: &egui::Context, settings: &LanguageSettings);
pub fn is_installed(ctx: &egui::Context) -> bool;
/// The language chosen by the last `install` ("en" before any).
pub fn current(ctx: &egui::Context) -> unic_langid::LanguageIdentifier;
/// The languages the registered resources have, English first.
pub fn available(ctx: &egui::Context) -> Vec<LanguageIdentifier>;
/// `lang`'s name in itself (`corundum-language-name`): "English", "日本語".
pub fn name_of(ctx: &egui::Context, lang: &LanguageIdentifier) -> String;

pub fn tr(ctx: &egui::Context, id: &str) -> String;
pub fn tr_args(ctx: &egui::Context, id: &str, args: &[(&str, Arg)]) -> String;

/// tr!(ui, "corundum-theme-appearance")
/// tr!(ui, "sapphire-version", version = app.version)
/// The first argument is anything with a context: `&Ui` or `&Context`.
macro_rules! tr { … }
```

- **Choosing.** `Language::System` takes `system_languages()`; `Explicit(tag)` takes
  `[tag]`. `fluent_langneg::negotiate_languages` (Filtering) matches them against
  `available`, with `en` as the default. An unparsable tag counts as no match.
  fluent-langneg 0.14 uses `icu_locid`, not `unic-langid`, so `choose` converts between
  the two as tag strings.
- **Bundles.** `fluent_bundle::concurrent::FluentBundle` (it is `Send + Sync`, which
  `ctx.data` needs), one for the chosen language and one for English, each with every
  registered resource for that language added in order with `add_resource_overriding`.
  `set_use_isolating(false)`: the bidi isolation marks Fluent puts around arguments draw
  as boxes in egui. Stored behind an `Arc`, as the theme is.
- **Lookup.** Chosen bundle, then English; then the id itself. A miss is logged once per
  id at `warn` (a set of ids already reported, in the stored state). A formatting error
  (a missing argument, say) is logged the same way and the formatted text is still shown.
- **A broken `.ftl`.** `FluentResource::try_new` returns what it could parse and the
  errors; the errors are logged at `warn` and the parsed part is used. For the embedded
  files a test makes this unreachable.

### Messages built outside egui

sapphire-framework-gui's `views/model.rs` makes its decisions in plain functions tested
without egui. Those that return text return a `Message` instead, resolved by the view:

```rust
#[derive(Clone, PartialEq, Debug)]
pub struct Message {
    pub id: &'static str,
    pub args: Vec<(&'static str, Arg)>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Arg {
    Str(String),
    Num(f64),
}
// From<String>, From<&str>, and From for the integer types and f64.

impl Message {
    pub fn new(id: &'static str) -> Message;
    pub fn arg(self, name: &'static str, value: impl Into<Arg>) -> Message;
    pub fn text(&self, ctx: &egui::Context) -> String;
}
```

### The system language (`i18n/system.rs`)

`system_languages() -> Vec<String>` (public):

- Android: `LocaleList.getDefault().toLanguageTags()` (comma-separated) through the JVM
  from `ndk_context::android_context()`. On any JNI failure, `sys-locale`.
- Everywhere else: `sys_locale::get_locales()`.

It is read once per `install`. A system language change while the app runs shows after
the next `install` or a restart.

### Language picker (`i18n/ui.rs`)

```rust
/// A "Language" row's control: System, then each available language under its own name.
/// A change is installed at once; `true` means save `settings`.
pub fn settings_ui(ui: &mut egui::Ui, settings: &mut LanguageSettings) -> bool;
```

A combo box. Each language is shown by its `corundum-language-name` message, looked up
in that language's own bundle ("English", "日本語"). The System entry shows what it
resolves to now: "System (日本語)". Like the theme widget, it draws only the control; the
caller lays out the row label (`tr!(ui, "corundum-language")`).

### Shell

- On its first frame, as for the theme: if no language is installed, install
  `LanguageSettings::default()`.
- `Destination::new(id, icon, label)` and `Action::new(id, icon, label)` take `label` as a
  message id, `&'static str`; the hover text is `tr(ctx, label)` at draw time. This
  changes corundum-gui 0.1's API.

### Theme widget

Its row labels and choices ("Appearance", "Light", "Dark", "System", "Contrast", "Low",
"Medium", "High", "Accent hue", "Accent strength", "Preview", "Accent") become
`corundum-theme-*` messages.

### sapphire-framework-gui

- Every string drawn in `settings.rs`, `shell.rs`, `manager.rs` and
  `views/*` becomes a `tr!` call with a `sapphire-` key. Constant tables of labels
  (`SYNC_SCREENS`, `SWITCH_CHOICES`) hold message ids.
- `views/model.rs`: `Badge::label`, `move_target` and `new_target_ok`'s errors,
  `embedding_status` and the other text-returning functions return `Message` (or
  `Result<_, Message>`). `role_badges` and `auto_resolution` return message ids
  (`&'static str`), since they take no arguments. `views/files/model.rs` returns
  `Result<_, Message>` (`sapphire-files-symlink`, `-outside`, `-io-error`). Plurals made by hand (`peer{s}`) become Fluent
  selectors.
- `SettingsSection::label(&self)` becomes `label(&self, ctx: &egui::Context) -> String`,
  so an app can translate its section names. `with_app_sections`' heading stays a
  `String`: it is the app's name.
- The language is a bridge setting, `gui.locale`, edited on the Appearance page; see
  "Saving the language".
- Each public sapphire view registers sapphire-framework-gui's `Resources` when it draws,
  as do `Shell`, `Settings` and the manager, before the corundum `Shell` installs a
  language. `register` is idempotent by `Resources::name`, so this costs nothing after the
  first frame. `sapphire_framework_gui::RESOURCES` is public for an app that looks strings up
  before the first frame.
- The views keep their errors as `Option<Message>`, resolved when drawn. Backend text is
  wrapped in `sapphire-error-text = { $text }`, so the text is shown as it came.
- Files: `crates/sapphire-framework-gui/locales/{en,ja}/sapphire.ftl`.

### Saving the language

The language follows the theme's path (#327, `2026-10-23-theme-sync-design.md`): the bridge
stores it in the workgroup, a device may override it, and the GUI follows it through its
poll. The apps save nothing.

1. **The key** is `gui.locale` (`sapphire_bridge_api::settings::gui::LOCALE`; `THEME_KEYS`
   is unchanged), defined by the bridge beside the theme's keys: section `"Appearance"`,
   `scopes: [Workgroup]`, `overridable: true`, `Apply::Live`, `Process::Bridge`,
   `Kind::String { non_empty: true }`, default `"system"`. The bridge's `validate` does
   not look at it.
2. **The value**: `"system"` is `Language::System`; anything else is parsed as a language
   tag (`unic_langid`) and becomes `Language::Explicit` with the tag as it prints; a value
   that does not parse, or none, is `Language::System`. `language_settings(&GuiValues)`
   and `locale_entry(&LanguageSettings)` in `views/appearance.rs` convert, beside the
   theme's conversions.
3. **The poll** reads `gui.locale` after the theme's keys into the same `GuiValues`. A
   bridge that does not know it (`unknown setting`) leaves it out of the map rather than
   making `gui` `None`: the theme is still followed with an older bridge, and the language
   stays as installed.
4. **Following**: `Settings::follow_theme` (called by the shell every frame) installs
   `language_settings` of the snapshot when its `gui.locale` differs from the last one
   seen. The theme and the language are compared each on its own, so a change of one does
   not reinstall the other. A failed write on the page forgets both, so the next poll
   restores what is stored.
5. **The Appearance page** draws the language picker (`i18n::settings_ui`) in each scope
   tab, under the theme widget, with the theme's tab semantics: the Workgroup tab edits the
   workgroup's value; the Device tab shows the effective one, `language: from workgroup`
   (or `default`) when the device does not set it, and `Use the workgroup's` unsets
   `gui.locale` on the device with the theme's keys; on the Workgroup tab the override
   line names the language when the device sets it. A choice writes `gui.locale` alone to
   the tab's scope at once (`Command::ConfigWrite`) and installs the language that will
   show — the device's own still wins on the Workgroup tab. With no app server the page
   shows only part 4's `unavailable` line, without the theme widget or the picker; with no
   bridge, or one that does not know the theme's keys, both are drawn disabled under the
   reason; with a bridge that knows the theme but not `gui.locale`, the picker alone is
   disabled, with `this sapphire-bridge does not know the language setting; update it`.
   `Use the workgroup's` unsets only the keys the device sets, so it never names
   `gui.locale` to a bridge that does not know it (which would refuse the whole write).
   Writes asked for in the same frame (a theme drag ending with a language choice) go as
   one.
6. `Settings::with_language`, `Settings::take_language_changed` and
   `Shell::take_language_changed`, never released, are removed.

### Not translated

- Text that comes from the backend or the bridge: error messages, pause reasons,
  `EmbedNote`. The words around it are translated; it is inserted as an argument.
- Names and values: workspace, file and device names, paths, versions, ids, size units
  (B, KB, MB).
- `tracing` log messages.
- The client's own errors, which reach the views as backend text does:
  `client/exec.rs` (a command's failures), `client/conn.rs` (`the {app} server is not
  running`, `… did not answer within …`) and `mobile/android.rs` (`Android gave no
  internal data path`, which ends the app before any window).

## Testing

In corundum-gui, with helpers sapphire-framework-gui's tests use as well:

- Every embedded `.ftl` parses without errors.
- `en` and `ja` have the same keys, both ways; the failure lists the difference.
- Every id given to `tr!`, `Message::new`, `Destination::new`, `Action::new` and the label
  tables in a crate's `src` exists in its `en` file: a test scans the sources (no regex
  crate) for string literals beginning with the crate's prefix.
- Lookup falls back ja → en → id; a missing argument still yields text.
- `install` with another language changes `tr` on the next call.
- `Language::System` with a given system list (`["ja-JP", "en-US"]` → ja, `["fr"]` → en);
  the choosing is a function taking the list, so the test does not depend on the machine.
- A later `register` replaces an earlier key, and rebuilds an installed language.
- `Arg::Num` selects the plural (`one` / `other`) in English.
- `LanguageSettings` serde round trip.
- The picker renders and reports no change when nothing is clicked.
- The Shell installs the default language, and keeps one already installed.

In sapphire-framework-gui, the existing `model.rs` tests assert on `Message` ids and args
instead of English text; the view tests that look for text keep working with English
installed.

## Rollout

One PR, a commit per step:

1. corundum-gui: `i18n` (settings, resources, install, lookup, `Message`), the system
   language, `corundum.ftl` (en, ja).
2. corundum-gui: the language picker; the theme widget's strings; `Shell` labels as
   message ids.
3. sapphire-framework-gui: `sapphire.ftl` (en, ja), the views and `model.rs` moved over.
4. sapphire-framework-gui: the language as the bridge's `gui.locale`, on the Appearance
   page.
5. Breaking changes marked with `!` in the commit messages (corundum-gui's API change:
   labels are message ids); release-plz writes the CHANGELOGs.
