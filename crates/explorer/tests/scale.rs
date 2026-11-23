//! Sanity timings on a synthetic million-node tree. Ignored by default; the
//! numbers only mean something in release mode:
//!
//! `cargo test --release -p explorer --test scale -- --ignored --nocapture`
//!
//! Real measurements belong in criterion benches.

use std::time::Instant;

use explorer::{Check, FixedBitSet, NodeId, Selection, Tree};

/// 10 top folders, 100 folders in each, 1,000 files in each of those:
/// 1,001,011 nodes.
fn million_paths() -> Vec<(String, u64)> {
    let mut paths = Vec::with_capacity(1_001_010);
    for top in 0..10 {
        for mid in 0..100 {
            for file in 0..1000 {
                paths.push((format!("Top {top}/dir{mid}/file{file}.txt"), file));
            }
        }
    }
    paths
}

#[test]
#[ignore = "timing probe; run in release with --ignored --nocapture"]
#[expect(clippy::print_stderr, reason = "the probe reports its timings")]
fn a_million_node_tree_builds_and_toggles_quickly() {
    let owned = million_paths();
    let paths: Vec<(&str, u64)> = owned.iter().map(|(p, s)| (p.as_str(), *s)).collect();

    let start = Instant::now();
    let tree = Tree::from_paths_with_sizes(&paths).unwrap();
    let built = start.elapsed();
    assert_eq!(tree.node_count(), 1_001_011);
    assert_eq!(tree.name(NodeId::new(1)), "Top 0");

    let mut selection = Selection::new(&tree);
    let start = Instant::now();
    selection.toggle(&tree, None, NodeId::ROOT);
    let toggled = start.elapsed();
    assert_eq!(selection.count(), 1_000_000);

    let start = Instant::now();
    let check = selection.check(&tree, None, NodeId::ROOT);
    let checked = start.elapsed();
    assert_eq!(check, Check::On);

    // Under a filter: a universe of every other node.
    let mut universe: FixedBitSet = (0..tree.node_count()).step_by(2).collect();
    universe.grow(tree.node_count());
    selection.clear();
    let start = Instant::now();
    selection.toggle(&tree, Some(&universe), NodeId::ROOT);
    let check = selection.check(&tree, Some(&universe), NodeId::ROOT);
    let filtered = start.elapsed();
    assert_eq!(check, Check::On);

    eprintln!("build {} nodes: {built:?}", tree.node_count());
    eprintln!("toggle root: {toggled:?}; check root: {checked:?}");
    eprintln!("toggle + check root under a universe: {filtered:?}");
}
