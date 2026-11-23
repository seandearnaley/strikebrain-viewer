use fixedbitset::FixedBitSet;
use proptest::prelude::*;

use super::{Check, Selection};
use crate::tree::tests::{path_sets, tree_of};
use crate::{NodeId, Tree, Visibility};

fn id(tree: &Tree, path: &str) -> NodeId {
    tree.find(path).unwrap()
}

fn all() -> Visibility {
    Visibility::all()
}

/// The universe a filter would give: the named nodes and their ancestors.
fn universe(tree: &Tree, visible: &[&str]) -> Visibility {
    Visibility::from_universe(tree, visible_set(tree, visible))
}

fn visible_set(tree: &Tree, visible: &[&str]) -> FixedBitSet {
    let mut set = FixedBitSet::with_capacity(tree.node_count());
    for path in visible {
        let mut node = Some(id(tree, path));
        while let Some(n) = node {
            set.insert(n.index());
            node = tree.parent(n);
        }
    }
    set
}

fn selected(tree: &Tree, selection: &Selection) -> Vec<String> {
    selection.iter().map(|n| tree.path(n)).collect()
}

fn sample() -> Tree {
    Tree::from_paths(&[
        "docs/a.md",
        "docs/b.md",
        "docs/deep/c.png",
        "empty/",
        "top.txt",
    ])
    .unwrap()
}

#[test]
fn a_new_selection_is_empty_and_every_node_is_off() {
    let tree = sample();
    let selection = Selection::new(&tree);
    assert!(selection.is_empty());
    assert_eq!(selection.count(), 0);
    assert_eq!(selection.as_bitset().len(), tree.node_count());
    for node in tree.ids() {
        assert_eq!(selection.check(&tree, &all(), node), Check::Off);
    }
}

#[test]
fn toggling_a_file_flips_it() {
    let tree = sample();
    let mut selection = Selection::new(&tree);
    let file = id(&tree, "top.txt");
    selection.toggle(&tree, &all(), file);
    assert!(selection.is_selected(file));
    assert_eq!(selection.check(&tree, &all(), file), Check::On);
    assert_eq!(selection.check(&tree, &all(), NodeId::ROOT), Check::Mixed);
    selection.toggle(&tree, &all(), file);
    assert!(!selection.is_selected(file));
    assert_eq!(selection.check(&tree, &all(), NodeId::ROOT), Check::Off);
}

#[test]
fn a_mixed_folder_completes_on_click() {
    let tree = sample();
    let mut selection = Selection::new(&tree);
    selection.toggle(&tree, &all(), id(&tree, "docs/a.md"));
    let docs = id(&tree, "docs");
    assert_eq!(selection.check(&tree, &all(), docs), Check::Mixed);
    selection.toggle(&tree, &all(), docs);
    assert_eq!(selection.check(&tree, &all(), docs), Check::On);
    assert_eq!(
        selected(&tree, &selection),
        ["docs/deep/c.png", "docs/a.md", "docs/b.md"]
    );
}

#[test]
fn a_fully_checked_folder_clears_on_click() {
    let tree = sample();
    let mut selection = Selection::new(&tree);
    let docs = id(&tree, "docs");
    selection.toggle(&tree, &all(), docs);
    selection.toggle(&tree, &all(), id(&tree, "top.txt"));
    assert_eq!(selection.check(&tree, &all(), docs), Check::On);
    selection.toggle(&tree, &all(), docs);
    assert_eq!(selection.check(&tree, &all(), docs), Check::Off);
    assert_eq!(selected(&tree, &selection), ["top.txt"]);
}

#[test]
fn an_empty_folder_is_off_and_stays_off() {
    let tree = sample();
    let mut selection = Selection::new(&tree);
    let empty = id(&tree, "empty");
    selection.toggle(&tree, &all(), empty);
    assert_eq!(selection.check(&tree, &all(), empty), Check::Off);
    assert!(selection.is_empty());
}

#[test]
fn select_all_under_a_filter_selects_only_matches() {
    let tree = sample();
    let mut selection = Selection::new(&tree);
    let visible = universe(&tree, &["docs/a.md", "top.txt"]);
    selection.select_all(&tree, &visible);
    assert_eq!(selected(&tree, &selection), ["docs/a.md", "top.txt"]);
    assert_eq!(selection.check(&tree, &visible, NodeId::ROOT), Check::On);
    assert_eq!(selection.check(&tree, &all(), NodeId::ROOT), Check::Mixed);
}

