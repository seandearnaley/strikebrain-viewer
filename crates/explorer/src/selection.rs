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
mod tests;
