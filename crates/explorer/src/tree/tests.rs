use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use proptest::prelude::*;

use super::{Kind, Tree, TreeError, canonical_order, compare_names};
use crate::NodeId;

const NO_NODES: &[NodeId] = &[];

fn paths(tree: &Tree) -> Vec<String> {
    tree.ids().map(|id| tree.path(id)).collect()
}

fn id(tree: &Tree, path: &str) -> NodeId {
    tree.find(path)
        .unwrap_or_else(|| panic!("no node at {path:?}"))
}

#[test]
fn an_empty_list_builds_just_the_root() {
    let tree = Tree::from_paths(&[]).unwrap();
    assert_eq!(tree.node_count(), 1);
    assert_eq!(tree.kind(NodeId::ROOT), Kind::Dir);
    assert_eq!(tree.parent(NodeId::ROOT), None);
    assert_eq!(tree.subtree(NodeId::ROOT), 0..1);
    assert_eq!(tree.path(NodeId::ROOT), "");
    assert_eq!(tree.name(NodeId::ROOT), "");
    assert_eq!(tree.children(NodeId::ROOT), NO_NODES);
    assert!(tree.root_path().as_os_str().is_empty());
}

#[test]
fn ids_follow_pre_order_with_folders_first() {
    let tree = Tree::from_paths(&["b.txt", "a/z.txt", "a/y/", "c/", "A.md"]).unwrap();
    assert_eq!(
        paths(&tree),
        ["", "a", "a/y", "a/z.txt", "c", "A.md", "b.txt"]
    );
    assert_eq!(tree.subtree(id(&tree, "a")), 1..4);
    assert_eq!(tree.subtree(id(&tree, "c")), 4..5);
    assert_eq!(tree.subtree(NodeId::ROOT), 0..7);
}

#[test]
fn file2_sorts_before_file10() {
    let tree = Tree::from_paths(&["file10", "file2", "file1", "File3"]).unwrap();
    assert_eq!(paths(&tree), ["", "file1", "file2", "File3", "file10"]);
}

#[test]
fn name_order_ignores_case_and_accents_then_breaks_ties_by_bytes() {
    assert_eq!(compare_names("apple", "Banana"), Ordering::Less);
    assert_eq!(compare_names("Zeta", "alpha"), Ordering::Greater);
    assert_eq!(compare_names("a2b", "a10b"), Ordering::Less);
    assert_eq!(compare_names("A", "a"), Ordering::Less);
    assert_eq!(compare_names("resume", "résumé"), Ordering::Less);
    assert_eq!(compare_names("same", "same"), Ordering::Equal);
}

#[test]
fn folders_come_first_whatever_their_names() {
    let order = canonical_order(Kind::File, "a", Kind::Dir, "z");
    assert_eq!(order, Ordering::Greater);
    let order = canonical_order(Kind::Unreadable, "z", Kind::Symlink, "a");
    assert_eq!(order, Ordering::Less);
}

#[test]
fn implicit_parent_folders_are_created() {
    let tree = Tree::from_paths(&["a/b/c.txt"]).unwrap();
    assert_eq!(paths(&tree), ["", "a", "a/b", "a/b/c.txt"]);
    assert_eq!(tree.kind(id(&tree, "a")), Kind::Dir);
    assert_eq!(tree.kind(id(&tree, "a/b/c.txt")), Kind::File);
    assert_eq!(tree.depth(id(&tree, "a/b/c.txt")), 3);
    assert_eq!(tree.parent(id(&tree, "a/b")), Some(id(&tree, "a")));
}

#[test]
fn a_trailing_slash_makes_a_folder_and_may_repeat() {
    let tree = Tree::from_paths(&["empty/", "a/", "a/x", "a/"]).unwrap();
    assert_eq!(paths(&tree), ["", "a", "a/x", "empty"]);
    assert_eq!(tree.kind(id(&tree, "empty")), Kind::Dir);
    assert_eq!(tree.children(id(&tree, "empty")), NO_NODES);
}

#[test]
fn folder_sizes_sum_their_files() {
    let tree =
        Tree::from_paths_with_sizes(&[("a/x", 3), ("a/b/y", 4), ("z", 10), ("a/", 99)]).unwrap();
    assert!(tree.has_sizes());
    assert_eq!(tree.size(id(&tree, "a/x")), Some(3));
    assert_eq!(tree.size(id(&tree, "a/b")), Some(4));
    assert_eq!(tree.size(id(&tree, "a")), Some(7));
    assert_eq!(tree.size(NodeId::ROOT), Some(17));
    // Sizes are set once; a second set changes nothing.
    assert!(!tree.set_sizes(vec![1; tree.node_count()]));
    assert_eq!(tree.size(NodeId::ROOT), Some(17));
}