#[test]
fn unchecking_a_folder_under_a_filter_clears_hidden_files_too() {
    let tree = sample();
    let mut selection = Selection::new(&tree);
    let docs = id(&tree, "docs");
    selection.toggle(&tree, &all(), docs);
    let visible = universe(&tree, &["docs/a.md"]);
    assert_eq!(selection.check(&tree, &visible, docs), Check::On);
    selection.toggle(&tree, &visible, docs);
    assert!(selection.is_empty());
}

#[test]
fn checking_a_folder_under_a_filter_selects_only_its_matches() {
    let tree = sample();
    let mut selection = Selection::new(&tree);
    let docs = id(&tree, "docs");
    let visible = universe(&tree, &["docs/b.md", "docs/deep/c.png"]);
    selection.toggle(&tree, &visible, docs);
    assert_eq!(
        selected(&tree, &selection),
        ["docs/deep/c.png", "docs/b.md"]
    );
    assert_eq!(selection.check(&tree, &visible, docs), Check::On);
    assert_eq!(selection.check(&tree, &all(), docs), Check::Mixed);
}

#[test]
fn only_hidden_selections_leave_a_folder_off_under_the_filter() {
    let tree = sample();
    let mut selection = Selection::new(&tree);
    selection.toggle(&tree, &all(), id(&tree, "docs/a.md"));
    let visible = universe(&tree, &["docs/b.md"]);
    assert_eq!(
        selection.check(&tree, &visible, id(&tree, "docs")),
        Check::Off
    );
    // A hidden file still shows its own state.
    assert_eq!(
        selection.check(&tree, &visible, id(&tree, "docs/a.md")),
        Check::On
    );
}

#[test]
fn folders_never_enter_the_selection() {
    let tree = sample();
    let mut selection = Selection::new(&tree);
    selection.set_file(&tree, id(&tree, "docs"), true);
    assert!(selection.is_empty());
    assert!(!selection.is_selected(id(&tree, "docs")));
    selection.set_file(&tree, id(&tree, "top.txt"), true);
    selection.set_file(&tree, id(&tree, "top.txt"), true);
    assert_eq!(selection.count(), 1);
}

#[test]
fn clear_empties_the_selection() {
    let tree = sample();
    let mut selection = Selection::new(&tree);
    selection.select_all(&tree, &all());
    assert_eq!(selection.count(), 4);
    assert_eq!(selection.check(&tree, &all(), NodeId::ROOT), Check::On);
    selection.clear();
    assert!(selection.is_empty());
    assert_eq!(selection.as_bitset().len(), tree.node_count());
}

#[test]
#[should_panic(expected = "sized for another tree")]
fn a_universe_for_another_tree_panics() {
    let tree = sample();
    let selection = Selection::new(&tree);
    let other = Tree::from_paths(&["a", "b"]).unwrap();
    let wrong = Visibility::from_universe(&other, FixedBitSet::with_capacity(3));
    std::hint::black_box(selection.check(&tree, &wrong, NodeId::ROOT));
}

/// A tree with a selection and an optional universe, both random.
fn scenario() -> impl Strategy<Value = (Tree, Selection, Option<FixedBitSet>)> {
    (
        path_sets(),
        prop::collection::vec(any::<bool>(), 300),
        prop::option::of(prop::collection::vec(any::<bool>(), 300)),
    )
        .prop_map(|(set, picks, universe)| {
            let tree = tree_of(&set);
            let mut selection = Selection::new(&tree);
            for node in tree.ids() {
                selection.set_file(
                    &tree,
                    node,
                    picks.get(node.index()).copied().unwrap_or(false),
                );
            }
            let universe = universe.map(|bits| {
                let mut set = FixedBitSet::with_capacity(tree.node_count());
                for node in tree.ids() {
                    set.set(
                        node.index(),
                        bits.get(node.index()).copied().unwrap_or(true),
                    );
                }
                set
            });
            (tree, selection, universe)
        })
}

/// The visibility a random universe stands for.
fn visibility_of(tree: &Tree, universe: Option<&FixedBitSet>) -> Visibility {
    universe.map_or_else(Visibility::all, |set| {
        Visibility::from_universe(tree, set.clone())
    })
}

