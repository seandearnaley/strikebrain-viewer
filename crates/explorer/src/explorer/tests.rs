use std::sync::Arc;

use proptest::prelude::*;

use super::Explorer;
use crate::sort::DisplayOrder;
use crate::tree::tests::{path_sets, tree_of};
use crate::{Check, Direction, KindFilter, NodeId, Sort, SortKey, Tree};

fn explorer(paths: &[&str]) -> Explorer {
    Explorer::new(Arc::new(Tree::from_paths(paths).unwrap()))
}

fn id(explorer: &Explorer, path: &str) -> NodeId {
    explorer
        .tree()
        .find(path)
        .unwrap_or_else(|| panic!("no node at {path:?}"))
}

fn rows(explorer: &Explorer) -> Vec<String> {
    let tree = explorer.tree();
    explorer.rows().iter().map(|&row| tree.path(row)).collect()
}

fn gallery(explorer: &Explorer) -> Vec<String> {
    let tree = explorer.tree();
    explorer.gallery().map(|file| tree.path(file)).collect()
}

const SAMPLE: [&str; 6] = [
    "docs/a.md",
    "docs/b.md",
    "docs/deep/c.png",
    "empty/",
    "src/main.rs",
    "top.txt",
];

#[test]
fn a_new_explorer_shows_the_top_level_closed() {
    let explorer = explorer(&SAMPLE);
    assert_eq!(rows(&explorer), ["docs", "empty", "src", "top.txt"]);
    assert_eq!(explorer.sort(), Sort::default());
    assert!(!explorer.visibility().is_active());
    assert_eq!(explorer.gallery_len(), 0);
    assert!(format!("{explorer:?}").contains("rows: 4"));
}

#[test]
fn the_chevron_opens_and_closes_a_folder() {
    let mut explorer = explorer(&SAMPLE);
    let docs = id(&explorer, "docs");
    explorer.toggle_folder(docs);
    assert!(explorer.is_open(docs));
    assert_eq!(
        rows(&explorer),
        [
            "docs",
            "docs/deep",
            "docs/a.md",
            "docs/b.md",
            "empty",
            "src",
            "top.txt"
        ]
    );
    assert_eq!(explorer.row_of(id(&explorer, "docs/b.md")), Some(3));
    explorer.toggle_folder(docs);
    assert!(!explorer.is_open(docs));
    assert_eq!(rows(&explorer), ["docs", "empty", "src", "top.txt"]);
    assert_eq!(explorer.row_of(id(&explorer, "docs/b.md")), None);
}

#[test]
fn files_and_the_root_do_not_open() {
    let mut explorer = explorer(&SAMPLE);
    explorer.toggle_folder(id(&explorer, "top.txt"));
    explorer.toggle_folder(NodeId::ROOT);
    assert!(!explorer.is_open(id(&explorer, "top.txt")));
    assert_eq!(rows(&explorer), ["docs", "empty", "src", "top.txt"]);
}

#[test]
fn expand_all_and_collapse_all() {
    let mut explorer = explorer(&SAMPLE);
    explorer.expand_all();
    assert_eq!(explorer.rows().len(), explorer.tree().node_count() - 1);
    assert!(explorer.is_open(id(&explorer, "docs/deep")));
    explorer.collapse_all();
    assert_eq!(rows(&explorer), ["docs", "empty", "src", "top.txt"]);
}

#[test]
fn a_mixed_folder_completes_on_click() {
    let mut explorer = explorer(&SAMPLE);
    explorer.toggle_check(id(&explorer, "docs/a.md"));
    let docs = id(&explorer, "docs");
    assert_eq!(explorer.check(docs), Check::Mixed);
    explorer.toggle_check(docs);
    assert_eq!(explorer.check(docs), Check::On);
    assert_eq!(explorer.gallery_len(), 3);
}

#[test]
fn a_fully_checked_folder_clears_on_click() {
    let mut explorer = explorer(&SAMPLE);
    let docs = id(&explorer, "docs");
    explorer.toggle_check(docs);
    explorer.toggle_check(id(&explorer, "top.txt"));
    explorer.toggle_check(docs);
    assert_eq!(explorer.check(docs), Check::Off);
    assert_eq!(gallery(&explorer), ["top.txt"]);
    assert!(explorer.is_selected(id(&explorer, "top.txt")));
    assert_eq!(explorer.selection().count(), 1);
}

