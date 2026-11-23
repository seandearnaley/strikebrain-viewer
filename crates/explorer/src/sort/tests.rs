use std::cmp::Ordering;
use std::path::PathBuf;

use proptest::prelude::*;

use super::{Direction, DisplayOrder, Rank, Sort, SortKey, sibling_order};
use crate::tree::RawTree;
use crate::tree::tests::{path_sets, tree_of};
use crate::{Kind, NodeId, Tree, compare_names};

const ASCENDING: Direction = Direction::Ascending;
const DESCENDING: Direction = Direction::Descending;

fn rank(folder: bool, key: u64, name: u32) -> Rank {
    Rank { folder, key, name }
}

fn sort(key: SortKey, direction: Direction) -> Sort {
    Sort { key, direction }
}

/// The names of the root's children in display order.
fn top(tree: &Tree, sort: Sort) -> Vec<&str> {
    let order = DisplayOrder::new(tree, sort);
    order
        .children(tree, NodeId::ROOT)
        .iter()
        .map(|&id| tree.name(id))
        .collect()
}

#[test]
fn sibling_order_puts_folders_first_in_both_directions() {
    let folder = rank(true, 9, 5);
    let file = rank(false, 1, 1);
    for direction in [ASCENDING, DESCENDING] {
        assert_eq!(sibling_order(folder, file, direction), Ordering::Less);
        assert_eq!(sibling_order(file, folder, direction), Ordering::Greater);
    }
}

#[test]
fn sibling_order_applies_the_direction_to_the_key_only() {
    let small = rank(false, 1, 7);
    let large = rank(false, 2, 3);
    assert_eq!(sibling_order(small, large, ASCENDING), Ordering::Less);
    assert_eq!(sibling_order(small, large, DESCENDING), Ordering::Greater);
    // Equal keys fall back to the name, ascending in both directions.
    let first = rank(false, 5, 1);
    let second = rank(false, 5, 2);
    assert_eq!(sibling_order(first, second, ASCENDING), Ordering::Less);
    assert_eq!(sibling_order(first, second, DESCENDING), Ordering::Less);
    assert_eq!(sibling_order(first, first, DESCENDING), Ordering::Equal);
}

#[test]
fn folders_sort_before_files_in_both_directions() {
    let tree =
        Tree::from_paths_with_sizes(&[("a.txt", 50), ("zeta/x", 1), ("b.rs", 5), ("alpha/", 0)])
            .unwrap();
    for key in [SortKey::Name, SortKey::Type, SortKey::Size] {
        for direction in [ASCENDING, DESCENDING] {
            let names = top(&tree, sort(key, direction));
            let folders: Vec<bool> = names
                .iter()
                .map(|&name| tree.kind(tree.find(name).unwrap()).is_folder())
                .collect();
            assert_eq!(folders, [true, true, false, false], "{key:?} {direction:?}");
        }
    }
    assert_eq!(
        top(&tree, sort(SortKey::Name, DESCENDING)),
        ["zeta", "alpha", "b.rs", "a.txt"]
    );
}

#[test]
fn file2_sorts_before_file10_and_after_it_descending() {
    let tree = Tree::from_paths(&["file10", "file2", "File1"]).unwrap();
    assert_eq!(top(&tree, Sort::default()), ["File1", "file2", "file10"]);
    assert_eq!(
        top(&tree, sort(SortKey::Name, DESCENDING)),
        ["file10", "file2", "File1"]
    );
}

#[test]
fn type_sorts_by_extension_then_name_ascending() {
    let tree = Tree::from_paths(&[
        "b.rs", "a.txt", "c.rs", "README", "x.MD", "z.md", "d.rs2", "e.rs10",
    ])
    .unwrap();
    assert_eq!(
        top(&tree, sort(SortKey::Type, ASCENDING)),
        [
            "README", "x.MD", "z.md", "b.rs", "c.rs", "d.rs2", "e.rs10", "a.txt"
        ]
    );
    assert_eq!(
        top(&tree, sort(SortKey::Type, DESCENDING)),
        [
            "a.txt", "e.rs10", "d.rs2", "b.rs", "c.rs", "x.MD", "z.md", "README"
        ]
    );
}

#[test]
fn size_sorts_by_bytes_then_name_and_folders_by_their_sums() {
    let tree = Tree::from_paths_with_sizes(&[
        ("big/a", 100),
        ("small/a", 1),
        ("small/b", 2),
        ("x", 10),
        ("y", 5),
        ("z", 10),
    ])
    .unwrap();
    assert_eq!(
        top(&tree, sort(SortKey::Size, ASCENDING)),
        ["small", "big", "y", "x", "z"]
    );
    assert_eq!(
        top(&tree, sort(SortKey::Size, DESCENDING)),
        ["big", "small", "x", "z", "y"]
    );
}

