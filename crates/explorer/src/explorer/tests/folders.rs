//! Several folders: adding, rescanning and removing one, and saving each.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::tree::tests::folder_at;
use crate::{
    Check, Direction, Explorer, KindFilter, NodeId, SavedFolder, SavedSelection, Sort, SortKey,
    Tree,
};

fn folder(root: &str, paths: &[&str]) -> Arc<Tree> {
    let sized: Vec<(&str, u64)> = paths.iter().map(|&path| (path, 1)).collect();
    Arc::new(folder_at(root, &sized))
}

fn rows(explorer: &Explorer) -> Vec<String> {
    let tree = explorer.tree();
    explorer.rows().iter().map(|&row| tree.path(row)).collect()
}

fn gallery(explorer: &Explorer) -> Vec<String> {
    let tree = explorer.tree();
    explorer.gallery().map(|file| tree.path(file)).collect()
}

fn at(explorer: &Explorer, path: &str) -> NodeId {
    explorer
        .tree()
        .find_abs(Path::new(path))
        .unwrap_or_else(|| panic!("no node at {path:?}"))
}

/// An explorer of `/w/b` and then `/w/a`, both open.
fn two_folders() -> Explorer {
    let mut explorer = Explorer::empty();
    explorer
        .add_folder(&folder("/w/b", &["z.txt", "src/x.rs"]), 0, None)
        .unwrap();
    explorer
        .add_folder(&folder("/w/a", &["readme.md", "docs/guide.md"]), 9, None)
        .unwrap();
    explorer
}

#[test]
fn folders_keep_the_order_they_were_added_in_and_open_when_added() {
    let explorer = Explorer::empty();
    assert_eq!(explorer.rows(), &[] as &[NodeId]);
    assert_eq!(explorer.check(NodeId::ROOT), Check::Off);
    let mut explorer = two_folders();
    assert_eq!(
        rows(&explorer),
        ["b", "b/src", "b/z.txt", "a", "a/docs", "a/readme.md"]
    );
    let b = at(&explorer, "/w/b");
    assert_eq!(explorer.row(0).map(|row| row.depth), Some(1));
    assert!(explorer.is_open(b));
    assert!(explorer.can_open(b));
    assert_eq!(
        explorer.folder_at(Path::new("/w/a")),
        Some(at(&explorer, "/w/a"))
    );
    assert_eq!(explorer.folder_at(Path::new("/w/a/docs")), None);

    // A sort reorders inside each folder; the folders stay as added.
    explorer.set_sort(Sort {
        key: SortKey::Name,
        direction: Direction::Descending,
    });
    assert_eq!(
        rows(&explorer),
        ["b", "b/src", "b/z.txt", "a", "a/docs", "a/readme.md"]
    );
    explorer.set_sort(Sort {
        key: SortKey::Type,
        direction: Direction::Ascending,
    });
    assert_eq!(rows(&explorer)[0], "b");

    // One added before the first goes first.
    explorer
        .add_folder(&folder("/w/c", &["c.txt"]), 0, None)
        .unwrap();
    assert_eq!(rows(&explorer)[..3], ["c", "c/c.txt", "b"]);

    // Collapse all closes the folders too; reveal opens them again.
    explorer.collapse_all();
    assert_eq!(rows(&explorer), ["c", "b", "a"]);
    let guide = at(&explorer, "/w/a/docs/guide.md");
    assert_eq!(explorer.reveal(guide), Some(4));
}

#[test]
fn a_folder_with_no_match_hides_like_any_other_folder_row() {
    let mut explorer = two_folders();
    explorer.set_filter("x.rs", KindFilter::All);
    assert_eq!(rows(&explorer), ["b", "b/src", "b/src/x.rs"]);
    // A folder whose own name matches shows, as any folder does.
    explorer.set_filter("a", KindFilter::All);
    assert!(rows(&explorer).contains(&"a".to_owned()));
    explorer.set_filter("-b", KindFilter::All);
    assert!(!rows(&explorer).contains(&"b".to_owned()));
}

#[test]
fn removing_a_folder_takes_its_checks_and_moves_the_stage_on() {
    let mut explorer = two_folders();
    let (x, z, guide) = (
        at(&explorer, "/w/b/src/x.rs"),
        at(&explorer, "/w/b/z.txt"),
        at(&explorer, "/w/a/docs/guide.md"),
    );
    for file in [x, z, guide] {
        explorer.toggle_check(file);
    }
    let b = at(&explorer, "/w/b");
    let (map, current) = explorer.remove_folder(b, Some(z));
    assert_eq!(rows(&explorer), ["a", "a/docs", "a/readme.md"]);
    assert_eq!(gallery(&explorer), ["a/docs/guide.md"]);
    assert_eq!(map.get(x), None);
    assert_eq!(current, Some(at(&explorer, "/w/a/docs/guide.md")));

    let a = at(&explorer, "/w/a");
    let (_, current) = explorer.remove_folder(a, current);
    assert_eq!(current, None);
    assert_eq!(explorer.rows(), &[] as &[NodeId]);
    assert!(explorer.tree().is_joined());
    assert_eq!(explorer.selection().count(), 0);
}

