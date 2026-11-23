//! The explorer engine: tree, scan, selection, filter, sort and rows.
//! This crate has no UI dependency.

/// Identifies one node of a scanned tree. Ids are positions in depth-first order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

impl NodeId {
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

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
    }
}
