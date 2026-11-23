//! Rows: the flat list of nodes on screen, which the tree view draws.

use crate::sort::DisplayOrder;
use crate::{NodeId, Tree};

/// Fills `rows` with the nodes on screen, depth first in display order.
/// Each child of the root, and of every open folder above it, is a row when
/// `shown` accepts it; a row's own children follow it when `open` says so.
/// The root itself is never a row.
///
/// A closed folder is skipped whole, so the work is the rows produced plus
/// the hidden children looked at inside open folders. The stack is explicit,
/// so a deep tree cannot overflow the thread's stack. `rows` is cleared
/// first and keeps its allocation.
pub(crate) fn flatten(
    tree: &Tree,
    order: &DisplayOrder,
    shown: impl Fn(NodeId) -> bool,
    open: impl Fn(NodeId) -> bool,
    rows: &mut Vec<NodeId>,
) {
    rows.clear();
    let mut stack = vec![order.children(tree, NodeId::ROOT).iter()];
    while let Some(top) = stack.last_mut() {
        let Some(&id) = top.next() else {
            stack.pop();
            continue;
        };
        if shown(id) {
            rows.push(id);
            if open(id) {
                let below = order.children(tree, id);
                if !below.is_empty() {
                    stack.push(below.iter());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use fixedbitset::FixedBitSet;
    use proptest::prelude::*;

    use super::flatten;
    use crate::sort::{Direction, DisplayOrder, Sort, SortKey};
    use crate::tree::tests::{path_sets, tree_of};
    use crate::{NodeId, Tree};

    fn names(tree: &Tree, rows: &[NodeId]) -> Vec<String> {
        rows.iter().map(|&id| tree.path(id)).collect()
    }

    #[test]
    fn closed_folders_hide_their_contents_and_the_root_is_no_row() {
        let tree = Tree::from_paths(&["a/b/c.txt", "a/d.txt", "e.txt"]).unwrap();
        let order = DisplayOrder::default();
        let mut rows = Vec::new();
        flatten(&tree, &order, |_| true, |_| false, &mut rows);
        assert_eq!(names(&tree, &rows), ["a", "e.txt"]);
        let a = tree.find("a").unwrap();
        flatten(&tree, &order, |_| true, |id| id == a, &mut rows);
        assert_eq!(names(&tree, &rows), ["a", "a/b", "a/d.txt", "e.txt"]);
        flatten(&tree, &order, |_| true, |_| true, &mut rows);
        assert_eq!(
            names(&tree, &rows),
            ["a", "a/b", "a/b/c.txt", "a/d.txt", "e.txt"]
        );
    }

    #[test]
    fn a_hidden_folder_hides_everything_below_it() {
        let tree = Tree::from_paths(&["a/b.txt", "c.txt"]).unwrap();
        let a = tree.find("a").unwrap();
        let mut rows = vec![NodeId::ROOT];
        flatten(
            &tree,
            &DisplayOrder::default(),
            |id| id != a,
            |_| true,
            &mut rows,
        );
        assert_eq!(names(&tree, &rows), ["c.txt"]);
    }

    /// The rows by recursion over the display order.
    fn naive(
        tree: &Tree,
        order: &DisplayOrder,
        node: NodeId,
        shown: &FixedBitSet,
        open: &FixedBitSet,
        out: &mut Vec<NodeId>,
    ) {
        for &child in order.children(tree, node) {
            if shown.contains(child.index()) {
                out.push(child);
                if open.contains(child.index()) {
                    naive(tree, order, child, shown, open, out);
                }
            }
        }
    }

    fn bits_for(tree: &Tree, picks: &[bool]) -> FixedBitSet {
        let mut set = FixedBitSet::with_capacity(tree.node_count());
        for id in tree.ids() {
            set.set(id.index(), picks.get(id.index()).copied().unwrap_or(true));
        }
        set
    }

    proptest! {
        #[test]
        fn rows_match_a_naive_recursive_walk(
            set in path_sets(),
            shown in prop::collection::vec(any::<bool>(), 300),
            open in prop::collection::vec(any::<bool>(), 300),
            descending in any::<bool>(),
        ) {
            let tree = tree_of(&set);
            let direction = if descending { Direction::Descending } else { Direction::Ascending };
            let order = DisplayOrder::new(&tree, Sort { key: SortKey::Name, direction });
            let (shown, open) = (bits_for(&tree, &shown), bits_for(&tree, &open));
            let mut rows = Vec::new();
            flatten(
                &tree,
                &order,
                |id| shown.contains(id.index()),
                |id| open.contains(id.index()),
                &mut rows,
            );
            let mut expected = Vec::new();
            naive(&tree, &order, NodeId::ROOT, &shown, &open, &mut expected);
            prop_assert_eq!(rows, expected);
        }
    }
}
