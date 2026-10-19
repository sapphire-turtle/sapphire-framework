mod common;

use common::*;
use sapphire_framework_sync::testing::MapSource;
use sapphire_framework_sync::{Content, ContentHash, PathUpdate, SkipReason, VersionVector};

#[test]
fn a_new_file_propagates() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "notes/a.txt", "hello");
    let report = scan(&mut a);
    assert_eq!(report.recorded.len(), 1);
    push(&a, &mut b);
    assert_eq!(read(&b, "notes/a.txt").as_deref(), Some("hello"));
    let sa = a.replica.state("notes/a.txt").unwrap().unwrap();
    let sb = b.replica.state("notes/a.txt").unwrap().unwrap();
    assert_eq!(sa.versions, sb.versions);
    assert_eq!(sa.seen, sb.seen);
}

#[test]
fn an_edit_on_top_of_a_synced_file_supersedes_it() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "one");
    scan(&mut a);
    sync(&mut a, &mut b);
    write(&b, "a.txt", "two");
    scan(&mut b);
    push(&b, &mut a);
    assert_eq!(read(&a, "a.txt").as_deref(), Some("two"));
    assert_eq!(a.replica.state("a.txt").unwrap().unwrap().versions.len(), 1);
    assert_eq!(tree(&a).len(), 1, "no conflict copy");
}

#[test]
fn a_delete_propagates() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "one");
    scan(&mut a);
    sync(&mut a, &mut b);
    remove(&a, "a.txt");
    let report = scan(&mut a);
    assert!(report.recorded[0].content.is_tombstone());
    push(&a, &mut b);
    assert_eq!(read(&b, "a.txt"), None);
    assert!(
        b.replica
            .state("a.txt")
            .unwrap()
            .unwrap()
            .winner()
            .content
            .is_tombstone()
    );
}

#[test]
fn scanning_twice_records_nothing_new() {
    let mut a = node(1);
    write(&a, "a.txt", "one");
    scan(&mut a);
    let again = scan(&mut a);
    assert!(again.recorded.is_empty());
    assert_eq!(again.changed, 0);
    write(&a, "a.txt", "one");
    assert!(
        scan(&mut a).recorded.is_empty(),
        "rewriting identical bytes is not an edit"
    );
}

#[test]
fn nothing_to_send_to_an_up_to_date_peer() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "one");
    scan(&mut a);
    sync(&mut a, &mut b);
    assert!(a.replica.delta_for(b.replica.vv()).unwrap().is_empty());
    assert!(b.replica.delta_for(a.replica.vv()).unwrap().is_empty());
}

#[test]
fn re_applying_a_full_delta_changes_nothing() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "one");
    scan(&mut a);
    push(&a, &mut b);
    let everything = a.replica.delta_for(&VersionVector::new()).unwrap();
    let report = b.replica.apply(&everything, &a.replica).unwrap();
    assert_eq!(report.changed, 0);
}

#[test]
fn hidden_and_ignored_files_are_not_recorded() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, ".sapphireignore", "*.tmp\n");
    write(&a, "x.tmp", "scratch");
    write(&a, ".git/config", "secret");
    write(&a, "keep.txt", "kept");
    let recorded: Vec<String> = scan(&mut a).recorded.into_iter().map(|e| e.path).collect();
    assert_eq!(
        recorded,
        vec![".sapphireignore".to_string(), "keep.txt".to_string()]
    );
    push(&a, &mut b);
    assert_eq!(read(&b, "x.tmp"), None);
    assert_eq!(read(&b, "keep.txt").as_deref(), Some("kept"));
}

#[test]
fn a_local_file_over_the_size_cap_is_skipped() {
    let mut a = node_with(1, |c| c.max_file_size = 4);
    write(&a, "big.bin", "0123456789");
    let report = scan(&mut a);
    assert!(report.recorded.is_empty());
    assert_eq!(
        report.skipped[0].reason,
        SkipReason::TooLarge { len: 10, max: 4 }
    );
}

#[test]
fn a_remote_file_over_the_size_cap_is_kept_but_not_written() {
    let mut a = node(1);
    let mut b = node_with(2, |c| c.max_file_size = 4);
    write(&a, "big.bin", "0123456789");
    scan(&mut a);
    let report = push(&a, &mut b);
    assert_eq!(
        report.skipped[0].reason,
        SkipReason::TooLarge { len: 10, max: 4 }
    );
    assert!(b.replica.state("big.bin").unwrap().is_some());
    assert_eq!(read(&b, "big.bin"), None);
}

