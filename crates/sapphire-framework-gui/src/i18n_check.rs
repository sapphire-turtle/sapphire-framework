//! The checks on this crate's translations: the files parse, `en` and `ja` have the same
//! keys, and every `"sapphire-…"` id in the source exists.

use std::path::Path;

use corundum_framework_gui::i18n::check::{key_difference, parse_errors, unknown_ids};

use crate::RESOURCES;

/// String literals that start with `sapphire-` and are not message ids.
const NOT_IDS: &[&str] = &[
    "sapphire-bridge",
    "sapphire-config-secret",
    "sapphire-ed-x",
    "sapphire-example",
    "sapphire-framework-gui",
    "sapphire-gui-test",
    "sapphire-mobiletest",
    "sapphire-timer",
];

#[test]
fn sapphire_files_parse() {
    assert_eq!(parse_errors(&RESOURCES), Vec::<String>::new());
}

#[test]
fn sapphire_en_and_ja_have_the_same_keys() {
    assert_eq!(key_difference(&RESOURCES, "en", "ja"), Vec::<String>::new());
}

#[test]
fn sapphire_ids_exist() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert_eq!(
        unknown_ids(&RESOURCES, "en", &src, "sapphire-", NOT_IDS),
        Vec::<String>::new()
    );
}