#[test]
fn a_rescan_of_one_folder_carries_its_view_and_leaves_the_others_alone() {
    let mut explorer = two_folders();
    let src = at(&explorer, "/w/b/src");
    explorer.toggle_check(src);
    explorer.toggle_check(at(&explorer, "/w/a/readme.md"));
    explorer.toggle_folder(at(&explorer, "/w/a/docs"));
    let readme = at(&explorer, "/w/a/readme.md");

    let b = at(&explorer, "/w/b");
    let rescan = folder("/w/b", &["z.txt", "src/x.rs", "src/y.rs", "new/n.txt"]);
    let (map, current) = explorer.replace_folder(b, &rescan, Some(readme)).unwrap();
    assert_eq!(map.get(b), Some(at(&explorer, "/w/b")));
    assert_eq!(current, Some(at(&explorer, "/w/a/readme.md")));
    // The fully checked folder takes the file added to it.
    assert_eq!(
        gallery(&explorer),
        ["b/src/x.rs", "b/src/y.rs", "a/readme.md"]
    );
    assert_eq!(
        rows(&explorer),
        [
            "b",
            "b/new",
            "b/src",
            "b/z.txt",
            "a",
            "a/docs",
            "a/docs/guide.md",
            "a/readme.md"
        ]
    );

    // A folder that cannot be read keeps its place, as one node.
    let a = at(&explorer, "/w/a");
    let locked = Arc::new(Tree::unreadable(Path::new("/w/a")));
    let (_, current) = explorer.replace_folder(a, &locked, current).unwrap();
    assert_eq!(rows(&explorer)[4..], ["a"]);
    // The file on the stage went with it: the checked file before it.
    assert_eq!(current, Some(at(&explorer, "/w/b/src/y.rs")));
    assert!(!explorer.can_open(at(&explorer, "/w/a")));
}

#[test]
fn a_folder_added_around_open_folders_takes_them_in_with_their_view() {
    let mut explorer = Explorer::empty();
    explorer
        .add_folder(&folder("/w/p/inner", &["deep/a.txt", "b.txt"]), 0, None)
        .unwrap();
    explorer
        .add_folder(&folder("/w/q", &["q.txt"]), 1, None)
        .unwrap();
    explorer
        .add_folder(&folder("/w/p/full", &["f.txt"]), 2, None)
        .unwrap();
    explorer.toggle_folder(at(&explorer, "/w/p/inner/deep"));
    explorer.toggle_check(at(&explorer, "/w/p/inner/b.txt"));
    explorer.toggle_check(at(&explorer, "/w/p/full"));
    let b = at(&explorer, "/w/p/inner/b.txt");

    let outer = folder(
        "/w/p",
        &[
            "inner/deep/a.txt",
            "inner/b.txt",
            "full/f.txt",
            "full/g.txt",
            "top.txt",
        ],
    );
    let (map, current) = explorer.add_folder(&outer, 3, Some(b)).unwrap();
    let tree = explorer.tree();
    assert_eq!(
        tree.folders()
            .map(|top| tree.abs_path(top))
            .collect::<Vec<_>>(),
        [PathBuf::from("/w/q"), PathBuf::from("/w/p")]
    );
    assert_eq!(current, Some(at(&explorer, "/w/p/inner/b.txt")));
    assert_eq!(map.get(b), current);
    // Checks and open folders came along; a folder that was fully checked
    // takes the file the outer scan found in it.
    assert_eq!(
        gallery(&explorer),
        ["p/full/f.txt", "p/full/g.txt", "p/inner/b.txt"]
    );
    assert!(explorer.is_open(at(&explorer, "/w/p/inner/deep")));
    assert!(explorer.is_open(at(&explorer, "/w/p")));
}

