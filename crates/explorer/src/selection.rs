//! Which files are checked, and the check state each row shows.
//!
//! A folder's state depends on a universe: the files a filter lets through,
//! or every file when no filter is active. The universe is the filter's
//! [`Visibility`].
//!
//! A row asks for its state on every frame, so counting must not scan a
//! folder's range: a folder high in a deep tree spans almost every node.
//! The universe's file count per range is precomputed (by the tree, and by
//! each filter run), and the selected count comes from running counts that
//! the selection builds once after each change, for the universe asked
//! about. Every folder's state is then constant time.

use std::fmt;
use std::ops::Range;
use std::sync::{Mutex, PoisonError};

use fixedbitset::FixedBitSet;

use crate::bits::Ranks;
use crate::{NodeId, Tree, Visibility, bits};

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
pub struct Selection {
    bits: FixedBitSet,
    /// Running counts of the selected files of one universe, made by the
    /// first [`Selection::check`] after a change and dropped by the next
    /// change. A lock, not a cell, so a selection stays `Sync`.
    counts: Mutex<Option<Counts>>,
}

/// Running counts of `bits & universe`.
struct Counts {
    /// [`Visibility::stamp`] of the universe.
    stamp: u64,
    ranks: Ranks,
}

impl Clone for Selection {
    fn clone(&self) -> Self {
        Self::from_set(self.bits.clone())
    }
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        self.bits == other.bits
    }
}

impl Eq for Selection {}

impl fmt::Debug for Selection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Selection")
            .field("files", &self.bits.ones().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Selection {
    /// An empty selection sized for `tree`.
    #[must_use]
    pub fn new(tree: &Tree) -> Self {
        Self::from_set(FixedBitSet::with_capacity(tree.node_count()))
    }

    /// A selection of the files among `bits`, one bit per node of `tree`.
    /// Folder bits are dropped.
    pub(crate) fn from_bits(tree: &Tree, mut bits: FixedBitSet) -> Self {
        assert_eq!(bits.len(), tree.node_count(), "one bit per node");
        bits.intersect_with(tree.files());
        Self::from_set(bits)
    }

    fn from_set(bits: FixedBitSet) -> Self {
        Self {
            bits,
            counts: Mutex::new(None),
        }
    }

    /// The checkbox for `id`. A file is [`Check::On`] when selected, whatever
    /// the universe. A folder compares the selected files of its universe
    /// (the files `visibility` shows) with the whole universe; both are
    /// counts over its subtree, each in constant time.
    ///
    /// Learning exercise: Sean may rewrite this body; the tests below define it.
    ///
    /// # Panics
    ///
    /// If the selection or the visibility was made for another tree.
    #[must_use]
    pub fn check(&self, tree: &Tree, visibility: &Visibility, id: NodeId) -> Check {
        self.assert_sized_for(tree, visibility);
        if tree.is_file(id) {
            return if self.is_selected(id) {
                Check::On
            } else {
                Check::Off
            };
        }
        let range = tree.subtree(id);
        let total = visibility.file_count(tree, range.clone());
        let selected = self.count_in(tree, visibility, range);
        match selected {
            0 => Check::Off,
            _ if selected == total => Check::On,
            _ => Check::Mixed,
        }
    }

    /// The selected files of `visibility`'s universe among the ids in
    /// `range`. Builds the running counts for that universe if the last ones
    /// were for another universe or the selection changed since.
    fn count_in(&self, tree: &Tree, visibility: &Visibility, range: Range<usize>) -> usize {
        let selected = self.bits.as_slice();
        let universe = visibility.universe_files(tree).as_slice();
        let word = |w: usize| selected[w] & universe[w];
        let stamp = visibility.stamp();
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        if counts.as_ref().is_some_and(|made| made.stamp != stamp) {
            *counts = None;
        }
        let counts = counts.get_or_insert_with(|| Counts {
            stamp,
            ranks: Ranks::new((0..selected.len()).map(word)),
        });
        counts.ranks.count(word, range)
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
            self.changed();
        }
    }

    /// The click on a checkbox: flips a file; a folder that is
    /// [`Check::On`] clears every file in its subtree, hidden ones included,
    /// and any other folder selects every file of its universe.
    ///
    /// # Panics
    ///
    /// If the selection or the visibility was made for another tree.
    pub fn toggle(&mut self, tree: &Tree, visibility: &Visibility, id: NodeId) {
        if tree.is_file(id) {
            self.bits.toggle(id.index());
        } else if self.check(tree, visibility, id) == Check::On {
            self.bits.remove_range(tree.subtree(id));
        } else {
            let universe = visibility.universe_files(tree);
            bits::or_range(&mut self.bits, universe, tree.subtree(id));
        }
        self.changed();
    }

    /// Selects every file in the universe. Files already selected stay
    /// selected.
    ///
    /// # Panics
    ///
    /// If the selection or the visibility was made for another tree.
    pub fn select_all(&mut self, tree: &Tree, visibility: &Visibility) {
        self.assert_sized_for(tree, visibility);
        let universe = visibility.universe_files(tree);
        bits::or_range(&mut self.bits, universe, tree.subtree(NodeId::ROOT));
        self.changed();
    }

    /// Deselects everything.
    pub fn clear(&mut self) {
        self.bits.clear();
        self.changed();
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

    /// Drops the running counts, which no longer match the bits.
    fn changed(&mut self) {
        *self
            .counts
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn assert_sized_for(&self, tree: &Tree, visibility: &Visibility) {
        let n = tree.node_count();
        assert!(
            self.bits.len() == n && visibility.universe().is_none_or(|u| u.len() == n),
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
}
