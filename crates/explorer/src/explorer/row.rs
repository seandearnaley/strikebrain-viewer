//! [`RowView`]: everything one row of the tree view draws, in one call.

use super::Explorer;
use crate::{Check, Kind, NodeId};

/// One row of the tree view, from [`Explorer::row`]. Borrowed from the
/// explorer; nothing is allocated. The Size cell formats `size` with
/// [`crate::format_size`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowView<'a> {
    /// The node on this row.
    pub id: NodeId,
    /// Levels below the root: 1 for a top-level row, so the row is
    /// indented `depth - 1` steps.
    pub depth: u16,
    /// What the node is on disk.
    pub kind: Kind,
    /// The node's name.
    pub name: &'a str,
    /// The checkbox.
    pub check: Check,
    /// Whether the folder shows its children ([`Explorer::is_open`]).
    pub open: bool,
    /// Whether the row has a chevron ([`Explorer::can_open`]).
    pub can_open: bool,
    /// Bytes; a folder's is the sum of its files. `None` until the tree's
    /// sizes are read.
    pub size: Option<u64>,
    /// The lowercase extension without its dot; `""` for folders and for
    /// names without one.
    pub ext: &'a str,
}

impl Explorer {
    /// Everything row `index` of [`Explorer::rows`] draws, or `None` past
    /// the last row.
    #[must_use]
    pub fn row(&self, index: usize) -> Option<RowView<'_>> {
        let id = *self.rows.get(index)?;
        let tree = &*self.tree;
        Some(RowView {
            id,
            depth: tree.depth(id),
            kind: tree.kind(id),
            name: tree.name(id),
            check: self.check(id),
            open: self.is_open(id),
            can_open: self.can_open(id),
            size: tree.size(id),
            ext: tree.extension(id),
        })
    }
}
