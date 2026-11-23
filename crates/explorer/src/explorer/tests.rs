use std::sync::Arc;

use super::{Explorer, RowView};
use crate::{Check, Direction, Kind, KindFilter, NodeId, Sort, SortKey, Tree};

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
fn the_selection_and_gallery_print_briefly() {
    let mut explorer = explorer(&SAMPLE);
    explorer.toggle_check(id(&explorer, "top.txt"));
    let top = id(&explorer, "top.txt").index();
    assert!(format!("{:?}", explorer.selection()).contains(&format!("files: [{top}]")));
    assert!(format!("{:?}", explorer.gallery()).starts_with("Gallery"));
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
fn the_filter_asked_for_and_whether_one_is_shown() {
    let mut explorer = explorer(&SAMPLE);
    assert_eq!(explorer.filter_text(), "");
    assert_eq!(explorer.filter_kind(), KindFilter::All);
    assert!(!explorer.is_filter_active());
    let job = explorer.filter_job("md ", KindFilter::Text);
    // Asked for, not shown yet.
    assert_eq!(explorer.filter_text(), "md ");
    assert_eq!(explorer.filter_kind(), KindFilter::Text);
    assert!(!explorer.is_filter_active());
    assert!(explorer.apply_filter(job.run().unwrap()));
    assert!(explorer.is_filter_active());
    explorer.set_filter("", KindFilter::All);
    assert_eq!(explorer.filter_text(), "");
    assert!(!explorer.is_filter_active());
}

#[test]
fn which_folders_can_open() {
    use std::path::PathBuf;

    use crate::tree::RawTree;

    let mut explorer = explorer(&SAMPLE);
    let (docs, deep, empty) = (
        id(&explorer, "docs"),
        id(&explorer, "docs/deep"),
        id(&explorer, "empty"),
    );
    assert!(explorer.can_open(docs));
    assert!(explorer.can_open(empty));
    assert!(!explorer.can_open(id(&explorer, "top.txt")));
    assert!(!explorer.can_open(NodeId::ROOT));
    // "deep" shows `docs/deep` by its name alone: nothing below it to open.
    explorer.set_filter("deep", KindFilter::All);
    assert!(explorer.can_open(docs));
    assert!(!explorer.can_open(deep));
    assert!(!explorer.can_open(empty));

    let mut raw = RawTree::new("root", true).unwrap();
    let locked = raw.push(0, "locked", Kind::Dir).unwrap();
    raw.set_kind(locked, Kind::Unreadable);
    let tree = Arc::new(raw.build(PathBuf::from("/root")).unwrap());
    let explorer = Explorer::new(tree);
    assert!(!explorer.can_open(NodeId::new(1)));
}

#[test]
fn set_open_opens_and_closes_and_ignores_what_cannot_open() {
    let mut explorer = explorer(&SAMPLE);
    let docs = id(&explorer, "docs");
    explorer.set_open(docs, true);
    explorer.set_open(docs, true);
    assert!(explorer.is_open(docs));
    assert_eq!(rows(&explorer)[..3], ["docs", "docs/deep", "docs/a.md"]);
    explorer.set_open(id(&explorer, "top.txt"), true);
    explorer.set_open(NodeId::ROOT, false);
    assert_eq!(rows(&explorer).len(), 7);
    explorer.set_open(docs, false);
    assert_eq!(rows(&explorer), ["docs", "empty", "src", "top.txt"]);
    // Under a filter it closes and reopens only what the filter opened.
    explorer.set_filter("deep", KindFilter::All);
    assert_eq!(rows(&explorer), ["docs", "docs/deep"]);
    explorer.set_open(docs, false);
    assert_eq!(rows(&explorer), ["docs"]);
    explorer.set_open(docs, true);
    explorer.set_open(id(&explorer, "docs/deep"), true);
    assert_eq!(rows(&explorer), ["docs", "docs/deep"]);
    // The stored open folders were not touched.
    explorer.set_filter("", KindFilter::All);
    assert_eq!(rows(&explorer), ["docs", "empty", "src", "top.txt"]);
}

#[test]
fn a_row_view_holds_what_a_row_draws() {
    let tree =
        Tree::from_paths_with_sizes(&[("docs/a.md", 10), ("docs/b.MD", 5), ("top", 1)]).unwrap();
    let mut explorer = Explorer::new(Arc::new(tree));
    let docs = id(&explorer, "docs");
    explorer.toggle_folder(docs);
    explorer.toggle_check(id(&explorer, "docs/a.md"));
    let folder = explorer.row(0).unwrap();
    assert_eq!(
        folder,
        RowView {
            id: docs,
            depth: 1,
            kind: Kind::Dir,
            name: "docs",
            check: Check::Mixed,
            open: true,
            can_open: true,
            size: Some(15),
            ext: "",
        }
    );
    let file = explorer.row(2).unwrap();
    assert_eq!(
        (
            file.name,
            file.depth,
            file.check,
            file.open,
            file.can_open,
            file.size,
            file.ext
        ),
        ("b.MD", 2, Check::Off, false, false, Some(5), "md")
    );
    assert_eq!(explorer.row(3).unwrap().ext, "");
    assert_eq!(explorer.row(4), None);
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
fn a_rescan_checks_files_added_to_a_fully_checked_folder() {
    // As a saved state restores a fully checked folder: every file in it.
    let mut explorer = explorer(&["full/a.txt", "full/sub/b.txt", "part/c.txt", "part/d.txt"]);
    explorer.toggle_check(id(&explorer, "full"));
    explorer.toggle_check(id(&explorer, "part/c.txt"));
    let new = Tree::from_paths(&[
        "full/a.txt",
        "full/sub/b.txt",
        "full/sub/new.txt",
        "full/new.txt",
        "part/c.txt",
        "part/d.txt",
        "part/new.txt",
    ])
    .unwrap();
    explorer.replace_tree(Arc::new(new));
    let mut checked = gallery(&explorer);
    checked.sort();
    assert_eq!(
        checked,
        [
            "full/a.txt",
            "full/new.txt",
            "full/sub/b.txt",
            "full/sub/new.txt",
            "part/c.txt"
        ]
    );

    // Everything checked: a file added anywhere is checked too.
    explorer.select_all();
    let more = Tree::from_paths(&["full/a.txt", "part/c.txt", "elsewhere/e.txt"]).unwrap();
    explorer.replace_tree(Arc::new(more));
    assert_eq!(explorer.check(NodeId::ROOT), Check::On);
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
fn saved_paths_restore_open_folders_and_checks() {
    let mut saved = explorer(&SAMPLE);
    saved.toggle_folder(id(&saved, "docs"));
    saved.toggle_folder(id(&saved, "docs/deep"));
    saved.toggle_check(id(&saved, "docs/deep/c.png"));
    saved.toggle_check(id(&saved, "top.txt"));
    let open: Vec<String> = saved.open_paths().collect();
    let selected: Vec<String> = saved.selected_paths().collect();
    assert_eq!(open, ["docs", "docs/deep"]);
    assert_eq!(selected, ["docs/deep/c.png", "top.txt"]);

    // The folder changed meanwhile: `top.txt` is gone, `src` became a file.
    let mut restored = explorer(&["docs/a.md", "docs/deep/c.png", "src", "new.txt"]);
    restored.toggle_check(id(&restored, "new.txt"));
    let mut open = open;
    open.extend(["src".to_owned(), "missing/".to_owned(), "../x".to_owned()]);
    let mut selected = selected;
    selected.extend(["docs".to_owned(), String::new()]);
    restored.restore_open(&open);
    restored.restore_selection(selected.iter().map(String::as_str));
    assert_eq!(gallery(&restored), ["docs/deep/c.png"]);
    assert_eq!(
        rows(&restored),
        [
            "docs",
            "docs/deep",
            "docs/deep/c.png",
            "docs/a.md",
            "new.txt",
            "src"
        ]
    );
    assert_eq!(
        restored.open_paths().collect::<Vec<_>>(),
        ["docs", "docs/deep"]
    );
}

#[test]
fn a_snapshot_saves_fully_checked_folders_as_one_entry() {
    use crate::SavedSelection;

    let mut view = explorer(&[
        "a/x.txt",
        "a/y.txt",
        "b/deep/1.txt",
        "b/deep/2.txt",
        "b/z.txt",
        "c/empty/",
        "top.txt",
    ]);
    let none = view.snapshot();
    assert!(none.selection().is_empty());
    assert!(format!("{none:?}").contains("selected: 0"));

    view.toggle_check(id(&view, "a"));
    view.toggle_check(id(&view, "b/deep"));
    view.toggle_check(id(&view, "top.txt"));
    view.toggle_folder(id(&view, "b"));
    let snapshot = view.snapshot();
    assert_eq!(
        snapshot.selection(),
        SavedSelection {
            folders: vec!["a".into(), "b/deep".into()],
            files: vec!["top.txt".into()],
        }
    );
    assert_eq!(snapshot.open_paths(), ["b"]);
    assert!(Arc::ptr_eq(snapshot.tree(), view.tree()));

    // A change after the snapshot does not reach it.
    view.select_all();
    assert_eq!(snapshot.selection().folders.len(), 2);
    // Everything checked is the root alone.
    assert_eq!(
        view.snapshot().selection(),
        SavedSelection {
            folders: vec![String::new()],
            files: vec![],
        }
    );
}

#[test]
fn a_compact_selection_restores_the_same_checks() {
    let paths = [
        "a/x.txt",
        "a/y.txt",
        "b/deep/1.txt",
        "b/deep/2.txt",
        "b/z.txt",
        "top.txt",
    ];
    for pick in [
        &["a"][..],
        &["b/deep/1.txt", "top.txt"],
        &["a", "b"],
        &["b/deep", "b/z.txt", "a/x.txt"],
    ] {
        let mut saved = explorer(&paths);
        for path in pick {
            saved.toggle_check(id(&saved, path));
        }
        let compact = saved.snapshot().selection();
        let mut restored = explorer(&paths);
        restored.toggle_check(id(&restored, "a/y.txt"));
        restored.restore_saved_selection(&compact);
        assert_eq!(gallery(&restored), gallery(&saved), "{pick:?}");
    }
    // Everything, through the root's entry.
    let mut saved = explorer(&paths);
    saved.select_all();
    let mut restored = explorer(&paths);
    restored.restore_saved_selection(&saved.snapshot().selection());
    assert_eq!(restored.selection().count(), paths.len());
}

#[test]
fn a_saved_folder_checks_what_it_holds_when_restored() {
    use crate::SavedSelection;

    let mut restored = explorer(&["a/old.txt", "a/new.txt", "a/sub/more.txt", "b.txt", "c/"]);
    restored.restore_saved_selection(&SavedSelection {
        // A file named as a folder, a folder named as a file, an empty
        // folder and paths that are gone are skipped.
        folders: vec!["a".into(), "b.txt".into(), "c".into(), "gone".into()],
        files: vec!["c".into(), "gone.txt".into()],
    });
    assert_eq!(
        gallery(&restored),
        ["a/sub/more.txt", "a/new.txt", "a/old.txt"]
    );
    assert!(!restored.is_selected(id(&restored, "b.txt")));
}

#[test]
fn open_paths_are_the_stored_open_folders_only() {
    use std::path::PathBuf;

    use crate::tree::RawTree;

    let mut raw = RawTree::new("root", true).unwrap();
    let dir = raw.push(0, "dir", Kind::Dir).unwrap();
    raw.push(dir, "f.txt", Kind::File).unwrap();
    let locked = raw.push(0, "locked", Kind::Dir).unwrap();
    raw.set_kind(locked, Kind::Unreadable);
    let mut explorer = Explorer::new(Arc::new(raw.build(PathBuf::from("/root")).unwrap()));
    explorer.expand_all();
    assert_eq!(explorer.open_paths().collect::<Vec<_>>(), ["dir"]);
    // Closing under a filter is temporary and not saved.
    explorer.set_filter("f", KindFilter::All);
    explorer.collapse_all();
    assert_eq!(explorer.open_paths().collect::<Vec<_>>(), ["dir"]);
}

#[test]
fn the_file_on_the_stage_follows_its_path_or_the_nearest_surviving_gallery_file() {
    const PATHS: [&str; 5] = ["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"];
    /// Replaces the tree with `left` and returns where `current` went.
    fn replace(view: &mut Explorer, current: &str, left: &[&str]) -> Option<String> {
        let current = id(view, current);
        let new = Arc::new(Tree::from_paths(left).unwrap());
        let (map, carried) = view.replace_tree_carrying(new, current);
        if let Some(same) = map.get(current) {
            assert_eq!(carried, Some(same));
        }
        carried.map(|node| view.tree().path(node))
    }

    // Still there, checked or not: the same path.
    let mut view = explorer(&PATHS);
    assert_eq!(
        replace(&mut view, "c.txt", &PATHS).as_deref(),
        Some("c.txt")
    );

    // Display order e d c b a; everything but `c` checked.
    let mut view = explorer(&PATHS);
    for path in ["a.txt", "b.txt", "d.txt", "e.txt"] {
        view.toggle_check(id(&view, path));
    }
    view.set_sort(Sort {
        key: SortKey::Name,
        direction: Direction::Descending,
    });
    // An unchecked file goes to the next gallery file after it. `u`, new
    // and never checked, keeps the root partly checked, so files that
    // appear stay unchecked.
    assert_eq!(
        replace(
            &mut view,
            "c.txt",
            &["a.txt", "b.txt", "d.txt", "e.txt", "u.txt"]
        )
        .as_deref(),
        Some("b.txt")
    );
    // `d` is deleted with `b`: the next survivor after it is `a`.
    assert_eq!(
        replace(&mut view, "d.txt", &["a.txt", "e.txt", "u.txt"]).as_deref(),
        Some("a.txt")
    );
    // Nothing after `a` survives: the last survivor before it.
    assert_eq!(
        replace(&mut view, "a.txt", &["e.txt", "u.txt", "z.txt"]).as_deref(),
        Some("e.txt")
    );
    // No gallery file survives: `z` is not checked.
    assert_eq!(replace(&mut view, "e.txt", &["z.txt"]), None);
}

#[test]
fn a_size_sort_is_redone_when_sizes_arrive() {
    use std::path::PathBuf;

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

mod props;