#[test]
fn a_folder_added_around_open_folders_leaves_those_its_scan_does_not_list() {
    let mut explorer = Explorer::empty();
    explorer
        .add_folder(&folder("/w/p/src", &["main.rs"]), 0, None)
        .unwrap();
    // A folder the outer folder's ignore rules hide: its scan lists none of it.
    explorer
        .add_folder(&folder("/w/p/target", &["out.o"]), 1, None)
        .unwrap();
    explorer.toggle_check(at(&explorer, "/w/p/target/out.o"));
    let outer = folder("/w/p", &["src/main.rs", "README.md"]);
    explorer.add_folder(&outer, 2, None).unwrap();
    let tree = explorer.tree();
    assert_eq!(
        tree.folders()
            .map(|top| tree.abs_path(top))
            .collect::<Vec<_>>(),
        [PathBuf::from("/w/p/target"), PathBuf::from("/w/p")]
    );
    assert_eq!(gallery(&explorer), ["target/out.o"]);
}

#[test]
fn a_rescan_that_now_lists_an_open_folder_takes_it_in() {
    let mut explorer = Explorer::empty();
    explorer
        .add_folder(&folder("/w/d/sub", &["x.txt"]), 0, None)
        .unwrap();
    explorer.toggle_check(at(&explorer, "/w/d/sub/x.txt"));
    // The outer folder could not be read at first: it takes nothing in.
    explorer
        .add_folder(&Tree::unreadable(Path::new("/w/d")), 1, None)
        .unwrap();
    assert_eq!(explorer.tree().folders().len(), 2);
    // Readable now, its rescan lists the other folder: one copy of x.txt.
    let d = at(&explorer, "/w/d");
    let x = at(&explorer, "/w/d/sub/x.txt");
    let readable = folder("/w/d", &["sub/x.txt", "top.txt"]);
    let (map, current) = explorer.replace_folder(d, &readable, Some(x)).unwrap();
    let tree = explorer.tree();
    assert_eq!(
        tree.folders()
            .map(|top| tree.abs_path(top))
            .collect::<Vec<_>>(),
        [PathBuf::from("/w/d")]
    );
    assert_eq!(rows(&explorer), ["d", "d/sub", "d/sub/x.txt", "d/top.txt"]);
    assert_eq!(gallery(&explorer), ["d/sub/x.txt"]);
    assert_eq!(current, map.get(x));
    assert_eq!(current, Some(at(&explorer, "/w/d/sub/x.txt")));
}

#[test]
fn replacing_a_view_of_one_scan_with_a_joined_tree_maps_its_root_to_its_folder() {
    let one = folder("/w/a", &["x.txt", "y.txt"]);
    let mut explorer = Explorer::new(Arc::clone(&one));
    explorer.select_all();
    let joined = Tree::join(&[
        (&*folder("/w/a", &["x.txt", "y.txt"]), NodeId::ROOT),
        (&*folder("/w/b", &["z.txt", "w.txt", "v.txt"]), NodeId::ROOT),
    ])
    .unwrap();
    let map = explorer.replace_tree(Arc::new(joined));
    assert_eq!(map.get(NodeId::ROOT), Some(at(&explorer, "/w/a")));
    assert_eq!(gallery(&explorer), ["a/x.txt", "a/y.txt"]);
}

#[test]
fn a_join_reserves_name_bytes_for_its_parts_only() {
    let mut explorer = Explorer::empty();
    for ix in 0..10 {
        let names: Vec<String> = (0..2000)
            .map(|file| format!("file-{file:05}.txt"))
            .collect();
        let paths: Vec<&str> = names.iter().map(String::as_str).collect();
        explorer
            .add_folder(&folder(&format!("/w/f{ix}"), &paths), ix, None)
            .unwrap();
    }
    let (held, reserved) = explorer.tree().name_bytes();
    assert!(reserved <= held * 2, "{reserved} bytes reserved for {held}");
}

#[test]
fn a_removed_folders_extensions_leave_the_table() {
    let mut explorer = two_folders();
    explorer
        .add_folder(&folder("/w/odd", &["a.zzq"]), 2, None)
        .unwrap();
    assert!(
        explorer
            .tree()
            .extensions()
            .iter()
            .any(|ext| &**ext == "zzq")
    );
    explorer.remove_folder(at(&explorer, "/w/odd"), None);
    assert!(
        !explorer
            .tree()
            .extensions()
            .iter()
            .any(|ext| &**ext == "zzq")
    );
    assert_eq!(
        explorer.tree().extension(at(&explorer, "/w/a/readme.md")),
        "md"
    );
}

#[test]
fn a_folder_that_cannot_be_read_takes_nothing_in() {
    let mut explorer = Explorer::empty();
    explorer
        .add_folder(&folder("/w/p/inner", &["a.txt"]), 0, None)
        .unwrap();
    explorer
        .add_folder(&Tree::unreadable(Path::new("/w/p")), 1, None)
        .unwrap();
    assert_eq!(rows(&explorer), ["inner", "inner/a.txt", "p"]);
    // A snapshot names one folder at a time too.
    let p = at(&explorer, "/w/p");
    assert_eq!(
        explorer.snapshot().folder(p),
        SavedFolder {
            path: "/w/p".into(),
            ..SavedFolder::default()
        }
    );
}

