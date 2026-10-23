//! The translations: every string the user reads, looked up by id in the language the user
//! chose.
//!
//! The strings are [Fluent](https://projectfluent.org) messages, in one `.ftl` file per
//! language per crate, embedded as [`Resources`]. corundum-gui's own ([`CORUNDUM`]) are
//! always there; an app adds its own with [`register`], and a key in them replaces the
//! same key from those registered before.
//!
//! [`install`] chooses the language from [`LanguageSettings`] and keeps it in the egui
//! context, as [`theme::install`](crate::theme::install) keeps the theme. [`tr`] and
//! [`tr_args`] — or the [`tr!`](crate::tr!) macro — look a message up in it, then in English,
//! then give the id itself. A miss is logged once per id.
//!
//! Text decided outside egui, in plain functions, is a [`Message`]: an id and its
//! arguments, resolved by the view that draws it.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource};
pub use unic_langid::LanguageIdentifier;

pub mod check;
mod message;
mod system;

pub use message::{Arg, Message};
pub use system::system_languages;

/// What the user chose: what the app saves, and hands to [`install`] at start.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct LanguageSettings {
    pub language: Language,
}

/// Which language to show.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Language {
    /// The system's preferred languages, in order.
    #[default]
    System,
    /// A BCP 47 tag: `"ja"`, `"en-GB"`. One no file has, or that does not parse, gives
    /// English.
    Explicit(String),
}

/// One crate's translations: `(language tag, .ftl source)` pairs, embedded with
/// `include_str!`.
#[derive(Clone, Copy, Debug)]
pub struct Resources {
    /// Names the set: [`register`] adds a name once.
    pub name: &'static str,
    pub files: &'static [(&'static str, &'static str)],
}

/// corundum-gui's own; always registered first.
pub const CORUNDUM: Resources = Resources {
    name: "corundum-gui",
    files: &[
        ("en", include_str!("../../locales/en/corundum.ftl")),
        ("ja", include_str!("../../locales/ja/corundum.ftl")),
    ],
};

type Bundle = FluentBundle<FluentResource>;

/// The registered [`Resources`], in order; [`CORUNDUM`] first.
#[derive(Clone)]
struct Registered(Vec<Resources>);

/// What [`install`] built, or — before it — English alone, built on the first lookup.
#[derive(Clone)]
struct Installed {
    /// `None` before [`install`].
    settings: Option<LanguageSettings>,
    chosen: LanguageIdentifier,
    bundle: Arc<Bundle>,
    english: Arc<Bundle>,
    /// The ids already logged as missing or failing to format.
    reported: Arc<Mutex<HashSet<String>>>,
    /// Each language's name in itself, for [`name_of`], filled as asked for.
    names: Arc<Mutex<HashMap<LanguageIdentifier, String>>>,
}

fn registered_key() -> egui::Id {
    egui::Id::new("corundum_i18n_resources")
}

fn installed_key() -> egui::Id {
    egui::Id::new("corundum_i18n")
}

fn english() -> LanguageIdentifier {
    LanguageIdentifier::from_bytes(b"en").expect("\"en\" is a language tag")
}

/// The registered resources, [`CORUNDUM`] first whether or not anything was registered.
fn registered(ctx: &egui::Context) -> Vec<Resources> {
    ctx.data(|d| d.get_temp::<Registered>(registered_key()))
        .map_or_else(|| vec![CORUNDUM], |r| r.0)
}

/// Add `resources` after those registered before; a key in them replaces the same key
/// from earlier ones. A name already registered is ignored, so calling this every frame
/// is cheap. If a language is installed, it is rebuilt with them.
pub fn register(ctx: &egui::Context, resources: Resources) {
    let mut list = registered(ctx);
    if list.iter().any(|r| r.name == resources.name) {
        return;
    }
    list.push(resources);
    ctx.data_mut(|d| d.insert_temp(registered_key(), Registered(list)));
    match ctx.data(|d| d.get_temp::<Installed>(installed_key())) {
        Some(Installed {
            settings: Some(settings),
            ..
        }) => install(ctx, &settings),
        // English alone, built for a lookup before `install`: built again on the next.
        Some(_) => ctx.data_mut(|d| d.remove::<Installed>(installed_key())),
        None => {}
    }
}

/// Use `settings` in `ctx`: choose the language, build its messages and English's, and
/// store both. Call it at start and whenever the settings change.
pub fn install(ctx: &egui::Context, settings: &LanguageSettings) {
    let requested = match &settings.language {
        Language::System => system_languages(),
        Language::Explicit(tag) => vec![tag.clone()],
    };
    let chosen = choose(&requested, &available(ctx));
    let installed = build(ctx, Some(settings.clone()), chosen);
    ctx.data_mut(|d| d.insert_temp(installed_key(), installed));
}