#[test]
fn unavailable_content_is_fetched_later() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "hello");
    scan(&mut a);
    let updates = a.replica.delta_for(b.replica.vv()).unwrap();
    let report = b.replica.apply(&updates, &MapSource::default()).unwrap();
    assert_eq!(report.skipped[0].reason, SkipReason::ContentUnavailable);
    assert_eq!(read(&b, "a.txt"), None);
    b.replica.fetch_missing(&a.replica).unwrap();
    assert_eq!(read(&b, "a.txt").as_deref(), Some("hello"));
}

#[test]
fn malformed_updates_are_ignored() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "hello");
    scan(&mut a);
    let mut update: PathUpdate = a.replica.delta_for(b.replica.vv()).unwrap().remove(0);
    update.path = "../escape.txt".into();
    for v in &mut update.versions {
        v.path = update.path.clone();
    }
    let report = b.replica.apply(&[update], &a.replica).unwrap();
    assert_eq!(report.changed, 0);
    assert!(b.replica.states().unwrap().is_empty());

    // A dot with counter 0 has no predecessor for `disk.seen` to pin a loser at.
    let mut zeroed: PathUpdate = a.replica.delta_for(b.replica.vv()).unwrap().remove(0);
    for v in &mut zeroed.versions {
        v.dot.counter = 0;
    }
    let report = b.replica.apply(&[zeroed], &a.replica).unwrap();
    assert_eq!(report.changed, 0);
    assert!(b.replica.states().unwrap().is_empty());
}

#[test]
fn state_survives_reopening() {
    let mut a = node(1);
    write(&a, "a.txt", "hello");
    scan(&mut a);
    let id = a.replica.replica_id();
    let mut a = reopen(a);
    assert_eq!(a.replica.replica_id(), id);
    assert!(scan(&mut a).recorded.is_empty());
    assert!(matches!(
        a.replica.state("a.txt").unwrap().unwrap().winner().content,
        Content::File { len: 5, .. }
    ));
}

#[test]
fn an_incoming_update_leaves_an_oversized_local_file_alone() {
    let mut a = node(1);
    let mut b = node_with(2, |c| c.max_file_size = 8);
    write(&a, "a.txt", "small");
    scan(&mut a);
    push(&a, &mut b);
    write(&b, "a.txt", "this is far too large");
    scan(&mut b);
    write(&a, "a.txt", "tiny");
    scan(&mut a);
    let report = push(&a, &mut b);
    assert_eq!(read(&b, "a.txt").as_deref(), Some("this is far too large"));
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.path == "a.txt" && s.reason == SkipReason::Occupied),
        "{report:?}"
    );
}

#[test]
fn a_directory_in_the_way_does_not_wedge_the_replica() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "notes", "a file");
    scan(&mut a);
    write(&b, "notes/inner.txt", "a directory here");
    scan(&mut b);
    let report = push(&a, &mut b);
    assert!(
        report.skipped.iter().any(|s| s.path == "notes"),
        "{report:?}"
    );
    assert_eq!(
        read(&b, "notes/inner.txt").as_deref(),
        Some("a directory here")
    );
    write(&b, "later.txt", "still recorded");
    assert!(scan(&mut b).recorded.iter().any(|e| e.path == "later.txt"));
}

#[cfg(unix)]
#[test]
fn symlinks_are_skipped() {
    let mut a = node(1);
    write(&a, "target.txt", "t");
    std::os::unix::fs::symlink(a.root.join("target.txt"), a.root.join("link.txt")).unwrap();
    let report = scan(&mut a);
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.path == "link.txt" && s.reason == SkipReason::Symlink)
    );
    assert!(report.recorded.iter().all(|e| e.path != "link.txt"));
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_does_not_abort_the_scan() {
    use std::os::unix::fs::PermissionsExt;
    let mut a = node(1);
    write(&a, "locked/inner.txt", "hidden");
    write(&a, "sibling.txt", "ok");
    let locked = a.root.join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Mode 000 denies root nothing, so under a root-run suite the directory would
    // still be readable and the test's premise would not hold: skip instead of failing.
    if std::fs::read_dir(&locked).is_ok() {
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let report = scan(&mut a);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        report
            .skipped
            .iter()
            .any(|s| matches!(s.reason, SkipReason::Io(_))),
        "{report:?}"
    );
    assert!(
        report.recorded.iter().any(|e| e.path == "sibling.txt"),
        "the walk continued past the unreadable directory: {report:?}"
    );
}

