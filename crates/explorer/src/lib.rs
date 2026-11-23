//! The explorer engine: tree, scan, selection, filter, sort and rows.
//! This crate has no UI dependency.

mod bits;
mod selection;
mod tree;

pub use fixedbitset::FixedBitSet;
pub use selection::{Check, Selection};
pub use tree::{Kind, Tree, TreeError, compare_names};

/// Identifies one node of a scanned tree. Ids are positions in depth-first order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

impl NodeId {
    /// The root folder of every tree.
    pub const ROOT: Self = Self(0);

    /// Wraps a raw id.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The id at array index `i`. A tree holds fewer than `u32::MAX` nodes,
    /// so every index into its arrays fits.
    pub(crate) fn from_index(i: usize) -> Self {
        Self(u32::try_from(i).expect("a tree holds fewer than u32::MAX nodes"))
    }

    /// The id as an index into the tree's arrays.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[cfg(test)]
mod tests {
    use super::NodeId;

    #[test]
    fn node_id_round_trips_its_index() {
        assert_eq!(NodeId::new(7).index(), 7);
        assert_eq!(NodeId::new(u32::MAX).index(), u32::MAX as usize);
    }

    #[test]
    fn the_root_is_id_zero() {
        assert_eq!(NodeId::ROOT.index(), 0);
    }
}