#[test]
fn unknown_sizes_sort_by_name() {
    let mut raw = RawTree::new("root", false).unwrap();
    for name in ["c", "a", "b"] {
        raw.push(0, name, Kind::File).unwrap();
    }
    let tree = raw.build(PathBuf::from("/root")).unwrap();
    for direction in [ASCENDING, DESCENDING] {
        assert_eq!(top(&tree, sort(SortKey::Size, direction)), ["a", "b", "c"]);
    }
}

#[test]
fn a_header_click_flips_the_same_column_and_starts_another_ascending() {
    let name = Sort::default();
    let name_down = name.clicked(SortKey::Name);
    assert_eq!(name_down, sort(SortKey::Name, DESCENDING));
    assert_eq!(name_down.clicked(SortKey::Name), name);
    assert_eq!(
        name_down.clicked(SortKey::Size),
        sort(SortKey::Size, ASCENDING)
    );
    assert_eq!(DESCENDING.flipped(), ASCENDING);
}

#[test]
fn the_default_order_is_the_tree_order_without_a_copy() {
    let tree = Tree::from_paths(&["b", "a/c"]).unwrap();
    let order = DisplayOrder::new(&tree, Sort::default());
    assert!(order.pre_order().is_none());
    assert_eq!(order.position(NodeId::new(2)), 2);
    assert_eq!(
        order.children(&tree, NodeId::ROOT),
        tree.children(NodeId::ROOT)
    );
}

#[test]
fn extensions_past_the_interned_table_still_sort_by_type() {
    // 65,535 interned extensions fill the table; the rest get id 0.
    let names: Vec<String> = (0..65_540).map(|i| format!("f.e{i}")).collect();
    let mut list: Vec<&str> = names.iter().map(String::as_str).collect();
    list.push("plain");
    let tree = Tree::from_paths(&list).unwrap();
    let order = DisplayOrder::new(&tree, sort(SortKey::Type, ASCENDING));
    let got: Vec<&str> = order
        .children(&tree, NodeId::ROOT)
        .iter()
        .map(|&id| tree.extension(id))
        .collect();
    let mut want: Vec<&str> = got.clone();
    want.sort_by(|a, b| compare_names(a, b));
    assert_eq!(got, want);
    assert_eq!(got[0], "");
}

/// The order a sort promises, from names, extensions and sizes.
fn brute_order(tree: &Tree, sort: Sort, a: NodeId, b: NodeId) -> Ordering {
    let primary = match sort.key {
        SortKey::Name => compare_names(tree.name(a), tree.name(b)),
        SortKey::Type => compare_names(tree.extension(a), tree.extension(b)),
        SortKey::Size => tree.size(a).cmp(&tree.size(b)),
    };
    let primary = match sort.direction {
        Direction::Ascending => primary,
        Direction::Descending => primary.reverse(),
    };
    tree.kind(b)
        .is_folder()
        .cmp(&tree.kind(a).is_folder())
        .then(primary)
        .then_with(|| compare_names(tree.name(a), tree.name(b)))
}

fn sorts() -> impl Strategy<Value = Sort> {
    (
        prop::sample::select(&[SortKey::Name, SortKey::Type, SortKey::Size][..]),
        prop::sample::select(&[ASCENDING, DESCENDING][..]),
    )
        .prop_map(|(key, direction)| Sort { key, direction })
}

/// Every node, depth first over the display order.
fn naive_pre_order(tree: &Tree, order: &DisplayOrder, node: NodeId, out: &mut Vec<NodeId>) {
    out.push(node);
    for &child in order.children(tree, node) {
        naive_pre_order(tree, order, child, out);
    }
}

proptest! {
    #[test]
    fn each_folder_matches_a_full_comparison_sort(set in path_sets(), sort in sorts()) {
        let tree = tree_of(&set);
        let order = DisplayOrder::new(&tree, sort);
        for folder in tree.ids() {
            let mut expected = tree.children(folder).to_vec();
            expected.sort_by(|&a, &b| brute_order(&tree, sort, a, b));
            prop_assert_eq!(order.children(&tree, folder), &expected[..]);
        }
    }

    #[test]
    fn positions_follow_the_expanded_display_order(set in path_sets(), sort in sorts()) {
        let tree = tree_of(&set);
        let order = DisplayOrder::new(&tree, sort);
        let mut expected = Vec::new();
        naive_pre_order(&tree, &order, NodeId::ROOT, &mut expected);
        if let Some(pre_order) = order.pre_order() {
            prop_assert_eq!(pre_order, &expected[..]);
        }
        for (at, &node) in expected.iter().enumerate() {
            prop_assert_eq!(order.position(node), at);
        }
    }
}