#[test]
fn select_all_under_a_filter_selects_only_matches() {
    let mut explorer = explorer(&SAMPLE);
    explorer.set_filter("md", KindFilter::All);
    explorer.select_all();
    assert_eq!(gallery(&explorer), ["docs/a.md", "docs/b.md"]);
    assert_eq!(explorer.check(NodeId::ROOT), Check::On);
    explorer.clear_selection();
    assert_eq!(explorer.gallery_len(), 0);
}

#[test]
fn unchecking_a_folder_under_a_filter_clears_hidden_files_too() {
    let mut explorer = explorer(&SAMPLE);
    let docs = id(&explorer, "docs");
    explorer.toggle_check(docs);
    explorer.set_filter("a.md", KindFilter::All);
    assert_eq!(explorer.check(docs), Check::On);
    explorer.toggle_check(docs);
    assert_eq!(explorer.gallery_len(), 0);
}

#[test]
fn folders_sort_before_files_in_both_directions() {
    let mut explorer = explorer(&["b.txt", "a/", "c/", "D.txt"]);
    explorer.set_sort(Sort {
        key: SortKey::Name,
        direction: Direction::Descending,
    });
    assert_eq!(rows(&explorer), ["c", "a", "D.txt", "b.txt"]);
    explorer.set_sort(Sort::default());
    assert_eq!(rows(&explorer), ["a", "c", "b.txt", "D.txt"]);
}

#[test]
fn file2_sorts_before_file10() {
    let explorer = explorer(&["file10.txt", "file2.txt", "file1.txt"]);
    assert_eq!(rows(&explorer), ["file1.txt", "file2.txt", "file10.txt"]);
}

#[test]
fn the_filter_finds_a_file_inside_a_closed_folder_and_shows_its_ancestors_open() {
    let mut explorer = explorer(&["deep/er/target.txt", "deep/er/other.txt", "side/x.txt"]);
    assert_eq!(rows(&explorer), ["deep", "side"]);
    explorer.set_filter("target", KindFilter::All);
    assert_eq!(rows(&explorer), ["deep", "deep/er", "deep/er/target.txt"]);
    assert!(explorer.is_open(id(&explorer, "deep")));
    // The stored open folders come back when the filter clears.
    explorer.set_filter("", KindFilter::All);
    assert_eq!(rows(&explorer), ["deep", "side"]);
}

#[test]
fn every_include_term_must_match() {
    let mut explorer = explorer(&["red apple.txt", "red.txt", "apple.txt"]);
    explorer.set_filter("apple red", KindFilter::All);
    assert_eq!(rows(&explorer), ["red apple.txt"]);
}

#[test]
fn closing_a_folder_under_a_filter_lasts_until_the_query_or_kind_changes() {
    let mut explorer = explorer(&["a/x.md", "a/y.md", "b/x.md"]);
    let (a, b) = (id(&explorer, "a"), id(&explorer, "b"));
    explorer.toggle_folder(b);
    explorer.set_filter("x", KindFilter::All);
    assert_eq!(rows(&explorer), ["a", "a/x.md", "b", "b/x.md"]);
    explorer.toggle_folder(a);
    assert!(!explorer.is_open(a));
    assert_eq!(rows(&explorer), ["a", "b", "b/x.md"]);
    // The same query again keeps the closed folder.
    explorer.set_filter("x", KindFilter::All);
    assert_eq!(rows(&explorer), ["a", "b", "b/x.md"]);
    // A different query, or kind, reopens it.
    explorer.set_filter("x.", KindFilter::All);
    assert_eq!(rows(&explorer), ["a", "a/x.md", "b", "b/x.md"]);
    explorer.toggle_folder(a);
    explorer.set_filter("x.", KindFilter::Text);
    assert_eq!(rows(&explorer), ["a", "a/x.md", "b", "b/x.md"]);
    // A folder the filter did not open does not toggle.
    explorer.set_filter("a", KindFilter::All);
    assert_eq!(rows(&explorer), ["a"]);
    explorer.toggle_folder(a);
    assert_eq!(rows(&explorer), ["a"]);
    // The stored open folders were never touched.
    explorer.set_filter("", KindFilter::All);
    assert_eq!(rows(&explorer), ["a", "b", "b/x.md"]);
}