/// Whether [`install`] has been called on `ctx`.
pub fn is_installed(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Installed>(installed_key()))
        .is_some_and(|i| i.settings.is_some())
}

/// The language chosen by the last [`install`]: English before any.
pub fn current(ctx: &egui::Context) -> LanguageIdentifier {
    ctx.data(|d| d.get_temp::<Installed>(installed_key()))
        .map_or_else(english, |i| i.chosen)
}

/// The languages the registered resources have: English first, then the rest by tag.
pub fn available(ctx: &egui::Context) -> Vec<LanguageIdentifier> {
    let mut langs: Vec<LanguageIdentifier> = registered(ctx)
        .iter()
        .flat_map(|r| r.files.iter())
        .filter_map(|(tag, _)| tag.parse().ok())
        .collect();
    let en = english();
    langs.sort_by_key(|l| (*l != en, l.to_string()));
    langs.dedup();
    langs
}

/// The message `id`, in the installed language. See [`tr_args`].
pub fn tr(ctx: &egui::Context, id: &str) -> String {
    tr_args(ctx, id, &[])
}

/// The message `id` with `args`, in the installed language; else in English; else `id`
/// itself. A miss, or an error formatting it (a missing argument, say), is logged once per
/// id; the text formatted despite the error is still given.
pub fn tr_args(ctx: &egui::Context, id: &str, args: &[(&str, Arg)]) -> String {
    let installed = state(ctx);
    let args = fluent_args(args);
    for bundle in [&installed.bundle, &installed.english] {
        if let Some(text) = format(&installed, bundle, id, args.as_ref()) {
            return text;
        }
    }
    report(&installed, id, || {
        tracing::warn!(id, "no message with this id in any language");
    });
    id.to_owned()
}

/// `lang`'s name in itself — `corundum-language-name` from its own resources ("English",
/// "日本語") — for a language picker. The tag, if it has none.
pub fn name_of(ctx: &egui::Context, lang: &LanguageIdentifier) -> String {
    let installed = state(ctx);
    let mut names = installed.names.lock().unwrap_or_else(|e| e.into_inner());
    names
        .entry(lang.clone())
        .or_insert_with(|| {
            let bundle = bundle_for(&registered(ctx), lang);
            bundle
                .get_message("corundum-language-name")
                .and_then(|m| m.value())
                .map(|pattern| {
                    let mut errors = Vec::new();
                    bundle
                        .format_pattern(pattern, None, &mut errors)
                        .into_owned()
                })
                .unwrap_or_else(|| lang.to_string())
        })
        .clone()
}

/// Anything [`tr!`](crate::tr!) can look strings up through: an [`egui::Ui`] or an
/// [`egui::Context`].
pub trait AsContext {
    fn as_context(&self) -> &egui::Context;
}

impl AsContext for egui::Context {
    fn as_context(&self) -> &egui::Context {
        self
    }
}

impl AsContext for egui::Ui {
    fn as_context(&self) -> &egui::Context {
        self.ctx()
    }
}

/// Look a message up: [`tr`](crate::i18n::tr) or, with arguments,
/// [`tr_args`](crate::i18n::tr_args).
///
/// The first argument is anything with a context ([`AsContext`](crate::i18n::AsContext)):
/// a `Ui` or a `Context`. Each argument after the id is `name = value`, the value anything
/// that converts into an [`Arg`](crate::i18n::Arg).
///
/// ```ignore
/// tr!(ui, "corundum-language")
/// tr!(ui, "sapphire-version", version = app.version)
/// ```
#[macro_export]
macro_rules! tr {
    ($c:expr, $id:expr $(,)?) => {{
        use $crate::i18n::AsContext as _;
        $crate::i18n::tr(($c).as_context(), $id)
    }};
    ($c:expr, $id:expr, $($name:ident = $value:expr),+ $(,)?) => {{
        use $crate::i18n::AsContext as _;
        $crate::i18n::tr_args(
            ($c).as_context(),
            $id,
            &[$((
                ::core::stringify!($name),
                ::core::convert::Into::<$crate::i18n::Arg>::into($value),
            )),+],
        )
    }};
}

/// The best of `available` for `requested`, English when none matches. A tag that does not
/// parse matches nothing.
pub(crate) fn choose(requested: &[String], available: &[LanguageIdentifier]) -> LanguageIdentifier {
    use fluent_langneg::{LanguageIdentifier as Tag, NegotiationStrategy, negotiate_languages};
    // fluent-langneg has its own identifier type; the two meet as tags.
    let requested: Vec<Tag> = requested
        .iter()
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse().ok())
        .collect();
    let offered: Vec<Tag> = available
        .iter()
        .filter_map(|l| l.to_string().parse().ok())
        .collect();
    let default: Tag = "en".parse().expect("\"en\" is a language tag");
    negotiate_languages(
        &requested,
        &offered,
        Some(&default),
        NegotiationStrategy::Filtering,
    )
    .first()
    .and_then(|tag| tag.to_string().parse().ok())
    .unwrap_or_else(english)
}

