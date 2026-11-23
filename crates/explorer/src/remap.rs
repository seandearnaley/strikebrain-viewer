//! [`IdMap`]: which node of a new tree each node of an old tree became.

use std::cmp::Ordering;
use std::fmt;

use fixedbitset::FixedBitSet;

use crate::tree::canonical_order;
use crate::{NodeId, Tree};

/// An old id with no node in the new tree.
const GONE: u32 = u32::MAX;

/// Old ids to new ids between two scans of the same folder. A node maps
/// when the new tree has a node at the same path that is also a folder, or
/// also a file; a file that became a folder, or the reverse, does not.
#[derive(Clone, PartialEq, Eq)]
pub struct IdMap {
    new_of_old: Vec<u32>,
}

impl fmt::Debug for IdMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mapped = self.new_of_old.iter().filter(|&&id| id != GONE).count();
        f.debug_struct("IdMap")
            .field("old_nodes", &self.new_of_old.len())
            .field("mapped", &mapped)
            .finish()
    }
}

impl IdMap {
    /// Pairs up `old` and `new` in one walk. Both trees list each folder's
    /// children in canonical order, so a matched pair of folders merges its
    /// children like two sorted lists: equal names (byte for byte) of the
    /// same folderness match, and [`canonical_order`] decides which side
    /// steps past a name the other lacks. Most pairs are equal, and those
    /// cost a byte comparison, not a collation. No recursion.
    pub(crate) fn new(old: &Tree, new: &Tree) -> Self {
        let mut new_of_old = vec![GONE; old.node_count()];
        new_of_old[NodeId::ROOT.index()] = 0;
        let mut pairs = vec![(NodeId::ROOT, NodeId::ROOT)];
        while let Some((a, b)) = pairs.pop() {
            let (xs, ys) = (old.children(a), new.children(b));
            let (mut i, mut j) = (0, 0);
            while let (Some(&x), Some(&y)) = (xs.get(i), ys.get(j)) {
                match merge_order(old, x, new, y) {
                    Ordering::Less => i += 1,
                    Ordering::Greater => j += 1,
                    Ordering::Equal => {
                        new_of_old[x.index()] = u32::try_from(y.index()).expect("ids fit in u32");
                        if !old.children(x).is_empty() && !new.children(y).is_empty() {
                            pairs.push((x, y));
                        }
                        i += 1;
                        j += 1;
                    }
                }
            }
        }
        Self { new_of_old }
    }

    /// The new id of `old`, or `None` if it is gone or not an id of the old
    /// tree.
    #[must_use]
    pub fn get(&self, old: NodeId) -> Option<NodeId> {
        match self.new_of_old.get(old.index()) {
            Some(&id) if id != GONE => Some(NodeId::new(id)),
            _ => None,
        }
    }

    /// The bits of `old` (one per old node) moved to their new ids, in a set
    /// of `new_len` bits. Bits of nodes that are gone are dropped.
    pub(crate) fn carry(&self, old: &FixedBitSet, new_len: usize) -> FixedBitSet {
        let mut carried = FixedBitSet::with_capacity(new_len);
        for bit in old.ones() {
            if let Some(id) = self.get(NodeId::from_index(bit)) {
                carried.insert(id.index());
            }
        }
        carried
    }
}

/// Where old child `x` falls against new child `y` in canonical order.
fn merge_order(old: &Tree, x: NodeId, new: &Tree, y: NodeId) -> Ordering {
    let (x_kind, y_kind) = (old.kind(x), new.kind(y));
    let (x_name, y_name) = (old.name(x), new.name(y));
    if x_kind.is_folder() == y_kind.is_folder() && x_name == y_name {
        Ordering::Equal
    } else {
        canonical_order(x_kind, x_name, y_kind, y_name)
    }
}

#[cfg(test)]
mod tests {
    use fixedbitset::FixedBitSet;
    use proptest::prelude::*;

    use super::IdMap;
    use crate::tree::tests::{path_sets, tree_of};
    use crate::{NodeId, Tree};

    #[test]
    fn nodes_map_by_path_and_folderness() {
        let old = Tree::from_paths(&["a/x.txt", "b.txt", "gone.txt", "kind/", "z/y"]).unwrap();
        let new =
            Tree::from_paths(&["a/x.txt", "a/new.txt", "b.txt", "kind", "z/y", "added/"]).unwrap();
        let map = IdMap::new(&old, &new);
        let at = |tree: &Tree, path: &str| tree.find(path).unwrap();
        for path in ["", "a", "a/x.txt", "b.txt", "z", "z/y"] {
            assert_eq!(map.get(at(&old, path)), Some(at(&new, path)), "{path}");
        }
        assert_eq!(map.get(at(&old, "gone.txt")), None);
        // A folder that became a file does not carry over.
        assert_eq!(map.get(at(&old, "kind")), None);
        assert_eq!(map.get(NodeId::new(999)), None);
        assert!(format!("{map:?}").contains("mapped: 6"));
    }

    #[test]
    fn bits_carry_to_their_new_ids() {
        let old = Tree::from_paths(&["a", "b", "c"]).unwrap();
        let new = Tree::from_paths(&["0", "b", "c"]).unwrap();
        let map = IdMap::new(&old, &new);
        let mut bits = FixedBitSet::with_capacity(old.node_count());
        bits.insert(old.find("a").unwrap().index());
        bits.insert(old.find("c").unwrap().index());
        let carried = map.carry(&bits, new.node_count());
        let ones: Vec<usize> = carried.ones().collect();
        assert_eq!(ones, [new.find("c").unwrap().index()]);
        assert_eq!(carried.len(), new.node_count());
    }

    proptest! {
        #[test]
        fn the_map_matches_a_path_lookup(old in path_sets(), new in path_sets()) {
            let (old, new) = (tree_of(&old), tree_of(&new));
            let map = IdMap::new(&old, &new);
            for node in old.ids() {
                let expected = new
                    .find(&old.path(node))
                    .filter(|&found| new.kind(found).is_folder() == old.kind(node).is_folder());
                prop_assert_eq!(map.get(node), expected, "{:?}", old.path(node));
            }
        }
    }
}