#[test]
fn expand_and_collapse_all_under_a_filter_are_temporary() {
    let mut explorer = explorer(&["a/x.txt", "b/x.txt"]);
    explorer.set_filter("x", KindFilter::All);
    explorer.collapse_all();
    assert_eq!(rows(&explorer), ["a", "b"]);
    explorer.expand_all();
    assert_eq!(rows(&explorer), ["a", "a/x.txt", "b", "b/x.txt"]);
    explorer.collapse_all();
    explorer.set_filter("", KindFilter::All);
    assert_eq!(rows(&explorer), ["a", "b"]);
    explorer.expand_all();
    explorer.set_filter("x", KindFilter::All);
    explorer.set_filter("", KindFilter::All);
    assert_eq!(rows(&explorer), ["a", "a/x.txt", "b", "b/x.txt"]);
}

#[test]
fn reveal_opens_the_folders_above_a_node() {
    let mut explorer = explorer(&SAMPLE);
    let deep = id(&explorer, "docs/deep/c.png");
    assert_eq!(explorer.reveal(deep), Some(2));
    assert_eq!(
        rows(&explorer)[..3],
        ["docs", "docs/deep", "docs/deep/c.png"]
    );
    // Already on screen: nothing changes.
    assert_eq!(explorer.reveal(deep), Some(2));
    assert_eq!(explorer.reveal(id(&explorer, "top.txt")), Some(7));
    assert_eq!(explorer.reveal(NodeId::ROOT), None);
}

#[test]
fn reveal_under_a_filter_reopens_closed_folders_and_skips_hidden_nodes() {
    let mut explorer = explorer(&SAMPLE);
    explorer.set_filter("c.png", KindFilter::All);
    explorer.collapse_all();
    assert_eq!(rows(&explorer), ["docs"]);
    assert_eq!(explorer.reveal(id(&explorer, "docs/deep/c.png")), Some(2));
    assert_eq!(rows(&explorer), ["docs", "docs/deep", "docs/deep/c.png"]);
    assert_eq!(explorer.reveal(id(&explorer, "top.txt")), None);
}

#[test]
fn a_stale_filter_result_is_rejected() {
    let mut explorer = explorer(&SAMPLE);
    let first = explorer.filter_job("md", KindFilter::All).run().unwrap();
    let second = explorer.filter_job("rs", KindFilter::All);
    // The first job was made before the second, so its result is stale.
    assert!(!explorer.apply_filter(first));
    assert_eq!(rows(&explorer), ["docs", "empty", "src", "top.txt"]);
    let second = second.run().unwrap();
    assert!(explorer.apply_filter(second.clone()));
    assert_eq!(rows(&explorer), ["src", "src/main.rs"]);
    // A synchronous filter supersedes it too.
    explorer.set_filter("top", KindFilter::All);
    assert!(!explorer.apply_filter(second));
    assert_eq!(rows(&explorer), ["top.txt"]);
}

#[test]
fn a_job_runs_on_another_thread() {
    let mut explorer = explorer(&SAMPLE);
    let job = explorer.filter_job("md", KindFilter::All);
    let result = std::thread::spawn(move || job.run())
        .join()
        .unwrap()
        .unwrap();
    assert!(explorer.apply_filter(result));
    assert_eq!(rows(&explorer), ["docs", "docs/a.md", "docs/b.md"]);
}

#[test]
fn a_superseded_job_stops_without_a_result() {
    let mut explorer = explorer(&SAMPLE);
    let old = explorer.filter_job("md", KindFilter::All);
    let new = explorer.filter_job("md", KindFilter::Images);
    assert!(old.run().is_none());
    assert!(new.run().is_some());
}

#[test]
fn replacing_the_tree_carries_checks_and_open_folders_by_path() {
    let mut explorer = explorer(&SAMPLE);
    explorer.toggle_folder(id(&explorer, "docs"));
    explorer.toggle_check(id(&explorer, "docs/b.md"));
    explorer.toggle_check(id(&explorer, "top.txt"));
    let old_b = id(&explorer, "docs/b.md");

    let new = Tree::from_paths(&["docs/b.md", "docs/new.md", "src/main.rs", "zz.txt"]).unwrap();
    let map = explorer.replace_tree(Arc::new(new));
    assert_eq!(map.get(old_b), Some(id(&explorer, "docs/b.md")));
    assert_eq!(map.get(NodeId::new(999)), None);
    assert_eq!(gallery(&explorer), ["docs/b.md"]);
    assert_eq!(
        rows(&explorer),
        ["docs", "docs/b.md", "docs/new.md", "src", "zz.txt"]
    );
}