/// `tolerate_io` is what keeps one bad path from wedging a replica: the path is
/// reported and the rest of the scan still runs.
#[cfg(unix)]
#[test]
fn an_unreadable_file_is_reported_and_the_scan_continues() {
    use std::os::unix::fs::PermissionsExt;
    let mut a = node(1);
    write(&a, "locked.txt", "secret");
    write(&a, "sibling.txt", "ok");
    let locked = a.root.join("locked.txt");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // Mode 000 denies root nothing, so under a root-run suite the file would still be
    // readable and the test's premise would not hold: skip instead of failing.
    if std::fs::read(&locked).is_ok() {
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
        return;
    }
    let report = scan(&mut a);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.path == "locked.txt" && matches!(s.reason, SkipReason::Io(_))),
        "{report:?}"
    );
    assert!(
        report.recorded.iter().any(|e| e.path == "sibling.txt"),
        "{report:?}"
    );
    assert!(a.replica.state("locked.txt").unwrap().is_none());
}

#[test]
fn an_ignored_path_is_reported_when_a_peer_sends_it() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "build.log", "noise");
    scan(&mut a);
    write(&b, ".sapphireignore", "*.log\n");
    scan(&mut b);
    let report = push(&a, &mut b);
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.path == "build.log" && s.reason == SkipReason::Ignored),
        "{report:?}"
    );
    assert_eq!(read(&b, "build.log"), None);
}

#[test]
fn open_clears_leftover_partials() {
    let a = node(1);
    let staging = a.replica.config().staging_dir.clone();
    std::fs::write(staging.join("x.partial"), b"junk").unwrap();
    std::fs::write(staging.join("kept"), b"done").unwrap();
    let a = reopen(a);
    assert!(!staging.join("x.partial").exists());
    assert!(staging.join("kept").exists());
    drop(a);
}

#[test]
fn content_path_trusts_an_unchanged_file() {
    let mut a = node(1);
    write(&a, "a.bin", "hello");
    scan(&mut a);
    let hash = ContentHash::of_bytes(b"hello");
    assert_eq!(
        a.replica.content_path(&hash).unwrap(),
        Some((a.root.join("a.bin"), 5))
    );
    assert_eq!(
        a.replica
            .content_path(&ContentHash::of_bytes(b"other"))
            .unwrap(),
        None
    );
}

#[test]
fn content_path_rehashes_a_file_whose_stamp_changed() {
    let mut a = node(1);
    write(&a, "a.bin", "hello");
    scan(&mut a);
    write(&a, "a.bin", "world");
    let hash = ContentHash::of_bytes(b"hello");
    assert_eq!(a.replica.content_path(&hash).unwrap(), None);
}

#[test]
fn content_path_finds_staged_content() {
    let a = node(1);
    let hash = ContentHash::of_bytes(b"hello");
    let mut w = a.replica.stager().writer(hash, 5).unwrap();
    w.write(b"hello").unwrap();
    w.finish().unwrap();
    assert_eq!(
        a.replica.content_path(&hash).unwrap(),
        Some((a.replica.config().staging_dir.join(hash.to_hex()), 5))
    );
}

#[test]
fn a_staged_file_is_used_without_rehashing() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "hello");
    scan(&mut a);
    let hash = ContentHash::of_bytes(b"hello");
    let mut w = b.replica.stager().writer(hash, 5).unwrap();
    w.write(b"hello").unwrap();
    let staged = w.finish().unwrap();
    assert!(staged.is_file());
    // Pin trust-on-existence: only `finish` ever writes `staging/<hex>`, so the file is
    // placed as it stands, not re-hashed. Same length, different bytes: if it were
    // re-hashed it would be refused instead of placed.
    std::fs::write(&staged, b"jello").unwrap();
    let updates = a.replica.delta_for(b.replica.vv()).unwrap();
    b.replica.apply(&updates, &MapSource::default()).unwrap();
    assert_eq!(read(&b, "a.txt").as_deref(), Some("jello"));
    assert!(!staged.exists());
}

