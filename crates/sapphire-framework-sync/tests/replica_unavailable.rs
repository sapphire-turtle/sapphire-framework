mod common;

use std::collections::HashSet;

use common::*;
use sapphire_framework_sync::ContentHash;
use sapphire_framework_sync::testing::MapSource;

/// Join everything `from` has into `to` without any content, as a session whose
/// content never arrived would, and commit it.
fn push_without_content(from: &Node, to: &mut Node) {
    let updates = from.replica.delta_for(to.replica.vv()).unwrap();
    to.replica.apply(&updates, &MapSource::default()).unwrap();
    to.replica.commit_session(from.replica.vv()).unwrap();
}

fn hash(body: &str) -> ContentHash {
    ContentHash::of_bytes(body.as_bytes())
}

fn unavailable(node: &Node) -> HashSet<ContentHash> {
    node.replica.unavailable().unwrap().into_iter().collect()
}

#[test]
fn a_winner_whose_content_never_arrived_is_unavailable() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "hello");
    scan(&mut a);
    push_without_content(&a, &mut b);
    assert_eq!(unavailable(&b), HashSet::from([hash("hello")]));
    // Committed: the next delta would not carry the path again.
    assert!(a.replica.delta_for(b.replica.vv()).unwrap().is_empty());
}

#[test]
fn materialised_content_is_not_unavailable() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "hello");
    scan(&mut a);
    push(&a, &mut b);
    assert!(unavailable(&b).is_empty());
    assert!(unavailable(&a).is_empty());
}

#[test]
fn a_tombstone_is_never_unavailable() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "hello");
    scan(&mut a);
    sync(&mut a, &mut b);
    remove(&a, "a.txt");
    scan(&mut a);
    push_without_content(&a, &mut b);
    assert!(unavailable(&b).is_empty());
}

#[test]
fn staged_content_is_not_unavailable() {
    let (mut a, mut b) = (node(1), node(2));
    write(&a, "a.txt", "hello");
    scan(&mut a);
    push_without_content(&a, &mut b);
    let mut w = b.replica.stager().writer(hash("hello"), 5).unwrap();
    w.write(b"hello").unwrap();
    w.finish().unwrap();
    assert!(unavailable(&b).is_empty());
}

#[test]
fn content_over_the_size_limit_is_not_unavailable() {
    let mut a = node(1);
    let mut b = node_with(2, |c| c.max_file_size = 4);
    write(&a, "big.bin", "0123456789");
    scan(&mut a);
    push_without_content(&a, &mut b);
    assert!(unavailable(&b).is_empty());
}

#[test]
fn content_of_an_ignored_path_is_not_unavailable() {
    let mut a = node(1);
    let mut b = node(2);
    write(&a, "a.txt", "hello");
    scan(&mut a);
    write(&b, ".sapphireignore", "a.txt\n");
    scan(&mut b);
    push_without_content(&a, &mut b);
    assert!(unavailable(&b).is_empty());
}

#[test]
fn a_loser_waiting_for_its_conflict_copy_is_unavailable() {
    let (mut a, mut b, mut c) = (node(1), node(2), node(3));
    write(&a, "a.txt", "from a");
    scan(&mut a);
    write(&b, "a.txt", "from b");
    scan(&mut b);
    push_without_content(&a, &mut c);
    push_without_content(&b, &mut c);
    assert_eq!(
        unavailable(&c),
        HashSet::from([hash("from a"), hash("from b")])
    );
}