#[test]
fn a_filter_result_for_the_old_tree_is_rejected() {
    let mut explorer = explorer(&SAMPLE);
    let stale = explorer.filter_job("md", KindFilter::All).run().unwrap();
    let new = Tree::from_paths(&["a.md", "b.rs"]).unwrap();
    explorer.replace_tree(Arc::new(new));
    // The query asked for applies to the new tree; the old result does not.
    assert_eq!(rows(&explorer), ["a.md"]);
    assert!(!explorer.apply_filter(stale));
}

#[test]
fn a_filter_result_from_another_explorer_is_rejected() {
    let mut a = explorer(&["x/1.txt", "x/2.txt", "y.txt"]);
    let job = a.filter_job("1", KindFilter::All);
    let owned: Vec<String> = (0..200).map(|i| format!("big/f{i}.txt")).collect();
    let paths: Vec<&str> = owned.iter().map(String::as_str).collect();
    let mut b = explorer(&paths);
    let _pending = b.filter_job("f1", KindFilter::All);
    // Both explorers are at their first generation.
    assert!(!b.apply_filter(job.run().unwrap()));
    assert!(!b.visibility().is_active());
    assert_eq!(rows(&b), ["big"]);
    assert_eq!(b.check(NodeId::ROOT), Check::Off);
}

#[test]
fn a_result_for_a_previous_tree_is_rejected_whenever_it_is_run() {
    let mut explorer = explorer(&SAMPLE);
    let finished = explorer.filter_job("md", KindFilter::All).run().unwrap();
    let pending = explorer.filter_job("rs", KindFilter::All);
    explorer.replace_tree(Arc::new(Tree::from_paths(&["a.md", "b.rs"]).unwrap()));
    assert!(!explorer.apply_filter(finished));
    // A job made for the old tree and run after the swap gives nothing.
    assert!(pending.run().is_none());
    // A job made for the new tree applies.
    let fresh = explorer.filter_job("md", KindFilter::All).run().unwrap();
    assert!(explorer.apply_filter(fresh));
    assert_eq!(rows(&explorer), ["a.md"]);
}

#[test]
fn replacing_the_tree_reruns_the_filter_last_asked_for() {
    let mut explorer = explorer(&["a/x.md", "b/y.md", "c.txt"]);
    explorer.set_filter("md", KindFilter::All);
    explorer.toggle_folder(id(&explorer, "a"));
    assert_eq!(rows(&explorer), ["a", "b", "b/y.md"]);
    // Asked for but not applied yet when the new tree arrives.
    let pending = explorer.filter_job("y", KindFilter::All);
    let new = Tree::from_paths(&["a/x.md", "b/y.md", "b/z.md", "c.txt"]).unwrap();
    explorer.replace_tree(Arc::new(new));
    assert!(pending.run().is_none());
    assert_eq!(rows(&explorer), ["b", "b/y.md"]);
    // With the applied query unchanged, the closed folder stays closed.
    explorer.set_filter("md", KindFilter::All);
    explorer.toggle_folder(id(&explorer, "a"));
    let newer = Tree::from_paths(&["a/x.md", "a/w.md", "b/y.md"]).unwrap();
    explorer.replace_tree(Arc::new(newer));
    assert_eq!(rows(&explorer), ["a", "b", "b/y.md"]);
}

#[test]
fn a_size_sort_is_redone_when_sizes_arrive() {
    use std::path::PathBuf;

    use crate::Kind;
    use crate::tree::RawTree;

    let mut raw = RawTree::new("root", false).unwrap();
    raw.push(0, "a", Kind::File).unwrap();
    raw.push(0, "b", Kind::File).unwrap();
    let tree = Arc::new(raw.build(PathBuf::from("/root")).unwrap());
    let mut explorer = Explorer::new(Arc::clone(&tree));
    explorer.set_sort(Sort {
        key: SortKey::Size,
        direction: Direction::Descending,
    });
    assert_eq!(rows(&explorer), ["a", "b"]);
    tree.set_sizes(vec![0, 1, 2]);
    explorer.sizes_loaded();
    assert_eq!(rows(&explorer), ["b", "a"]);
    explorer.set_sort(Sort::default());
    explorer.sizes_loaded();
    assert_eq!(rows(&explorer), ["a", "b"]);
}

