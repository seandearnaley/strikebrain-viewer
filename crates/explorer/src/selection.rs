//! Which files are checked, and the check state each row shows.
//!
//! A folder's state depends on a universe: the files a filter lets through,
//! or every file when no filter is active. The universe is passed in as the
//! visibility bitset (one bit per node), or `None` for every file.

use fixedbitset::FixedBitSet;

use crate::{NodeId, Tree, bits};

/// A row's checkbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Check {
    /// Nothing in the universe is selected, or the universe is empty.
    Off,
    /// Some but not all of the universe is selected.
    Mixed,
    /// The universe is non-empty and fully selected.
    On,
}

/// The checked files: one bit per node, set only for files and symlinks.
/// There is no cap on how many.
///
/// A selection is sized for its tree, so there is no empty default; make
/// one with [`Selection::new`].
///
/// ```compile_fail
/// let selection = explorer::Selection::default();
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    bits: FixedBitSet,
}

impl Selection {
    /// An empty selection sized for `tree`.
    #[must_use]
    pub fn new(tree: &Tree) -> Self {
        Self {
            bits: FixedBitSet::with_capacity(tree.node_count()),
        }
    }

    /// The checkbox for `id`. A file is [`Check::On`] when selected, whatever
    /// the universe. A folder compares the selected files of its universe
    /// with the whole universe; both are ranged bit counts over its subtree.
    ///
    /// Learning exercise: Sean may rewrite this body; the tests below define it.
    ///
    /// # Panics
    ///
    /// If the selection or the universe was sized for another tree.
    #[must_use]
    pub fn check(&self, tree: &Tree, universe: Option<&FixedBitSet>, id: NodeId) -> Check {
        self.assert_sized_for(tree, universe);
        if tree.is_file(id) {
            return if self.is_selected(id) {
                Check::On
            } else {
                Check::Off
            };
        }
        let range = tree.subtree(id);
        // Only file bits are ever set, so the selection needs no mask by `files`.
        let (selected, total) = match universe {
            None => (
                self.bits.count_ones(range.clone()),
                tree.files().count_ones(range),
            ),
            Some(universe) => (
                bits::count_and(&self.bits, universe, range.clone()),
                bits::count_and(tree.files(), universe, range),
            ),
        };
        match selected {
            0 => Check::Off,
            _ if selected == total => Check::On,
            _ => Check::Mixed,
        }
    }

    /// A selection of the files among `bits`, one bit per node of `tree`.
    /// Folder bits are dropped.
    pub(crate) fn from_bits(tree: &Tree, mut bits: FixedBitSet) -> Self {
        assert_eq!(bits.len(), tree.node_count(), "one bit per node");
        bits.intersect_with(tree.files());
        Self { bits }
    }

    /// Whether the file `id` is selected. Always false for a folder.
    #[must_use]
    pub fn is_selected(&self, id: NodeId) -> bool {
        self.bits.contains(id.index())
    }

    /// Selects or deselects the file `id`. A folder is left alone.
    pub fn set_file(&mut self, tree: &Tree, id: NodeId, on: bool) {
        if tree.is_file(id) {
            self.bits.set(id.index(), on);
        }
    }

    /// The click on a checkbox: flips a file; a folder that is
    /// [`Check::On`] clears every file in its subtree, hidden ones included,
    /// and any other folder selects every file of its universe.
    ///
    /// # Panics
    ///
    /// If the selection or the universe was sized for another tree.
    pub fn toggle(&mut self, tree: &Tree, universe: Option<&FixedBitSet>, id: NodeId) {
        if tree.is_file(id) {
            self.bits.toggle(id.index());
        } else if self.check(tree, universe, id) == Check::On {
            self.bits.remove_range(tree.subtree(id));
        } else {
            bits::or_and(&mut self.bits, tree.files(), universe, tree.subtree(id));
        }
    }

    /// Selects every file in the universe. Files already selected stay
    /// selected.
    ///
    /// # Panics
    ///
    /// If the selection or the universe was sized for another tree.
    pub fn select_all(&mut self, tree: &Tree, universe: Option<&FixedBitSet>) {
        self.assert_sized_for(tree, universe);
        bits::or_and(
            &mut self.bits,
            tree.files(),
            universe,
            tree.subtree(NodeId::ROOT),
        );
    }

    /// Deselects everything.
    pub fn clear(&mut self) {
        self.bits.clear();
    }

    /// How many files are selected.
    #[must_use]
    pub fn count(&self) -> usize {
        self.bits.count_ones(..)
    }

    /// Whether nothing is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bits.is_clear()
    }

    /// The selected files in id order.
    pub fn iter(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.bits.ones().map(NodeId::from_index)
    }

    /// The selection as one bit per node.
    #[must_use]
    pub fn as_bitset(&self) -> &FixedBitSet {
        &self.bits
    }

    fn assert_sized_for(&self, tree: &Tree, universe: Option<&FixedBitSet>) {
        let n = tree.node_count();
        assert!(
            self.bits.len() == n && universe.is_none_or(|u| u.len() == n),
            "selection or universe sized for another tree"
        );
    }
}

#[cfg(test)]
mod tests {
    use fixedbitset::FixedBitSet;
    use proptest::prelude::*;

    use super::{Check, Selection};
    use crate::tree::tests::{path_sets, tree_of};
    use crate::{NodeId, Tree};

    fn id(tree: &Tree, path: &str) -> NodeId {
        tree.find(path).unwrap()
    }

