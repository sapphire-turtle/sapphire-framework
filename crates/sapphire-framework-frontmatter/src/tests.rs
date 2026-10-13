use super::*;

use indexmap::IndexMap;
use serde::Deserialize;

/// The shape of a journal entry's front matter: typed fields, a nested table, a date, and
/// the keys the application keeps without interpreting.
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Entry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task: Option<Task>,
    #[serde(flatten)]
    extra: IndexMap<String, Value>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Task {
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    due: Option<chrono::NaiveDateTime>,
}

// ── split ────────────────────────────────────────────────────────────────

#[test]
fn split_lf() {
    let raw = "---\nfoo: 1\nbar: two\n---\n\n# Body\n";
    let (front, body) = split(raw).unwrap();
    assert_eq!(front, "foo: 1\nbar: two\n");
    assert_eq!(body, "\n# Body\n");
}

#[test]
fn split_crlf() {
    let raw = "---\r\nfoo: 1\r\n---\r\nbody\r\n";
    let (front, body) = split(raw).unwrap();
    assert_eq!(front, "foo: 1\r\n");
    assert_eq!(body, "body\r\n");
}

#[test]
fn split_empty_front_matter() {
    let (front, body) = split("---\n---\nbody\n").unwrap();
    assert_eq!(front, "");
    assert_eq!(body, "body\n");
}

#[test]
fn split_without_front_matter() {
    assert!(split("# Just markdown\n").is_none());
    assert!(split("").is_none());
    assert!(split("---\nno closing fence\n").is_none());
    // The fence must be a line of its own.
    assert!(split("---title: x\n---\n").is_none());
}

#[test]
fn a_fence_inside_the_body_is_the_bodys() {
    let (front, body) = split("---\na: 1\n---\nbefore\n---\nafter\n").unwrap();
    assert_eq!(front, "a: 1\n");
    assert_eq!(body, "before\n---\nafter\n");
}

// ── parse ────────────────────────────────────────────────────────────────

#[test]
fn parse_reads_a_typed_entry() {
    let raw = "---\nid: '0000000'\ntitle: Hello\ntags: [rust, cli]\n---\nsome body\n";
    let (entry, body): (Entry, _) = parse(raw).unwrap();
    assert_eq!(entry.id, "0000000");
    assert_eq!(entry.title, "Hello");
    assert_eq!(entry.tags, vec!["rust", "cli"]);
    assert_eq!(body, "some body\n");
}

#[test]
fn parse_reads_crlf() {
    let raw = "---\r\nid: '0000000'\r\ntitle: Hello\r\ntags: [rust, cli]\r\n---\r\nsome body\r\n";
    let (entry, body): (Entry, _) = parse(raw).unwrap();
    assert_eq!(entry.title, "Hello");
    assert_eq!(entry.tags, vec!["rust", "cli"]);
    assert_eq!(body, "some body\r\n");
}

#[test]
fn parse_tells_missing_from_unclosed() {
    assert!(matches!(
        parse::<Entry>("# no front matter\n"),
        Err(Error::Missing)
    ));
    assert!(matches!(
        parse::<Entry>("---\ntitle: x\n"),
        Err(Error::Unclosed)
    ));
    assert!(matches!(
        parse::<Entry>("---\ntitle: [unclosed\n---\n"),
        Err(Error::Parse(_))
    ));
}

#[test]
fn an_empty_block_reads_as_an_empty_mapping() {
    let (entry, _): (Entry, _) = parse("---\n---\nbody\n").unwrap();
    assert_eq!(entry, Entry::default());
    let (map, _): (Mapping, _) = parse("---\n\n---\n").unwrap();
    assert!(map.is_empty());
}

#[test]
fn parse_optional_reads_a_plain_file_as_all_body() {
    let (entry, body) = parse_optional::<Entry>("# Just markdown\n").unwrap();
    assert!(entry.is_none());
    assert_eq!(body, "# Just markdown\n");
    let (entry, body) = parse_optional::<Entry>("---\ntitle: x\n---\nb\n").unwrap();
    assert_eq!(entry.unwrap().title, "x");
    assert_eq!(body, "b\n");
    assert!(matches!(
        parse_optional::<Entry>("---\ntitle: x\n"),
        Err(Error::Unclosed)
    ));
}

#[test]
fn unknown_keys_are_kept_in_order() {
    let raw = "---\ntitle: t\nzeta: 1\nalpha: [a, b]\nnested:\n  k: v\n---\n";
    let (entry, _): (Entry, _) = parse(raw).unwrap();
    let keys: Vec<&str> = entry.extra.keys().map(String::as_str).collect();
    assert_eq!(keys, ["zeta", "alpha", "nested"]);
    assert_eq!(entry.extra["alpha"], serde_json::json!(["a", "b"]));
    assert_eq!(entry.extra["nested"]["k"], serde_json::json!("v"));
}

