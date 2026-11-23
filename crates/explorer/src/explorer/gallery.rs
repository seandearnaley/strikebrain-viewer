//! The gallery: every checked file in display order across the whole tree,
//! whatever the filter shows or which folders are open.
//!
//! Under the default sort, display order is id order, so every question is
//! a bit search or a ranged count over the selection: microseconds on a
//! million nodes. Under another sort the explorer keeps the gallery as a
//! list in display order, made by the first question after the checks, the
//! sort or the tree change, and answers each question with a binary search
//! over it. Either way the stage can ask on every render.

use std::fmt;
use std::slice;

use fixedbitset::FixedBitSet;

use super::Explorer;
use crate::{NodeId, bits};

/// The checked files in display order; see [`Explorer::gallery`].
pub struct Gallery<'a> {
    files: Files<'a>,
}

enum Files<'a> {
    /// Display order is id order: the selection's set bits from `at` on.
    InIdOrder { set: &'a FixedBitSet, at: usize },
    /// The explorer's list in display order.
    Listed(slice::Iter<'a, NodeId>),
}

impl fmt::Debug for Gallery<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gallery").finish_non_exhaustive()
    }
}

impl Iterator for Gallery<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        match &mut self.files {
            Files::InIdOrder { set, at } => {
                let found = bits::next_one(set, *at)?;
                *at = found + 1;
                Some(NodeId::from_index(found))
            }
            Files::Listed(files) => files.next().copied(),
        }
    }
}

impl Explorer {
    /// Every checked file in display order across the whole tree. The
    /// filter and closed folders do not affect it.
    #[must_use]
    pub fn gallery(&self) -> Gallery<'_> {
        self.gallery_from(0)
    }

    /// The gallery from its file at `start` on: what a grid of tiles
    /// scrolled down to `start` draws. Empty past the end. Under the tree's
    /// own order this finds the file a word of the selection at a time,
    /// some 15,000 words for a million nodes; under another sort it is an
    /// index into the list.
    #[must_use]
    pub fn gallery_from(&self, start: usize) -> Gallery<'_> {
        let files = match self.listed() {
            None => {
                let set = self.selection.as_bitset();
                // Past the end, the search starts past the last bit.
                let at = bits::nth_one(set, start).unwrap_or(set.len());
                Files::InIdOrder { set, at }
            }
            Some(list) => Files::Listed(list.get(start..).unwrap_or_default().iter()),
        };
        Gallery { files }
    }

    /// How many files the gallery holds.
    #[must_use]
    pub fn gallery_len(&self) -> usize {
        self.selection.count()
    }

    /// The index of `id` in the gallery, or `None` if it is not checked.
    /// Cheap enough for every render.
    #[must_use]
    pub fn gallery_position(&self, id: NodeId) -> Option<usize> {
        if !self.selection.is_selected(id) {
            return None;
        }
        Some(match self.listed() {
            None => self.selection.as_bitset().count_ones(..id.index()),
            Some(list) => self.listed_before(list, id),
        })
    }

    /// The gallery file after `id` in display order. `id` need not be in
    /// the gallery: from any node this is the first checked file after it.
    #[must_use]
    pub fn gallery_next(&self, id: NodeId) -> Option<NodeId> {
        match self.listed() {
            None => {
                bits::next_one(self.selection.as_bitset(), id.index() + 1).map(NodeId::from_index)
            }
            Some(list) => {
                let at = self.order.position(id);
                let after = list.partition_point(|&file| self.order.position(file) <= at);
                list.get(after).copied()
            }
        }
    }

    /// The gallery file before `id` in display order. `id` need not be in
    /// the gallery: from any node this is the last checked file before it.
    #[must_use]
    pub fn gallery_previous(&self, id: NodeId) -> Option<NodeId> {
        match self.listed() {
            None => bits::prev_one(self.selection.as_bitset(), id.index()).map(NodeId::from_index),
            Some(list) => {
                let before = self.listed_before(list, id);
                before.checked_sub(1).map(|last| list[last])
            }
        }
    }

    /// How many listed files come before `id` in display order.
    fn listed_before(&self, list: &[NodeId], id: NodeId) -> usize {
        let at = self.order.position(id);
        list.partition_point(|&file| self.order.position(file) < at)
    }

    /// The checked files in display order, made on first use after a
    /// change; `None` under the tree's own order, where id order is display
    /// order and the selection's bits answer directly.
    fn listed(&self) -> Option<&[NodeId]> {
        let pre_order = self.order.pre_order()?;
        Some(self.gallery.get_or_init(|| {
            let count = self.selection.count();
            if count.saturating_mul(SORT_BELOW) < pre_order.len() {
                // Few checked files: sorting them beats a pass over the tree.
                let mut files: Vec<NodeId> = self.selection.iter().collect();
                files.sort_unstable_by_key(|&file| self.order.position(file));
                files.into_boxed_slice()
            } else {
                pre_order
                    .iter()
                    .copied()
                    .filter(|&node| self.selection.is_selected(node))
                    .collect()
            }
        }))
    }

    /// Forgets the gallery list after a change to the checks, the order or
    /// the tree.
    pub(super) fn forget_gallery(&mut self) {
        self.gallery.take();
    }
}

/// The list is made by sorting the checked files when fewer than one node
/// in this many is checked, and by a pass over the display order otherwise.
const SORT_BELOW: usize = 16;