#[test]
fn files_bitset_marks_files_only() {
    let tree = Tree::from_paths(&["d/f", "g"]).unwrap();
    let files: Vec<usize> = tree.files().ones().collect();
    assert_eq!(files, [2, 3]);
    assert!(tree.is_file(id(&tree, "g")));
    assert!(!tree.is_file(id(&tree, "d")));
}

#[test]
fn extensions_are_lowercase_and_interned() {
    let tree = Tree::from_paths(&[
        "a.PNG",
        "b.png",
        ".gitignore",
        "arc.tar.GZ",
        "noext",
        "dir.d/",
    ])
    .unwrap();
    let a = id(&tree, "a.PNG");
    assert_eq!(tree.extension(a), "png");
    assert_eq!(tree.ext_id(a), tree.ext_id(id(&tree, "b.png")));
    assert_eq!(&*tree.extensions()[usize::from(tree.ext_id(a))], "png");
    assert_eq!(tree.extension(id(&tree, ".gitignore")), "");
    assert_eq!(tree.ext_id(id(&tree, ".gitignore")), 0);
    assert_eq!(tree.extension(id(&tree, "arc.tar.GZ")), "gz");
    assert_eq!(tree.extension(id(&tree, "noext")), "");
    assert_eq!(tree.extension(id(&tree, "dir.d")), "");
    assert_eq!(tree.ext_id(id(&tree, "dir.d")), 0);
    assert_eq!(&*tree.extensions()[0], "");
}

#[test]
fn the_haystack_holds_lowercase_names_separated_by_nul() {
    let tree = Tree::from_paths(&["Read Me.TXT", "Ärger/"]).unwrap();
    assert_eq!(tree.haystack(), "\0ärger\0read me.txt\0");
    let file = id(&tree, "Read Me.TXT");
    assert_eq!(tree.lower_name(file), "read me.txt");
    assert_eq!(tree.name(file), "Read Me.TXT");
    let at = tree.haystack().find("me.txt").unwrap();
    assert_eq!(tree.node_at_haystack(at), file);
    assert_eq!(tree.node_at_haystack(0), NodeId::ROOT);
}

#[test]
#[should_panic(expected = "past the haystack")]
fn an_offset_past_the_haystack_panics() {
    let tree = Tree::from_paths(&["a"]).unwrap();
    std::hint::black_box(tree.node_at_haystack(tree.haystack().len()));
}

#[test]
fn find_and_path_round_trip() {
    let tree = Tree::from_paths(&["src/lib.rs", "src/tree/mod.rs", "README.md"]).unwrap();
    for node in tree.ids() {
        assert_eq!(tree.find(&tree.path(node)), Some(node));
    }
    assert_eq!(tree.find(""), Some(NodeId::ROOT));
    assert_eq!(tree.find("src/tree/"), tree.find("src/tree"));
}

#[test]
fn find_misses_absent_paths() {
    let tree = Tree::from_paths(&["src/lib.rs"]).unwrap();
    assert_eq!(tree.find("src/main.rs"), None);
    assert_eq!(tree.find("src/lib.rs/x"), None);
    assert_eq!(tree.find("SRC/lib.rs"), None);
}

#[test]
fn abs_path_joins_the_root_path() {
    let tree = Tree::from_paths(&["a/b.txt"]).unwrap();
    assert_eq!(
        tree.abs_path(id(&tree, "a/b.txt")),
        PathBuf::from("a").join("b.txt")
    );
    assert_eq!(tree.abs_path(NodeId::ROOT), PathBuf::new());
}

#[test]
fn invalid_paths_are_rejected() {
    for bad in ["", "/", "/a", "a//b", "./a", "a/../b", "a/.", "a\0b"] {
        assert_eq!(
            Tree::from_paths(&[bad]).unwrap_err(),
            TreeError::InvalidPath(bad.to_owned()),
            "{bad:?}"
        );
    }
}

#[test]
fn a_path_used_twice_or_as_file_and_folder_conflicts() {
    for set in [
        &["a", "a/b"][..],
        &["a/b", "a"],
        &["x", "x"],
        &["d/", "d"],
        &["d", "d/"],
    ] {
        let err = Tree::from_paths(set).unwrap_err();
        assert!(
            matches!(err, TreeError::Conflict(_)),
            "{set:?} gave {err:?}"
        );
    }
}

#[test]
fn errors_explain_themselves() {
    assert_eq!(
        TreeError::InvalidPath("/a".into()).to_string(),
        r#"invalid path "/a""#
    );
    assert!(
        TreeError::Conflict("x".into())
            .to_string()
            .contains("both file and folder")
    );
    assert!(TreeError::TooLarge.to_string().contains("4,294,967,295"));
    assert!(TreeError::TooDeep.to_string().contains("65,535"));
}

#[test]
fn children_hold_canonical_order() {
    let tree = Tree::from_paths(&["b", "a/", "c/x", "B/"]).unwrap();
    let names: Vec<&str> = tree
        .children(NodeId::ROOT)
        .iter()
        .map(|&c| tree.name(c))
        .collect();
    assert_eq!(names, ["a", "B", "c", "b"]);
    assert_eq!(tree.children(id(&tree, "b")), NO_NODES);
}

