//! The gallery: every checked file in display order across the whole tree,
//! whatever the filter shows or which folders are open.
//!
//! Positions are indices into the fully expanded display order. Under the
//! default sort that order is id order, so every question is a bit search
//! over the selection. Under another sort the explorer keeps the order as an
//! array, and a question scans it: linear, about a millisecond on a million
//! nodes, and asked once per key press or stage update.

use super::Explorer;
use crate::{NodeId, bits};

/// The checked files in display order; see [`Explorer::gallery`].
#[derive(Debug)]
pub struct Gallery<'a> {
    explorer: &'a Explorer,
    /// The display position to look from.
    next: usize,
}

impl Iterator for Gallery<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let found = self.explorer.selected_from(self.next)?;
        self.next = self.explorer.order.position(found) + 1;
        Some(found)
    }
}

impl Explorer {
    /// Every checked file in display order across the whole tree. The
    /// filter and closed folders do not affect it.
    #[must_use]
    pub fn gallery(&self) -> Gallery<'_> {
        Gallery {
            explorer: self,
            next: 0,
        }
    }

    /// How many files the gallery holds.
    #[must_use]
    pub fn gallery_len(&self) -> usize {
        self.selection.count()
    }

    /// The index of `id` in the gallery, or `None` if it is not checked.
    #[must_use]
    pub fn gallery_position(&self, id: NodeId) -> Option<usize> {
        if !self.selection.is_selected(id) {
            return None;
        }
        let at = self.order.position(id);
        Some(match self.order.pre_order() {
            None => self.selection.as_bitset().count_ones(..at),
            Some(order) => order[..at]
                .iter()
                .filter(|&&node| self.selection.is_selected(node))
                .count(),
        })
    }

    /// The gallery file after `id` in display order. `id` need not be in
    /// the gallery: from any node this is the first checked file after it.
    #[must_use]
    pub fn gallery_next(&self, id: NodeId) -> Option<NodeId> {
        self.selected_from(self.order.position(id) + 1)
    }

    /// The gallery file before `id` in display order. `id` need not be in
    /// the gallery: from any node this is the last checked file before it.
    #[must_use]
    pub fn gallery_previous(&self, id: NodeId) -> Option<NodeId> {
        let before = self.order.position(id);
        match self.order.pre_order() {
            None => bits::prev_one(self.selection.as_bitset(), before).map(NodeId::from_index),
            Some(order) => order[..before]
                .iter()
                .rev()
                .copied()
                .find(|&node| self.selection.is_selected(node)),
        }
    }

    /// The first checked file at display position `from` or later.
    fn selected_from(&self, from: usize) -> Option<NodeId> {
        match self.order.pre_order() {
            None => bits::next_one(self.selection.as_bitset(), from).map(NodeId::from_index),
            Some(order) => order
                .get(from..)?
                .iter()
                .copied()
                .find(|&node| self.selection.is_selected(node)),
        }
    }
}
