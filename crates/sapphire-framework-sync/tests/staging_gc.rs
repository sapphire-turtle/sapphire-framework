mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use common::*;
use sapphire_framework_sync::ContentHash;
use sapphire_framework_sync::testing::MapSource;

/// Join everything `from` has into `to` without any content, and commit it.
fn push_without_content(from: &Node, to: &mut Node) {
    let updates = from.replica.delta_for(to.replica.vv()).unwrap();
    to.replica.apply(&updates, &MapSource::default()).unwrap();
    to.replica.commit_session(from.replica.vv()).unwrap();
}

/// Put `body` in `node`'s staging directory, as a finished receive would.
fn stage(node: &Node, body: &str) -> PathBuf {
    let hash = ContentHash::of_bytes(body.as_bytes());
    let mut w = node
        .replica
        .stager()
        .writer(hash, body.len() as u64)
        .unwrap();
    w.write(body.as_bytes()).unwrap();
    w.finish().unwrap()
}

/// Move `path`'s mtime two hours back: past the grace period, and not racy.
fn age(path: &Path) {
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(2 * 60 * 60))
        .unwrap();
}

fn stage_old(node: &Node, body: &str) -> PathBuf {
    let path = stage(node, body);
    age(&path);
    path
}

#[test]
fn content_no_version_refers_to_is_removed() {
    let a = node(1);
    let stray = stage_old(&a, "stray");
    assert_eq!(a.replica.collect_staging().unwrap(), 1);
    assert!(!stray.exists());
}

#[test]
fn content_staged_within_the_grace_period_is_kept() {
    let a = node(1);
    let stray = stage(&a, "stray");
    assert_eq!(a.replica.collect_staging().unwrap(), 0);
    assert!(stray.exists());
}

#[test]
fn content_a_workspace_file_holds_is_removed() {
    let mut a = node(1);
    write(&a, "a.txt", "hello");
    age(&a.root.join("a.txt"));
    scan(&mut a);
    let staged = stage_old(&a, "hello");
    assert_eq!(a.replica.collect_staging().unwrap(), 1);
    assert!(!staged.exists());
}

#[test]
fn content_a_racy_workspace_file_holds_is_kept() {
    let mut a = node(1);
    write(&a, "a.txt", "hello");
    scan(&mut a);
    let staged = stage_old(&a, "hello");
    assert_eq!(a.replica.collect_staging().unwrap(), 0);
    assert!(staged.exists());
}

#[test]
fn content_of_an_unmaterialised_winner_is_kept() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "hello");
    scan(&mut a);
    push_without_content(&a, &mut b);
    let staged = stage_old(&b, "hello");
    assert_eq!(b.replica.collect_staging().unwrap(), 0);
    assert!(staged.exists());
}

#[test]
fn content_of_a_superseded_version_is_removed() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "one");
    scan(&mut a);
    push_without_content(&a, &mut b);
    write(&a, "a.txt", "two");
    scan(&mut a);
    push_without_content(&a, &mut b);
    let old = stage_old(&b, "one");
    let new = stage_old(&b, "two");
    assert_eq!(b.replica.collect_staging().unwrap(), 1);
    assert!(!old.exists());
    assert!(new.exists());
}

#[test]
fn content_of_a_loser_waiting_for_its_conflict_copy_is_kept() {
    let (mut a, mut b, mut c) = (node(1), node(2), node(3));
    write(&a, "a.txt", "from a");
    scan(&mut a);
    write(&b, "a.txt", "from b");
    scan(&mut b);
    push_without_content(&a, &mut c);
    push_without_content(&b, &mut c);
    let from_a = stage_old(&c, "from a");
    let from_b = stage_old(&c, "from b");
    assert_eq!(c.replica.collect_staging().unwrap(), 0);
    assert!(from_a.exists() && from_b.exists());
}

#[test]
fn files_not_named_by_a_hash_are_left_alone() {
    let a = node(1);
    let dir = a.replica.config().staging_dir.clone();
    let other = dir.join("notes");
    std::fs::write(&other, "not content").unwrap();
    age(&other);
    assert_eq!(a.replica.collect_staging().unwrap(), 0);
    assert!(other.exists());
}

#[test]
fn a_scan_collects_staging() {
    let mut a = node(1);
    let stray = stage_old(&a, "stray");
    scan(&mut a);
    assert!(!stray.exists());
}

#[test]
fn opening_a_replica_collects_staging() {
    let a = node(1);
    let stray = stage_old(&a, "stray");
    let _a = reopen(a);
    assert!(!stray.exists());
}