/// What [`install`] left, or English alone (stored for the next lookup) before it.
fn state(ctx: &egui::Context) -> Installed {
    if let Some(installed) = ctx.data(|d| d.get_temp::<Installed>(installed_key())) {
        return installed;
    }
    let installed = build(ctx, None, english());
    ctx.data_mut(|d| d.insert_temp(installed_key(), installed.clone()));
    installed
}

fn build(
    ctx: &egui::Context,
    settings: Option<LanguageSettings>,
    chosen: LanguageIdentifier,
) -> Installed {
    let resources = registered(ctx);
    let english = Arc::new(bundle_for(&resources, &english()));
    let bundle = if chosen == self::english() {
        english.clone()
    } else {
        Arc::new(bundle_for(&resources, &chosen))
    };
    Installed {
        settings,
        chosen,
        bundle,
        english,
        reported: Arc::default(),
        names: Arc::default(),
    }
}

/// `lang`'s messages from every resource that has them, later ones replacing earlier ones'.
/// A file that does not parse is logged and its parsed part used.
fn bundle_for(resources: &[Resources], lang: &LanguageIdentifier) -> Bundle {
    let mut bundle = Bundle::new_concurrent(vec![lang.clone()]);
    // Fluent wraps each argument in bidi isolation marks, which egui draws as boxes.
    bundle.set_use_isolating(false);
    for r in resources {
        for (tag, source) in r.files {
            if tag.parse::<LanguageIdentifier>().ok().as_ref() != Some(lang) {
                continue;
            }
            let resource = match FluentResource::try_new(source.to_string()) {
                Ok(resource) => resource,
                Err((resource, errors)) => {
                    for error in errors {
                        tracing::warn!(resources = r.name, tag, %error, "translation file error");
                    }
                    resource
                }
            };
            bundle.add_resource_overriding(resource);
        }
    }
    bundle
}