#[test]
fn a_file_above_64_mib_is_recorded_by_default() {
    let mut a = node(1);
    let path = a.root.join("big.bin");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(file);
    let report = scan(&mut a);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let recorded: Vec<&str> = report.recorded.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(recorded, ["big.bin"]);
}

#[test]
fn a_conflict_copy_of_a_large_loser_is_written() {
    let (mut a, mut b) = (node(1), node(2));
    let big = "x".repeat(100 * 1024);
    write(&a, "a.txt", &big);
    scan(&mut a);
    b.clock.set(9_000_000);
    write(&b, "a.txt", "from b");
    scan(&mut b);
    // The loser's bytes are only reachable through the source's path.
    let updates = a.replica.delta_for(b.replica.vv()).unwrap();
    let source = MapSource::default().with(big.as_bytes());
    let report = b.replica.apply(&updates, &source).unwrap();
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(read(&b, "a.txt").as_deref(), Some("from b"));
    assert_eq!(read(&b, &report.conflicts[0].copy_path), Some(big));
}

/// b holds a local, recorded `a.txt` ("from b ...", over 4 bytes) as the loser of a
/// conflict with a's newer one-byte "A", and b's size limit has since dropped to 4.
fn loser_on_disk_over_a_lowered_limit() -> (Node, Node, Vec<PathUpdate>) {
    let (mut a, mut b) = (node(1), node(2));
    write(&b, "a.txt", "from b, longer than the new limit");
    scan(&mut b);
    let b = reopen_with(b, |c| c.max_file_size = 4);
    a.clock.set(9_000_000);
    write(&a, "a.txt", "A");
    scan(&mut a);
    let updates = a.replica.delta_for(b.replica.vv()).unwrap();
    (a, b, updates)
}

#[test]
fn an_on_disk_loser_over_a_lowered_limit_still_gets_its_conflict_copy() {
    let (a, mut b, updates) = loser_on_disk_over_a_lowered_limit();
    let report = b.replica.apply(&updates, &a.replica).unwrap();
    assert_eq!(report.conflicts.len(), 1, "{:?}", report.skipped);
    assert_eq!(
        read(&b, &report.conflicts[0].copy_path).as_deref(),
        Some("from b, longer than the new limit")
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_on_disk_loser_is_an_io_skip_and_stays_untouched() {
    use std::os::unix::fs::PermissionsExt;
    let (a, mut b, updates) = loser_on_disk_over_a_lowered_limit();
    let path = b.root.join("a.txt");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&path).is_ok() {
        // Running with privileges that ignore file modes.
        return;
    }
    // `apply` reports a per-path I/O error as a skip rather than failing the batch.
    let result = b.replica.apply(&updates, &a.replica);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let report = result.unwrap();
    assert!(report.conflicts.is_empty());
    assert!(
        report
            .skipped
            .iter()
            .any(|s| matches!(s.reason, SkipReason::Io(_))),
        "{:?}",
        report.skipped
    );
    assert_eq!(
        read(&b, "a.txt").as_deref(),
        Some("from b, longer than the new limit")
    );
}

/// A source whose file does not hold the bytes its name promises.
struct LyingSource(std::path::PathBuf);

impl sapphire_framework_sync::ContentSource for LyingSource {
    fn locate(&self, _: &ContentHash) -> Option<std::path::PathBuf> {
        Some(self.0.clone())
    }
}

#[test]
fn a_located_file_with_the_wrong_bytes_is_unavailable_not_placed() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "hello");
    scan(&mut a);
    let dir = tempfile::tempdir().unwrap();
    let wrong = dir.path().join("wrong");
    std::fs::write(&wrong, "jello").unwrap();
    let updates = a.replica.delta_for(b.replica.vv()).unwrap();
    let report = b.replica.apply(&updates, &LyingSource(wrong)).unwrap();
    assert_eq!(report.skipped[0].reason, SkipReason::ContentUnavailable);
    assert_eq!(read(&b, "a.txt"), None);
    let staging = &b.replica.config().staging_dir;
    assert!(
        std::fs::read_dir(staging)
            .map(|d| d.count() == 0)
            .unwrap_or(true)
    );
}