#[test]
fn a_size_sort_catches_up_on_the_next_change_without_being_told() {
    use std::path::PathBuf;

    use crate::Kind;
    use crate::tree::RawTree;

    let mut raw = RawTree::new("root", false).unwrap();
    let dir = raw.push(0, "dir", Kind::Dir).unwrap();
    raw.push(dir, "inner", Kind::File).unwrap();
    raw.push(0, "a", Kind::File).unwrap();
    raw.push(0, "b", Kind::File).unwrap();
    let tree = Arc::new(raw.build(PathBuf::from("/root")).unwrap());
    let mut explorer = Explorer::new(Arc::clone(&tree));
    explorer.set_sort(Sort {
        key: SortKey::Size,
        direction: Direction::Descending,
    });
    assert_eq!(rows(&explorer), ["dir", "a", "b"]);
    // Ids: root 0, dir 1, inner 2, a 3, b 4.
    tree.set_sizes(vec![0, 0, 1, 2, 3]);
    // The app forgot `sizes_loaded`; any change to the view catches up.
    explorer.toggle_folder(id(&explorer, "dir"));
    assert_eq!(rows(&explorer), ["dir", "dir/inner", "b", "a"]);
}

#[test]
fn the_gallery_follows_display_order_and_ignores_the_filter() {
    let mut explorer = explorer(&SAMPLE);
    explorer.select_all();
    explorer.set_filter("rs", KindFilter::All);
    assert_eq!(
        gallery(&explorer),
        [
            "docs/deep/c.png",
            "docs/a.md",
            "docs/b.md",
            "src/main.rs",
            "top.txt"
        ]
    );
    explorer.set_sort(Sort {
        key: SortKey::Name,
        direction: Direction::Descending,
    });
    assert_eq!(
        gallery(&explorer),
        [
            "src/main.rs",
            "docs/deep/c.png",
            "docs/b.md",
            "docs/a.md",
            "top.txt"
        ]
    );
    assert_eq!(explorer.gallery_len(), 5);
    assert_eq!(
        explorer.gallery_position(id(&explorer, "docs/b.md")),
        Some(2)
    );
}

#[test]
fn next_and_previous_work_from_any_node() {
    let mut explorer = explorer(&SAMPLE);
    let (a, c, main, top) = (
        id(&explorer, "docs/a.md"),
        id(&explorer, "docs/deep/c.png"),
        id(&explorer, "src/main.rs"),
        id(&explorer, "top.txt"),
    );
    explorer.toggle_check(c);
    explorer.toggle_check(top);
    assert_eq!(explorer.gallery_next(c), Some(top));
    assert_eq!(explorer.gallery_previous(top), Some(c));
    assert_eq!(explorer.gallery_next(top), None);
    assert_eq!(explorer.gallery_previous(c), None);
    // `a.md` and `main.rs` are not in the gallery: neighbours by display order.
    assert_eq!(explorer.gallery_position(a), None);
    assert_eq!(explorer.gallery_next(a), Some(top));
    assert_eq!(explorer.gallery_previous(a), Some(c));
    assert_eq!(explorer.gallery_previous(main), Some(c));
    assert_eq!(explorer.gallery_next(NodeId::ROOT), Some(c));
}