#[test]
fn a_mapping_keeps_values_and_order() {
    let (map, _): (Mapping, _) = parse("---\nfoo: 1\nbar: two\nflag: true\n---\n").unwrap();
    let keys: Vec<&str> = map.keys().map(String::as_str).collect();
    assert_eq!(keys, ["foo", "bar", "flag"]);
    assert_eq!(map["foo"].as_i64(), Some(1));
    assert_eq!(map["bar"].as_str(), Some("two"));
    assert_eq!(map["flag"].as_bool(), Some(true));
}

/// Files written by `serde_yaml`, as the apps wrote them before: they must still read.
#[test]
fn front_matter_written_by_serde_yaml_still_reads() {
    let raw = "---\n\
id: '0000000'\n\
title: 'Fix: the parser'\n\
tags:\n\
- rust\n\
- cli\n\
task:\n  status: open\n  due: 2026-03-10T00:00:00\n\
created_at: 2026-03-01T09:30:00\n\
note: |\n  line one\n  line two\n\
---\n\nbody\n";
    let (entry, body): (Entry, _) = parse(raw).unwrap();
    assert_eq!(entry.id, "0000000");
    assert_eq!(entry.title, "Fix: the parser");
    assert_eq!(entry.tags, vec!["rust", "cli"]);
    let task = entry.task.unwrap();
    assert_eq!(task.status, "open");
    assert_eq!(
        task.due,
        Some("2026-03-10T00:00:00".parse().unwrap()),
        "a date reads into chrono"
    );
    assert_eq!(
        entry.extra["created_at"],
        serde_json::json!("2026-03-01T09:30:00")
    );
    assert_eq!(
        entry.extra["note"],
        serde_json::json!("line one\nline two\n")
    );
    assert_eq!(body, "\nbody\n");
}

#[test]
fn duplicate_keys_are_an_error() {
    assert!(matches!(
        parse::<Mapping>("---\na: 1\na: 2\n---\n"),
        Err(Error::Parse(_))
    ));
}

// ── render ───────────────────────────────────────────────────────────────

#[test]
fn render_writes_fences_a_blank_line_and_the_body() {
    let entry = Entry {
        id: "0000000".into(),
        title: "My Task".into(),
        task: Some(Task {
            status: "open".into(),
            due: Some("2026-03-10T00:00:00".parse().unwrap()),
        }),
        ..Entry::default()
    };
    let out = render(&entry, "\n\nbody\n").unwrap();
    assert!(out.starts_with("---\n"), "{out}");
    assert!(out.ends_with("---\n\nbody\n"), "{out}");
    assert!(out.contains("title: My Task"), "{out}");
    assert!(out.contains("status: open"), "{out}");
    // A date-like string is quoted, so YAML 1.1 readers keep it a string; it reads back
    // into the date all the same.
    assert!(out.contains("due: \"2026-03-10T00:00:00\""), "{out}");
    let (again, _): (Entry, _) = parse(&out).unwrap();
    assert_eq!(again, entry);
}

#[test]
fn render_with_an_empty_body_ends_at_the_fence() {
    let out = render(&Entry::default(), "").unwrap();
    assert!(out.ends_with("---\n"), "{out:?}");
    assert!(!out.ends_with("---\n\n"), "{out:?}");
}

#[test]
fn render_round_trips() {
    let raw = "---\nid: '0000007'\ntitle: 'a: b'\ntags: [x]\nzeta: 1\n---\n\nbody text\n";
    let (entry, body): (Entry, _) = parse(raw).unwrap();
    let out = render(&entry, body).unwrap();
    let (again, body_again): (Entry, _) = parse(&out).unwrap();
    assert_eq!(again, entry);
    assert_eq!(body_again, "\nbody text\n");
    // A string that looks like a number stays a string.
    assert_eq!(again.id, "0000007");
}

#[test]
fn nested_maps_keep_their_order_through_a_round_trip() {
    let raw = "---\nouter:\n  zeta: 1\n  alpha: 2\n---\n";
    let (map, body): (Mapping, _) = parse(raw).unwrap();
    let out = render(&map, body).unwrap();
    assert!(
        out.find("zeta").unwrap() < out.find("alpha").unwrap(),
        "{out}"
    );
}

#[test]
fn a_mapping_round_trips() {
    let raw = "---\nfoo: 1\nbar: two\nlist: [a, b]\n---\n\nbody\n";
    let (map, body): (Mapping, _) = parse(raw).unwrap();
    let out = render(&map, body).unwrap();
    let (again, _): (Mapping, _) = parse(&out).unwrap();
    assert_eq!(again, map);
}

// ── set_top_level_key ────────────────────────────────────────────────────

