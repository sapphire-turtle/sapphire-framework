//! Checks over a crate's translation files, for its own tests: that they parse, that every
//! language has the same keys, and that every id the code uses exists.
//!
//! Each returns what it found wrong, empty when nothing is, so a test asserts it empty and
//! a failure lists every problem at once.

use std::collections::BTreeSet;
use std::path::Path;

use fluent_bundle::FluentResource;
use fluent_syntax::ast::Entry;

use super::Resources;

/// Every parse error in `r`'s files, as `"<name> <tag>:<line>: <error>"`.
pub fn parse_errors(r: &Resources) -> Vec<String> {
    let mut found = Vec::new();
    for (tag, source) in r.files {
        if let Err((_, errors)) = FluentResource::try_new(source.to_string()) {
            for error in errors {
                let line = source[..error.pos.start.min(source.len())]
                    .matches('\n')
                    .count()
                    + 1;
                found.push(format!("{} {tag}:{line}: {error}", r.name));
            }
        }
    }
    found
}

/// The keys one of `a` and `b` has and the other lacks, as `"ja lacks x"`: `b`'s missing
/// first, then `a`'s, each by key.
pub fn key_difference(r: &Resources, a: &str, b: &str) -> Vec<String> {
    let (keys_a, keys_b) = (keys(r, a), keys(r, b));
    let lacks = |lang: &str, have: &BTreeSet<String>, other: &BTreeSet<String>| {
        other
            .difference(have)
            .map(|k| format!("{lang} lacks {k}"))
            .collect::<Vec<_>>()
    };
    let mut found = lacks(b, &keys_b, &keys_a);
    found.extend(lacks(a, &keys_a, &keys_b));
    found
}

/// Every string literal `"<prefix>…"` (the rest lowercase letters, digits and `-`) in the
/// `.rs` files under `src` that `lang`'s file lacks, as `"<file>:<line>: <id>"`; those in
/// `ignore` are not ids. A directory or file that cannot be read is reported too, and so is
/// a `src` with no `.rs` files under it, so a wrong path fails rather than passes.
pub fn unknown_ids(
    r: &Resources,
    lang: &str,
    src: &Path,
    prefix: &str,
    ignore: &[&str],
) -> Vec<String> {
    let known = keys(r, lang);
    let mut files = Vec::new();
    let mut found = Vec::new();
    rust_files(src, &mut files, &mut found);
    if files.is_empty() {
        found.push(format!("{}: no .rs files under it", src.display()));
    }
    files.sort();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            found.push(format!("{}: unreadable", file.display()));
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            for id in ids_in(line, prefix) {
                if !known.contains(id) && !ignore.contains(&id) {
                    found.push(format!("{}:{}: {id}", file.display(), n + 1));
                }
            }
        }
    }
    found
}

/// The message and term (`-name`) ids in `lang`'s files.
fn keys(r: &Resources, lang: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for (_, source) in r.files.iter().filter(|(tag, _)| *tag == lang) {
        let resource = FluentResource::try_new(source.to_string()).unwrap_or_else(|(r, _)| r);
        for entry in resource.entries() {
            match entry {
                Entry::Message(m) => keys.insert(m.id.name.to_owned()),
                Entry::Term(t) => keys.insert(format!("-{}", t.id.name)),
                _ => false,
            };
        }
    }
    keys
}

/// The `.rs` files under `dir` into `out`; a directory that cannot be read into `found`.
fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>, found: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            found.push(format!("{}: unreadable: {error}", dir.display()));
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The ids in `line`'s string literals that are `prefix` and then `[a-z0-9-]+`.
fn ids_in<'a>(line: &'a str, prefix: &str) -> Vec<&'a str> {
    let opening = format!("\"{prefix}");
    let mut ids = Vec::new();
    let mut rest = line;
    while let Some(at) = rest.find(&opening) {
        let start = at + 1;
        let tail = &rest[start + prefix.len()..];
        let len = tail
            .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
            .unwrap_or(tail.len());
        if len > 0 && tail[len..].starts_with('"') {
            ids.push(&rest[start..start + prefix.len() + len]);
        }
        rest = &rest[start..];
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::CORUNDUM;

    #[test]
    fn corundum_files_parse() {
        assert_eq!(parse_errors(&CORUNDUM), Vec::<String>::new());
    }

    #[test]
    fn corundum_en_and_ja_have_the_same_keys() {
        assert_eq!(key_difference(&CORUNDUM, "en", "ja"), Vec::<String>::new());
    }

    // Reads the crate's sources, which are not on the phone the Android tests run on.
    #[cfg(not(target_os = "android"))]
    #[test]
    fn corundum_ids_exist() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        assert_eq!(
            unknown_ids(&CORUNDUM, "en", &src, "corundum-", &["corundum-gui"]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn ids_in_finds_whole_literals() {
        // `probe-`, not `corundum-`: `corundum_ids_exist` scans this file too.
        let line = r#"f("probe-a-b", "probe-x.y", "probe-", "probe-c1")"#;
        assert_eq!(ids_in(line, "probe-"), ["probe-a-b", "probe-c1"]);
        assert_eq!(ids_in(r#"x("probe-one")"#, "probe-"), ["probe-one"]);
        assert_eq!(ids_in("no literals here", "probe-"), Vec::<&str>::new());
    }

    // Reads the crate's sources, which are not on the phone the Android tests run on.
    #[cfg(not(target_os = "android"))]
    #[test]
    fn unknown_ids_reports_unknown_ids_and_bad_paths() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        // Only `corundum-language` is known, so the other ids in src are reported.
        let r = Resources {
            name: "t",
            files: &[("en", "corundum-language = L\n")],
        };
        let found = unknown_ids(&r, "en", &src, "corundum-", &["corundum-gui"]);
        assert!(
            found
                .iter()
                .any(|f| f.ends_with(": corundum-language-name")),
            "{found:?}"
        );
        assert!(!found.iter().any(|f| f.ends_with(": corundum-language")));

        let missing = src.join("no-such-dir");
        let found = unknown_ids(&CORUNDUM, "en", &missing, "corundum-", &[]);
        assert!(!found.is_empty());
        assert!(
            found
                .iter()
                .all(|f| f.starts_with(&missing.display().to_string()))
        );
    }

    #[test]
    fn check_reports_differences() {
        let r = Resources {
            name: "t",
            files: &[
                ("en", "both = B\nonly-a = A\n"),
                ("ja", "both = B\nonly-b = B\nbad line\n"),
            ],
        };
        assert_eq!(
            parse_errors(&r),
            ["t ja:3: Expected a token starting with \"=\""]
        );
        assert_eq!(
            key_difference(&r, "en", "ja"),
            ["ja lacks only-a", "en lacks only-b"]
        );
    }
}