/// One step a user can take.
#[derive(Clone, Debug)]
enum Step {
    Folder(usize),
    Check(usize),
    Filter(&'static str, KindFilter),
    Sort(SortKey, Direction),
    ExpandAll,
    CollapseAll,
    SelectAll,
    Reveal(usize),
    /// Asks the gallery questions the stage asks on every render, so any
    /// cached answer is made mid-sequence and must not go stale.
    AskGallery(usize),
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    let kind = prop::sample::select(&[KindFilter::All, KindFilter::Images, KindFilter::Text][..]);
    let key = prop::sample::select(&[SortKey::Name, SortKey::Type, SortKey::Size][..]);
    let direction = prop::sample::select(&[Direction::Ascending, Direction::Descending][..]);
    let text = prop::sample::select(&["", "src", "a", ".md", "-src", "e b", "-x.d", "2"][..]);
    let step = prop_oneof![
        3 => (0_usize..80).prop_map(Step::Folder),
        3 => (0_usize..80).prop_map(Step::Check),
        2 => (text, kind).prop_map(|(t, k)| Step::Filter(t, k)),
        1 => (key, direction).prop_map(|(k, d)| Step::Sort(k, d)),
        1 => Just(Step::ExpandAll),
        1 => Just(Step::CollapseAll),
        1 => Just(Step::SelectAll),
        1 => (0_usize..80).prop_map(Step::Reveal),
        2 => (0_usize..80).prop_map(Step::AskGallery),
    ];
    prop::collection::vec(step, 0..12)
}

fn node(explorer: &Explorer, index: usize) -> NodeId {
    NodeId::new(u32::try_from(index % explorer.tree().node_count()).unwrap())
}

fn play(explorer: &mut Explorer, steps: &[Step]) {
    for step in steps {
        match *step {
            Step::Folder(i) => explorer.toggle_folder(node(explorer, i)),
            Step::Check(i) => explorer.toggle_check(node(explorer, i)),
            Step::Filter(text, kind) => explorer.set_filter(text, kind),
            Step::Sort(key, direction) => explorer.set_sort(Sort { key, direction }),
            Step::ExpandAll => explorer.expand_all(),
            Step::CollapseAll => explorer.collapse_all(),
            Step::SelectAll => explorer.select_all(),
            Step::Reveal(i) => {
                let target = node(explorer, i);
                if let Some(row) = explorer.reveal(target) {
                    assert_eq!(explorer.rows()[row], target);
                }
            }
            Step::AskGallery(i) => {
                let target = node(explorer, i);
                std::hint::black_box((
                    explorer.gallery_position(target),
                    explorer.gallery_next(target),
                    explorer.gallery_previous(target),
                    explorer.gallery().count(),
                ));
            }
        }
    }
}

/// The rows by recursion: a child shows when visible, and its children
/// follow when it is open.
fn naive_rows(explorer: &Explorer, order: &DisplayOrder, node: NodeId, out: &mut Vec<NodeId>) {
    for &child in order.children(explorer.tree(), node) {
        if explorer.visibility().is_visible(child) {
            out.push(child);
            if explorer.is_open(child) {
                naive_rows(explorer, order, child, out);
            }
        }
    }
}

/// Every checked file, depth first over the display order.
fn naive_gallery(explorer: &Explorer, order: &DisplayOrder, node: NodeId, out: &mut Vec<NodeId>) {
    if explorer.is_selected(node) {
        out.push(node);
    }
    for &child in order.children(explorer.tree(), node) {
        naive_gallery(explorer, order, child, out);
    }
}

proptest! {
    #[test]
    fn rows_match_a_naive_walk_after_any_steps(set in path_sets(), steps in steps()) {
        let mut explorer = Explorer::new(Arc::new(tree_of(&set)));
        play(&mut explorer, &steps);
        let order = DisplayOrder::new(explorer.tree(), explorer.sort());
        let mut expected = Vec::new();
        naive_rows(&explorer, &order, NodeId::ROOT, &mut expected);
        prop_assert_eq!(explorer.rows(), &expected[..]);
    }

    #[test]
    fn the_gallery_matches_a_naive_walk(set in path_sets(), steps in steps()) {
        let mut explorer = Explorer::new(Arc::new(tree_of(&set)));
        play(&mut explorer, &steps);
        let order = DisplayOrder::new(explorer.tree(), explorer.sort());
        let mut expected = Vec::new();
        naive_gallery(&explorer, &order, NodeId::ROOT, &mut expected);
        let got: Vec<NodeId> = explorer.gallery().collect();
        prop_assert_eq!(&got, &expected);
        prop_assert_eq!(explorer.gallery_len(), expected.len());
        let mut all = Vec::new();
        naive_pre_order(&explorer, &order, NodeId::ROOT, &mut all);
        for (at, &node) in all.iter().enumerate() {
            prop_assert_eq!(
                explorer.gallery_position(node),
                expected.iter().position(|&file| file == node)
            );
            let after = all[at + 1..].iter().copied().find(|&n| explorer.is_selected(n));
            let before = all[..at].iter().rev().copied().find(|&n| explorer.is_selected(n));
            prop_assert_eq!(explorer.gallery_next(node), after);
            prop_assert_eq!(explorer.gallery_previous(node), before);
        }
    }
}

fn naive_pre_order(explorer: &Explorer, order: &DisplayOrder, node: NodeId, out: &mut Vec<NodeId>) {
    out.push(node);
    for &child in order.children(explorer.tree(), node) {
        naive_pre_order(explorer, order, child, out);
    }
}