fn in_universe(universe: Option<&FixedBitSet>, node: NodeId) -> bool {
    universe.is_none_or(|u| u.contains(node.index()))
}

/// Selected and total files of the universe under `node`, by recursion.
fn recursive_count(
    tree: &Tree,
    selection: &Selection,
    universe: Option<&FixedBitSet>,
    node: NodeId,
) -> (usize, usize) {
    if tree.kind(node).is_file() {
        let counted = in_universe(universe, node);
        return (
            usize::from(counted && selection.is_selected(node)),
            usize::from(counted),
        );
    }
    tree.children(node)
        .iter()
        .map(|&child| recursive_count(tree, selection, universe, child))
        .fold((0, 0), |(s, t), (cs, ct)| (s + cs, t + ct))
}

fn brute_check(
    tree: &Tree,
    selection: &Selection,
    universe: Option<&FixedBitSet>,
    node: NodeId,
) -> Check {
    if tree.kind(node).is_file() {
        return if selection.is_selected(node) {
            Check::On
        } else {
            Check::Off
        };
    }
    match recursive_count(tree, selection, universe, node) {
        (0, _) => Check::Off,
        (selected, total) if selected == total => Check::On,
        _ => Check::Mixed,
    }
}

proptest! {
    #[test]
    fn check_matches_a_recursive_count((tree, selection, universe) in scenario()) {
        let visibility = visibility_of(&tree, universe.as_ref());
        for node in tree.ids() {
            prop_assert_eq!(
                selection.check(&tree, &visibility, node),
                brute_check(&tree, &selection, universe.as_ref(), node)
            );
        }
    }

    #[test]
    fn checks_stay_right_across_changes_and_universes(
        (tree, mut selection, universe) in scenario(),
        clicks in prop::collection::vec((0_usize..300, any::<bool>()), 0..8),
    ) {
        let filtered = visibility_of(&tree, universe.as_ref());
        let every = Visibility::all();
        for (pick, under_filter) in clicks {
            for (visibility, set) in [(&filtered, universe.as_ref()), (&every, None)] {
                for node in tree.ids() {
                    prop_assert_eq!(
                        selection.check(&tree, visibility, node),
                        brute_check(&tree, &selection, set, node)
                    );
                }
            }
            let node = NodeId::new(u32::try_from(pick % tree.node_count()).unwrap());
            let visibility = if under_filter { &filtered } else { &every };
            match pick % 4 {
                0 => selection.set_file(&tree, node, !selection.is_selected(node)),
                1 => selection.select_all(&tree, visibility),
                2 if pick % 8 == 2 => selection.clear(),
                _ => selection.toggle(&tree, visibility, node),
            }
        }
        for node in tree.ids() {
            prop_assert_eq!(
                selection.check(&tree, &filtered, node),
                brute_check(&tree, &selection, universe.as_ref(), node)
            );
        }
    }

    #[test]
    fn toggling_a_folder_matches_a_per_file_loop((tree, selection, universe) in scenario()) {
        let visibility = visibility_of(&tree, universe.as_ref());
        for folder in tree.ids().filter(|&n| tree.kind(n).is_folder()) {
            let mut fast = selection.clone();
            fast.toggle(&tree, &visibility, folder);

            let mut naive = selection.clone();
            let clear = brute_check(&tree, &selection, universe.as_ref(), folder) == Check::On;
            for i in tree.subtree(folder) {
                let file = NodeId::new(u32::try_from(i).unwrap());
                if clear {
                    naive.set_file(&tree, file, false);
                } else if in_universe(universe.as_ref(), file) {
                    naive.set_file(&tree, file, true);
                }
            }
            prop_assert_eq!(fast, naive);
        }
    }

    #[test]
    fn select_all_matches_a_per_file_loop((tree, selection, universe) in scenario()) {
        let mut fast = selection.clone();
        fast.select_all(&tree, &visibility_of(&tree, universe.as_ref()));
        let mut naive = selection.clone();
        for node in tree.ids() {
            if in_universe(universe.as_ref(), node) {
                naive.set_file(&tree, node, true);
            }
        }
        prop_assert_eq!(&fast, &naive);
        prop_assert!(fast.iter().all(|n| tree.is_file(n)));
        prop_assert_eq!(fast.count(), fast.iter().count());
    }
}