#[test]
fn debug_output_summarises_the_tree() {
    let tree = Tree::from_paths(&["a/b"]).unwrap();
    let text = format!("{tree:?}");
    assert!(text.contains("nodes: 3"), "{text}");
    assert!(text.contains("files: 1"), "{text}");
}

// Folder and file names share no entry, so random paths never conflict.
const FOLDERS: [&str; 9] = [
    "src", "Src", "src10", "src2", "Ä", "a b", ".hidden", "x.d", "SRC",
];
const FILES: [&str; 11] = [
    "a.txt",
    "A.TXT",
    "b10.rs",
    "b2.rs",
    "readme",
    ".gitignore",
    "é.md",
    "E.md",
    "e.md",
    "z",
    "f.tar.gz",
];

/// Random relative paths with sizes: up to three folders, then a file or a
/// trailing slash.
pub(crate) fn path_sets() -> impl Strategy<Value = BTreeMap<String, u64>> {
    let one = (
        prop::collection::vec(prop::sample::select(&FOLDERS[..]), 0..4),
        prop::option::of(prop::sample::select(&FILES[..])),
        0_u64..1000,
    )
        .prop_map(|(folders, file, size)| {
            let mut path = folders.join("/");
            match file {
                Some(file) => {
                    if !path.is_empty() {
                        path.push('/');
                    }
                    path.push_str(file);
                }
                None if path.is_empty() => path.push_str("src/"),
                None => path.push('/'),
            }
            (path, size)
        });
    prop::collection::vec(one, 0..60).prop_map(|paths| paths.into_iter().collect())
}

/// Builds a tree from a generated path set.
pub(crate) fn tree_of(set: &BTreeMap<String, u64>) -> Tree {
    let list: Vec<(&str, u64)> = set.iter().map(|(p, &s)| (p.as_str(), s)).collect();
    Tree::from_paths_with_sizes(&list).unwrap()
}

fn is_ancestor_or_self(tree: &Tree, ancestor: NodeId, mut node: NodeId) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        match tree.parent(node) {
            Some(parent) => node = parent,
            None => return false,
        }
    }
}

proptest! {
    #[test]
    fn every_path_and_implied_folder_becomes_one_node(set in path_sets()) {
        let tree = tree_of(&set);
        let mut expected = BTreeSet::new();
        for path in set.keys() {
            let trimmed = path.trim_end_matches('/');
            let mut prefix = String::new();
            for part in trimmed.split('/') {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                expected.insert(prefix.clone());
            }
        }
        let got: BTreeSet<String> = tree.ids().skip(1).map(|id| tree.path(id)).collect();
        prop_assert_eq!(got, expected);
        for node in tree.ids() {
            prop_assert_eq!(tree.find(&tree.path(node)), Some(node));
        }
    }

    #[test]
    fn parents_precede_children(set in path_sets()) {
        let tree = tree_of(&set);
        for node in tree.ids().skip(1) {
            let parent = tree.parent(node).unwrap();
            prop_assert!(parent < node);
            prop_assert_eq!(tree.depth(node), tree.depth(parent) + 1);
        }
    }

    #[test]
    fn a_subtree_range_holds_exactly_the_node_and_its_descendants(set in path_sets()) {
        let tree = tree_of(&set);
        for node in tree.ids() {
            let range = tree.subtree(node);
            if tree.kind(node).is_file() {
                prop_assert_eq!(range.clone(), node.index()..node.index() + 1);
            }
            for other in tree.ids() {
                prop_assert_eq!(range.contains(&other.index()), is_ancestor_or_self(&tree, node, other));
            }
        }
    }

    #[test]
    fn siblings_follow_canonical_order(set in path_sets()) {
        let tree = tree_of(&set);
        for node in tree.ids() {
            let expected: Vec<NodeId> = tree.ids().filter(|&c| tree.parent(c) == Some(node)).collect();
            prop_assert_eq!(tree.children(node), &expected[..]);
            for pair in expected.windows(2) {
                let order = canonical_order(tree.kind(pair[0]), tree.name(pair[0]), tree.kind(pair[1]), tree.name(pair[1]));
                prop_assert_eq!(order, Ordering::Less);
            }
        }
    }

    #[test]
    fn a_folder_size_is_the_sum_of_its_files(set in path_sets()) {
        let tree = tree_of(&set);
        for node in tree.ids() {
            let sum: u64 = tree.subtree(node).filter(|&i| tree.files().contains(i)).map(|i| tree.size(NodeId::new(u32::try_from(i).unwrap())).unwrap()).sum();
            prop_assert_eq!(tree.size(node), Some(sum));
            if tree.kind(node).is_file() {
                prop_assert_eq!(tree.size(node), set.get(&tree.path(node)).copied());
            }
        }
    }
}