#[test]
fn set_replaces_only_the_key_and_keeps_the_rest() {
    let raw = "---\n# fires the morning call\nschedule: \"0 8 * * *\"\nenabled: true\nroom_id: \"!ops:x\"\n---\n\n# Morning\nCall the room.\n";
    let out = set_top_level_key(raw, "enabled", &false).unwrap().unwrap();
    assert!(out.contains("enabled: false"), "{out}");
    assert!(out.contains("# fires the morning call"), "{out}");
    assert!(out.contains("room_id: \"!ops:x\""), "{out}");
    assert!(out.ends_with("# Morning\nCall the room.\n"), "{out}");
    let (map, _): (Mapping, _) = parse(&out).unwrap();
    assert_eq!(map["enabled"].as_bool(), Some(false));
    // Only the one line changed.
    assert_eq!(raw.lines().count(), out.lines().count());
}

#[test]
fn set_inserts_the_key_when_it_is_absent() {
    let raw = "---\nschedule: \"0 8 * * *\"\n---\n\nBody\n";
    let out = set_top_level_key(raw, "enabled", &false).unwrap().unwrap();
    let (map, body): (Mapping, _) = parse(&out).unwrap();
    assert_eq!(map["enabled"].as_bool(), Some(false));
    assert_eq!(body, "\nBody\n");
    // A second call replaces rather than duplicating.
    let again = set_top_level_key(&out, "enabled", &true).unwrap().unwrap();
    assert_eq!(again.matches("enabled:").count(), 1, "{again}");
}

#[test]
fn set_leaves_a_nested_key_of_the_same_name_alone() {
    let raw = "---\nschedule: \"0 8 * * *\"\nvoice:\n  device_id: \"01J\"\n  enabled: true\n---\n\nBody\n";
    let out = set_top_level_key(raw, "enabled", &false).unwrap().unwrap();
    assert!(out.contains("  enabled: true"), "{out}");
    assert_eq!(out.matches("enabled: false").count(), 1, "{out}");
}

#[test]
fn set_does_not_match_a_longer_key() {
    let raw = "---\nenabled_at: 2026-01-01\n---\n";
    let out = set_top_level_key(raw, "enabled", &true).unwrap().unwrap();
    assert!(out.contains("enabled_at: 2026-01-01\n"), "{out}");
    assert!(out.contains("enabled: true\n"), "{out}");
}

#[test]
fn set_replaces_a_block_value_with_its_continuation_lines() {
    let raw = "---\ntags:\n- a\n- b\nnote:\n  nested: 1\ntitle: t\n---\nbody\n";
    let out = set_top_level_key(raw, "tags", &"none").unwrap().unwrap();
    let (map, body): (Mapping, _) = parse(&out).unwrap();
    assert_eq!(map["tags"].as_str(), Some("none"));
    assert_eq!(map["note"]["nested"].as_i64(), Some(1));
    assert_eq!(map["title"].as_str(), Some("t"));
    assert_eq!(body, "body\n");

    let out = set_top_level_key(raw, "note", &"flat").unwrap().unwrap();
    let (map, _): (Mapping, _) = parse(&out).unwrap();
    assert_eq!(map["note"].as_str(), Some("flat"));
    assert_eq!(map["tags"], serde_json::json!(["a", "b"]));
}

#[test]
fn set_quotes_a_string_that_needs_it() {
    let raw = "---\nroom_id: x\n---\n";
    let out = set_top_level_key(raw, "room_id", &"!ops: x #1")
        .unwrap()
        .unwrap();
    let (map, _): (Mapping, _) = parse(&out).unwrap();
    assert_eq!(map["room_id"].as_str(), Some("!ops: x #1"));
}

#[test]
fn set_refuses_a_value_that_does_not_fit_on_one_line() {
    let raw = "---\na: 1\n---\n";
    assert!(matches!(
        set_top_level_key(raw, "a", &vec!["x", "y"]),
        Err(Error::NotAScalar(_))
    ));
    assert!(matches!(
        set_top_level_key(raw, "a", &"two\nlines"),
        Err(Error::NotAScalar(_))
    ));
}

#[test]
fn set_is_none_without_front_matter() {
    assert!(
        set_top_level_key("# Just markdown\n", "enabled", &true)
            .unwrap()
            .is_none()
    );
    assert!(set_top_level_key("", "enabled", &true).unwrap().is_none());
    assert!(
        set_top_level_key("---\nnever closed\n", "enabled", &true)
            .unwrap()
            .is_none()
    );
}

/// CRLF files exist; rewriting them as LF would be a whole-file diff.
#[test]
fn set_keeps_crlf() {
    let raw = "---\r\nschedule: \"0 8 * * *\"\r\n---\r\nBody\r\n";
    let out = set_top_level_key(raw, "enabled", &false).unwrap().unwrap();
    assert!(out.ends_with("---\r\nBody\r\n"), "{out:?}");
    assert!(out.contains("enabled: false\r\n"), "{out:?}");
    assert!(
        !out.replace("\r\n", "").contains('\n'),
        "an LF crept in: {out:?}"
    );
}
