//! Sanity timings on a synthetic million-node tree and on a real folder.
//! Ignored by default; the numbers only mean something in release mode:
//!
//! `cargo test --release -p explorer --test scale -- --ignored --nocapture`
//!
//! The real-folder probe scans `$EXPLORER_SCAN_ROOT` and does nothing when
//! it is unset.
//!
//! Real measurements belong in criterion benches.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use explorer::{
    Check, Explorer, KindFilter, NodeId, Query, ScanOptions, Selection, Tree, Visibility, filter,
    read_sizes, scan,
};

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

    let all = Visibility::all();
    let mut selection = Selection::new(&tree);
    let start = Instant::now();
    selection.toggle(&tree, &all, NodeId::ROOT);
    let toggled = start.elapsed();
    assert_eq!(selection.count(), 1_000_000);

    let start = Instant::now();
    let check = selection.check(&tree, &all, NodeId::ROOT);
    let checked = start.elapsed();
    assert_eq!(check, Check::On);

    // Under a filter: the files whose names hold a 1.
    let universe = filter(&tree, &Query::parse("1"), KindFilter::All);
    selection.clear();
    let start = Instant::now();
    selection.toggle(&tree, &universe, NodeId::ROOT);
    let check = selection.check(&tree, &universe, NodeId::ROOT);
    let filtered = start.elapsed();
    assert_eq!(check, Check::On);

    eprintln!("build {} nodes: {built:?}", tree.node_count());
    eprintln!("toggle root: {toggled:?}; check root: {checked:?}");
    eprintln!("toggle + check root under a universe: {filtered:?}");
}

/// A chain of 50 folders with 1,000,000 files at the bottom: every folder of
/// the chain spans every file.
fn deep_chain() -> Tree {
    let prefix = vec!["d"; 50].join("/");
    let owned: Vec<String> = (0..1_000_000).map(|i| format!("{prefix}/f{i}")).collect();
    let paths: Vec<&str> = owned.iter().map(String::as_str).collect();
    Tree::from_paths(&paths).expect("the chain paths are valid and distinct")
}

#[test]
#[ignore = "timing probe; run in release with --ignored --nocapture"]
#[expect(clippy::print_stderr, reason = "the probe reports its timings")]
fn checking_a_deep_chain_of_folders_is_cheap() {
    let tree = Arc::new(deep_chain());
    let mut explorer = Explorer::new(Arc::clone(&tree));
    let chain: Vec<NodeId> = tree.ids().filter(|&id| !tree.is_file(id)).skip(1).collect();
    assert_eq!(chain.len(), 50);
    // Every other file checked, so every folder is mixed.
    for file in tree.files().ones().step_by(2) {
        explorer.toggle_check(NodeId::new(u32::try_from(file).unwrap()));
    }
    // One frame asks for the state of each folder on screen.
    let frame = |explorer: &Explorer| {
        let start = Instant::now();
        for &folder in &chain {
            assert_eq!(explorer.check(folder), Check::Mixed);
        }
        start.elapsed()
    };
    let runs = 200;
    let best = |explorer: &Explorer| (0..runs).map(|_| frame(explorer)).min().unwrap();
    // The first frame after a click also rebuilds what the counts need.
    let last = NodeId::new(u32::try_from(tree.node_count() - 1).unwrap());
    let after_click = |explorer: &mut Explorer| {
        (0..runs)
            .map(|_| {
                explorer.toggle_check(last);
                frame(explorer)
            })
            .min()
            .unwrap()
    };
    let plain = (best(&explorer), after_click(&mut explorer));
    explorer.set_filter("f1", KindFilter::All);
    let filtered = (best(&explorer), after_click(&mut explorer));
    eprintln!(
        "50 folders over 1M files, best of {runs} frames: {:?} ({:?} right after a click); \
         under a filter: {:?} ({:?} right after a click)",
        plain.0, plain.1, filtered.0, filtered.1
    );
}

#[test]
#[ignore = "timing probe; set EXPLORER_SCAN_ROOT and run in release with --ignored --nocapture"]
#[expect(clippy::print_stderr, reason = "the probe reports its timings")]
fn a_real_folder_scans_with_and_without_ignored_entries() {
    let Some(root) = std::env::var_os("EXPLORER_SCAN_ROOT") else {
        return;
    };
    for show_ignored in [false, true] {
        let start = Instant::now();
        let tree = scan(
            Path::new(&root),
            ScanOptions { show_ignored },
            &AtomicBool::new(false),
        )
        .unwrap();
        let listed = start.elapsed();
        let start = Instant::now();
        read_sizes(&tree, &AtomicBool::new(false)).unwrap();
        let measured = start.elapsed();
        let files = tree.files().count_ones(..);
        eprintln!(
            "scan show_ignored={show_ignored}: {} nodes, {files} files listed in {listed:?}, sizes in {measured:?}",
            tree.node_count()
        );
    }
}