#[test]
fn a_folder_added_after_select_all_comes_in_unchecked() {
    let mut explorer = two_folders();
    explorer.select_all();
    explorer
        .add_folder(&folder("/w/c", &["c.txt"]), 2, None)
        .unwrap();
    assert_eq!(explorer.check(at(&explorer, "/w/c")), Check::Off);
    assert_eq!(explorer.check(at(&explorer, "/w/a")), Check::On);
    assert_eq!(explorer.check(NodeId::ROOT), Check::Mixed);
}

#[test]
fn the_gallery_runs_through_the_folders_in_the_order_added() {
    let mut explorer = two_folders();
    explorer.select_all();
    assert_eq!(
        gallery(&explorer),
        ["b/src/x.rs", "b/z.txt", "a/docs/guide.md", "a/readme.md"]
    );
    explorer.set_sort(Sort {
        key: SortKey::Name,
        direction: Direction::Descending,
    });
    assert_eq!(
        gallery(&explorer),
        ["b/src/x.rs", "b/z.txt", "a/docs/guide.md", "a/readme.md"]
    );
    let z = at(&explorer, "/w/b/z.txt");
    assert_eq!(
        explorer.gallery_next(z),
        Some(at(&explorer, "/w/a/docs/guide.md"))
    );
}

#[test]
fn each_folder_saves_and_restores_its_own_view() {
    let mut explorer = two_folders();
    explorer.toggle_check(at(&explorer, "/w/b/src"));
    explorer.toggle_check(at(&explorer, "/w/a/readme.md"));
    explorer.toggle_folder(at(&explorer, "/w/a/docs"));
    explorer.toggle_folder(at(&explorer, "/w/b"));
    let saved = explorer.snapshot().folders();
    assert_eq!(
        saved,
        [
            SavedFolder {
                path: "/w/b".into(),
                open: vec![],
                selection: SavedSelection {
                    folders: vec!["src".into()],
                    files: vec![],
                },
            },
            SavedFolder {
                path: "/w/a".into(),
                open: vec![String::new(), "docs".into()],
                selection: SavedSelection {
                    folders: vec![],
                    files: vec!["readme.md".into()],
                },
            },
        ]
    );

    // Restored folder by folder, as each scan lands.
    let mut restored = two_folders();
    restored.toggle_check(at(&restored, "/w/b/z.txt"));
    assert!(restored.restore_folder(&saved[1]));
    assert_eq!(gallery(&restored), ["b/z.txt", "a/readme.md"]);
    assert!(restored.restore_folder(&saved[0]));
    assert_eq!(gallery(&restored), ["b/src/x.rs", "a/readme.md"]);
    assert_eq!(rows(&restored), rows(&explorer));
    assert!(!restored.restore_folder(&SavedFolder {
        path: "/w/gone".into(),
        ..SavedFolder::default()
    }));

    // A whole folder checked saves as its own "".
    restored.toggle_check(at(&restored, "/w/b"));
    let saved = restored.snapshot().folders();
    assert_eq!(saved[0].selection.folders, [""]);
}

#[test]
fn replacing_a_joined_tree_pairs_its_folders_by_place() {
    let mut explorer = two_folders();
    let x = at(&explorer, "/w/b/src/x.rs");
    explorer.toggle_check(x);
    let other = Explorer::new(Arc::new(
        Tree::join(&[
            (&folder("/w/a", &["readme.md"]), NodeId::ROOT),
            (&folder("/w/b", &["src/x.rs"]), NodeId::ROOT),
        ])
        .unwrap(),
    ));
    let map = explorer.replace_tree(Arc::clone(other.tree()));
    assert_eq!(map.get(x), Some(at(&explorer, "/w/b/src/x.rs")));
    assert_eq!(gallery(&explorer), ["b/src/x.rs"]);
}

#[test]
fn a_tree_that_is_not_joined_becomes_one_folder_when_another_is_added() {
    let mut explorer = Explorer::new(Arc::new(
        Tree::from_paths(&["docs/a.md", "top.txt"]).unwrap(),
    ));
    let a = explorer.tree().find("docs/a.md").unwrap();
    explorer.toggle_check(a);
    let (map, _) = explorer
        .add_folder(&folder("/w/new", &["n.txt"]), 1, None)
        .unwrap();
    let tree = explorer.tree();
    assert!(tree.is_joined());
    assert_eq!(tree.path(map.get(a).unwrap()), "/docs/a.md");
    assert_eq!(rows(&explorer), ["", "new", "new/n.txt"]);
}