fn fluent_args<'a>(args: &'a [(&'a str, Arg)]) -> Option<FluentArgs<'a>> {
    if args.is_empty() {
        return None;
    }
    let mut fluent = FluentArgs::with_capacity(args.len());
    for (name, value) in args {
        fluent.set(*name, value.to_fluent());
    }
    Some(fluent)
}

/// `id` from `bundle`, or `None` if it has no such message (or one without a value).
fn format(
    installed: &Installed,
    bundle: &Bundle,
    id: &str,
    args: Option<&FluentArgs>,
) -> Option<String> {
    let pattern = bundle.get_message(id)?.value()?;
    let mut errors = Vec::new();
    let text = bundle
        .format_pattern(pattern, args, &mut errors)
        .into_owned();
    if !errors.is_empty() {
        report(installed, id, || {
            tracing::warn!(id, ?errors, "error formatting a message");
        });
    }
    Some(text)
}

/// Run `log` the first time `id` is reported.
fn report(installed: &Installed, id: &str, log: impl FnOnce()) {
    let mut reported = installed.reported.lock().unwrap_or_else(|e| e.into_inner());
    if reported.insert(id.to_owned()) {
        log();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang(tag: &str) -> LanguageIdentifier {
        tag.parse().unwrap()
    }

    fn explicit(tag: &str) -> LanguageSettings {
        LanguageSettings {
            language: Language::Explicit(tag.to_owned()),
        }
    }

    const TEST: Resources = Resources {
        name: "test",
        files: &[
            (
                "en",
                "only-en = Only English\n\
                 greet = Hello { $name }!\n\
                 peers = { $n ->\n    [one] one peer\n   *[other] { $n } peers\n}\n",
            ),
            ("ja", "greet = こんにちは { $name }!\n"),
        ],
    };

    #[test]
    fn choose_matches_regions_and_falls_back() {
        let available = [lang("en"), lang("ja")];
        let pick = |tags: &[&str]| {
            let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
            choose(&tags, &available)
        };
        assert_eq!(pick(&["ja-JP", "en-US"]), lang("ja"));
        assert_eq!(pick(&["en-US"]), lang("en"));
        assert_eq!(pick(&["fr"]), lang("en"));
        assert_eq!(pick(&["zh-Hant-TW"]), lang("en"));
        assert_eq!(pick(&[]), lang("en"));
        assert_eq!(pick(&["xx-!!", ""]), lang("en"));
    }

    #[test]
    fn explicit_and_garbage_tags() {
        let ctx = egui::Context::default();
        install(&ctx, &explicit("ja"));
        assert_eq!(current(&ctx), lang("ja"));
        assert_eq!(tr(&ctx, "corundum-language"), "言語");
        install(&ctx, &explicit("xx-!!"));
        assert_eq!(current(&ctx), lang("en"));
        assert_eq!(tr(&ctx, "corundum-language"), "Language");
        install(&ctx, &explicit(""));
        assert_eq!(current(&ctx), lang("en"));
    }

    #[test]
    fn falls_back_to_english_then_the_id() {
        let ctx = egui::Context::default();
        register(&ctx, TEST);
        install(&ctx, &explicit("ja"));
        assert_eq!(tr(&ctx, "only-en"), "Only English");
        assert_eq!(tr(&ctx, "missing-id"), "missing-id");
        // Reported once, and still the id the second time.
        assert_eq!(tr(&ctx, "missing-id"), "missing-id");
    }

    #[test]
    fn missing_argument_still_gives_text() {
        let ctx = egui::Context::default();
        register(&ctx, TEST);
        install(&ctx, &explicit("en"));
        assert!(tr(&ctx, "greet").contains("Hello"));
        install(&ctx, &explicit("ja"));
        assert!(tr(&ctx, "greet").contains("こんにちは"));
    }

    #[test]
    fn plural_follows_the_number() {
        let ctx = egui::Context::default();
        register(&ctx, TEST);
        install(&ctx, &explicit("en"));
        assert_eq!(tr_args(&ctx, "peers", &[("n", Arg::from(1))]), "one peer");
        let two = tr_args(&ctx, "peers", &[("n", Arg::from(2))]);
        assert_eq!(two, "2 peers");
        assert!(!two.contains('\u{2068}'));
        assert_eq!(crate::tr!(ctx, "peers", n = 2u64), "2 peers");
    }

    #[test]
    fn later_registration_overrides_and_rebuilds() {
        let ctx = egui::Context::default();
        install(&ctx, &explicit("en"));
        assert_eq!(tr(&ctx, "corundum-language"), "Language");
        register(
            &ctx,
            Resources {
                name: "X",
                files: &[("en", "corundum-language = Tongue\n")],
            },
        );
        assert_eq!(tr(&ctx, "corundum-language"), "Tongue");
    }

    #[test]
    fn register_is_idempotent_by_name() {
        let ctx = egui::Context::default();
        register(&ctx, TEST);
        register(&ctx, TEST);
        let names: Vec<_> = registered(&ctx).iter().map(|r| r.name).collect();
        assert_eq!(names, ["corundum-gui", "test"]);
    }

    #[test]
    fn switching_takes_effect_on_the_next_lookup() {
        let ctx = egui::Context::default();
        install(&ctx, &explicit("ja"));
        assert_eq!(tr(&ctx, "corundum-language"), "言語");
        install(&ctx, &explicit("en"));
        assert_eq!(tr(&ctx, "corundum-language"), "Language");
    }

    #[test]
    fn message_text_resolves_args() {
        let ctx = egui::Context::default();
        register(&ctx, TEST);
        install(&ctx, &explicit("en"));
        assert_eq!(Message::new("peers").arg("n", 2).text(&ctx), "2 peers");
    }

    #[test]
    fn available_lists_english_first() {
        let ctx = egui::Context::default();
        assert_eq!(available(&ctx), [lang("en"), lang("ja")]);
    }

    #[test]
    fn before_install_english_is_used() {
        let ctx = egui::Context::default();
        assert!(!is_installed(&ctx));
        assert_eq!(current(&ctx), lang("en"));
        assert_eq!(tr(&ctx, "corundum-language"), "Language");
        assert!(!is_installed(&ctx));
        install(&ctx, &LanguageSettings::default());
        assert!(is_installed(&ctx));
    }

    #[test]
    fn name_of_uses_the_languages_own_name() {
        let ctx = egui::Context::default();
        install(&ctx, &explicit("en"));
        assert_eq!(name_of(&ctx, &lang("ja")), "日本語");
        assert_eq!(name_of(&ctx, &lang("en")), "English");
    }

    #[test]
    fn tr_takes_a_ui() {
        let ctx = egui::Context::default();
        install(&ctx, &explicit("ja"));
        let mut text = String::new();
        crate::test_support::render_in(&ctx, [200.0, 100.0], |ui| {
            text = crate::tr!(ui, "corundum-language-system", language = "日本語");
        });
        assert_eq!(text, "システム (日本語)");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn settings_round_trip() {
        for settings in [explicit("ja"), LanguageSettings::default()] {
            let json = serde_json::to_string(&settings).unwrap();
            assert_eq!(
                serde_json::from_str::<LanguageSettings>(&json).unwrap(),
                settings
            );
        }
    }
}