    /// The universe a filter would give: the named nodes and their ancestors.
    fn universe(tree: &Tree, visible: &[&str]) -> FixedBitSet {
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
            assert_eq!(selection.check(&tree, None, node), Check::Off);
        }
    }

    #[test]
    fn toggling_a_file_flips_it() {
        let tree = sample();
        let mut selection = Selection::new(&tree);
        let file = id(&tree, "top.txt");
        selection.toggle(&tree, None, file);
        assert!(selection.is_selected(file));
        assert_eq!(selection.check(&tree, None, file), Check::On);
        assert_eq!(selection.check(&tree, None, NodeId::ROOT), Check::Mixed);
        selection.toggle(&tree, None, file);
        assert!(!selection.is_selected(file));
        assert_eq!(selection.check(&tree, None, NodeId::ROOT), Check::Off);
    }

    #[test]
    fn a_mixed_folder_completes_on_click() {
        let tree = sample();
        let mut selection = Selection::new(&tree);
        selection.toggle(&tree, None, id(&tree, "docs/a.md"));
        let docs = id(&tree, "docs");
        assert_eq!(selection.check(&tree, None, docs), Check::Mixed);
        selection.toggle(&tree, None, docs);
        assert_eq!(selection.check(&tree, None, docs), Check::On);
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
        selection.toggle(&tree, None, docs);
        selection.toggle(&tree, None, id(&tree, "top.txt"));
        assert_eq!(selection.check(&tree, None, docs), Check::On);
        selection.toggle(&tree, None, docs);
        assert_eq!(selection.check(&tree, None, docs), Check::Off);
        assert_eq!(selected(&tree, &selection), ["top.txt"]);
    }

    #[test]
    fn an_empty_folder_is_off_and_stays_off() {
        let tree = sample();
        let mut selection = Selection::new(&tree);
        let empty = id(&tree, "empty");
        selection.toggle(&tree, None, empty);
        assert_eq!(selection.check(&tree, None, empty), Check::Off);
        assert!(selection.is_empty());
    }

    #[test]
    fn select_all_under_a_filter_selects_only_matches() {
        let tree = sample();
        let mut selection = Selection::new(&tree);
        let visible = universe(&tree, &["docs/a.md", "top.txt"]);
        selection.select_all(&tree, Some(&visible));
        assert_eq!(selected(&tree, &selection), ["docs/a.md", "top.txt"]);
        assert_eq!(
            selection.check(&tree, Some(&visible), NodeId::ROOT),
            Check::On
        );
        assert_eq!(selection.check(&tree, None, NodeId::ROOT), Check::Mixed);
    }

    #[test]
    fn unchecking_a_folder_under_a_filter_clears_hidden_files_too() {
        let tree = sample();
        let mut selection = Selection::new(&tree);
        let docs = id(&tree, "docs");
        selection.toggle(&tree, None, docs);
        let visible = universe(&tree, &["docs/a.md"]);
        assert_eq!(selection.check(&tree, Some(&visible), docs), Check::On);
        selection.toggle(&tree, Some(&visible), docs);
        assert!(selection.is_empty());
    }

    #[test]
    fn checking_a_folder_under_a_filter_selects_only_its_matches() {
        let tree = sample();
        let mut selection = Selection::new(&tree);
        let docs = id(&tree, "docs");
        let visible = universe(&tree, &["docs/b.md", "docs/deep/c.png"]);
        selection.toggle(&tree, Some(&visible), docs);
        assert_eq!(
            selected(&tree, &selection),
            ["docs/deep/c.png", "docs/b.md"]
        );
        assert_eq!(selection.check(&tree, Some(&visible), docs), Check::On);
        assert_eq!(selection.check(&tree, None, docs), Check::Mixed);
    }

    #[test]
    fn only_hidden_selections_leave_a_folder_off_under_the_filter() {
        let tree = sample();
        let mut selection = Selection::new(&tree);
        selection.toggle(&tree, None, id(&tree, "docs/a.md"));
        let visible = universe(&tree, &["docs/b.md"]);
        assert_eq!(
            selection.check(&tree, Some(&visible), id(&tree, "docs")),
            Check::Off
        );
        // A hidden file still shows its own state.
        assert_eq!(
            selection.check(&tree, Some(&visible), id(&tree, "docs/a.md")),
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
        selection.select_all(&tree, None);
        assert_eq!(selection.count(), 4);
        assert_eq!(selection.check(&tree, None, NodeId::ROOT), Check::On);
        selection.clear();
        assert!(selection.is_empty());
        assert_eq!(selection.as_bitset().len(), tree.node_count());
    }

    #[test]
    #[should_panic(expected = "sized for another tree")]
    fn a_universe_for_another_tree_panics() {
        let tree = sample();
        let selection = Selection::new(&tree);
        let wrong = FixedBitSet::with_capacity(3);
        std::hint::black_box(selection.check(&tree, Some(&wrong), NodeId::ROOT));
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
            for node in tree.ids() {
                prop_assert_eq!(
                    selection.check(&tree, universe.as_ref(), node),
                    brute_check(&tree, &selection, universe.as_ref(), node)
                );
            }
        }

        #[test]
        fn toggling_a_folder_matches_a_per_file_loop((tree, selection, universe) in scenario()) {
            for folder in tree.ids().filter(|&n| tree.kind(n).is_folder()) {
                let mut fast = selection.clone();
                fast.toggle(&tree, universe.as_ref(), folder);

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
            fast.select_all(&tree, universe.as_ref());
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
}