#[test]
fn a_size_sort_is_redone_as_each_folder_gets_its_sizes() {
    let mut raw = crate::tree::RawTree::new("late", false).unwrap();
    raw.push(0, "big.txt", crate::Kind::File).unwrap();
    raw.push(0, "small.txt", crate::Kind::File).unwrap();
    let late = Arc::new(raw.build(PathBuf::from("/w/late")).unwrap());
    let mut explorer = two_folders();
    explorer.add_folder(&late, 2, None).unwrap();
    explorer.set_sort(Sort {
        key: SortKey::Size,
        direction: Direction::Descending,
    });
    assert_eq!(
        rows(&explorer)[6..],
        ["late", "late/big.txt", "late/small.txt"]
    );
    assert!(late.set_sizes(&[0, 10, 1000]));
    explorer.sizes_loaded();
    assert_eq!(
        rows(&explorer)[6..],
        ["late", "late/small.txt", "late/big.txt"]
    );
}

/// A scan of the folder `/r/{name}` holding a generated path set.
fn generated(name: &str, set: &std::collections::BTreeMap<String, u64>) -> Arc<Tree> {
    let list: Vec<(&str, u64)> = set
        .iter()
        .map(|(path, &size)| (path.as_str(), size))
        .collect();
    Arc::new(folder_at(&format!("/r/{name}"), &list))
}

proptest::proptest! {
    #[test]
    fn a_folder_change_maps_ids_as_a_lookup_by_place_does(
        sets in proptest::collection::vec(crate::tree::tests::path_sets(), 1..4),
        ops in proptest::collection::vec(
            (0_usize..4, 0_usize..4, crate::tree::tests::path_sets()),
            0..6,
        ),
    ) {
        let mut explorer = Explorer::empty();
        for (ix, set) in sets.iter().enumerate() {
            explorer.add_folder(&generated(&format!("f{ix}"), set), ix, None).unwrap();
        }
        for (step, (op, pick, set)) in ops.iter().enumerate() {
            let old = Arc::clone(explorer.tree());
            let tops: Vec<NodeId> = old.folders().collect();
            let (map, _) = match (op, tops.is_empty()) {
                (0, _) => explorer
                    .add_folder(&generated(&format!("n{step}"), set), pick % (tops.len() + 1), None)
                    .unwrap(),
                (1, false) => explorer.remove_folder(tops[pick % tops.len()], None),
                (2, false) => {
                    // A rescan: the same folder, other contents.
                    let top = tops[pick % tops.len()];
                    let list: Vec<(&str, u64)> =
                        set.iter().map(|(path, &size)| (path.as_str(), size)).collect();
                    let path = old.abs_path(top);
                    let rescan = folder_at(path.to_str().expect("test paths are UTF-8"), &list);
                    explorer.replace_folder(top, &rescan, None).unwrap()
                }
                // A folder around some of the open ones: its scan lists
                // every other one, which it takes in.
                (3, false) => {
                    let mut paths: Vec<(String, u64)> = Vec::new();
                    for (ix, &top) in tops.iter().enumerate() {
                        if ix % 2 == pick % 2 {
                            let name = old.name(top);
                            paths.push((format!("{name}/"), 0));
                            paths.extend(old.subtree(top).skip(1).map(|id| {
                                let id = NodeId::from_index(id);
                                let slash = if old.kind(id).is_folder() { "/" } else { "" };
                                (format!("{name}/{}{slash}", old.path_from(top, id)), 1)
                            }));
                        }
                    }
                    let list: Vec<(&str, u64)> =
                        paths.iter().map(|(path, size)| (path.as_str(), *size)).collect();
                    let outer = Arc::new(folder_at("/r", &list));
                    let at = pick % (tops.len() + 1);
                    let (map, current) = explorer.add_folder(&outer, at, None).unwrap();
                    // No folder below /r is left beside it if /r lists it.
                    let tree = explorer.tree();
                    for top in tree.folders() {
                        let path = tree.abs_path(top);
                        proptest::prop_assert!(
                            path == Path::new("/r") || outer.find_abs(&path).is_none(),
                            "{}", path.display()
                        );
                    }
                    (map, current)
                }
                _ => continue,
            };
            let expected = crate::remap::IdMap::new(&old, explorer.tree());
            for node in old.ids() {
                proptest::prop_assert_eq!(map.get(node), expected.get(node), "{}", old.path(node));
            }
        }
    }
}
