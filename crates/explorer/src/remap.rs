//! [`IdMap`]: which node of a new tree each node of an old tree became.

use std::cmp::Ordering;
use std::fmt;

use fixedbitset::FixedBitSet;

use crate::tree::canonical_order;
use crate::{NodeId, Tree};

/// An old id with no node in the new tree.
const GONE: u32 = u32::MAX;

/// Old ids to new ids between two scans of the same folders. A node maps
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
    /// Pairs up `old` and `new` by path, in one walk. Between two trees of
    /// one folder, the roots pair. When `new` is joined, each scanned
    /// folder of `old` pairs with the new node at its place on disk
    /// ([`Tree::find_abs`]): a joined tree's top-level folders, or the root
    /// of a tree of one folder; the root of a joined tree pairs only with
    /// the root of another. When only `old` is joined, its top-level
    /// folders pair with their places in `new` the same way. Then every
    /// pair of folders merges its children (see [`IdMap::merge`]).
    pub(crate) fn new(old: &Tree, new: &Tree) -> Self {
        let mut map = Self::gone(old);
        let pairs: Vec<(NodeId, NodeId)> = if old.is_joined() || new.is_joined() {
            if old.is_joined() && new.is_joined() {
                map.new_of_old[NodeId::ROOT.index()] = 0;
            }
            old.folders()
                .filter_map(|top| Some((top, new.find_abs(&old.abs_path(top))?)))
                .collect()
        } else {
            vec![(NodeId::ROOT, NodeId::ROOT)]
        };
        map.merge(old, new, pairs);
        map
    }

    /// The map across a join made from `old`: each subtree in `kept`, given
    /// by its old and new top, moved whole; each node in `merged` paired
    /// with its new node, and the folders below merged by name. Everything
    /// else is gone.
    pub(crate) fn rejoined(
        old: &Tree,
        new: &Tree,
        kept: &[(NodeId, NodeId)],
        merged: Vec<(NodeId, NodeId)>,
    ) -> Self {
        let mut map = Self::gone(old);
        map.new_of_old[NodeId::ROOT.index()] = 0;
        for &(from, to) in kept {
            let range = old.subtree(from);
            let shift =
                |id: usize| u32::try_from(id - from.index() + to.index()).expect("ids fit in u32");
            for id in range.clone() {
                map.new_of_old[id] = shift(id);
            }
        }
        map.merge(old, new, merged);
        map
    }

    /// A map of every old node to nothing.
    fn gone(old: &Tree) -> Self {
        Self {
            new_of_old: vec![GONE; old.node_count()],
        }
    }

    /// Maps each pair whose nodes are both folders or both files, then
    /// merges their children by name. Both trees list a folder's children
    /// in canonical order, so a matched pair of folders merges its children
    /// like two sorted lists: equal names (byte for byte) of the same
    /// folderness match, and [`canonical_order`] decides which side steps
    /// past a name the other lacks. Most pairs are equal, and those cost a
    /// byte comparison, not a collation. No recursion.
    fn merge(&mut self, old: &Tree, new: &Tree, pairs: Vec<(NodeId, NodeId)>) {
        let new_id = |id: NodeId| u32::try_from(id.index()).expect("ids fit in u32");
        let mut stack = Vec::with_capacity(pairs.len());
        for (a, b) in pairs {
            if old.kind(a).is_folder() == new.kind(b).is_folder() {
                self.new_of_old[a.index()] = new_id(b);
                stack.push((a, b));
            }
        }
        while let Some((a, b)) = stack.pop() {
            let (xs, ys) = (old.children(a), new.children(b));
            let (mut i, mut j) = (0, 0);
            while let (Some(&x), Some(&y)) = (xs.get(i), ys.get(j)) {
                match merge_order(old, x, new, y) {
                    Ordering::Less => i += 1,
                    Ordering::Greater => j += 1,
                    Ordering::Equal => {
                        self.new_of_old[x.index()] = new_id(y);
                        if !old.children(x).is_empty() && !new.children(y).is_empty() {
                            stack.push((x, y));
                        }
                        i += 1;
                        j += 1;
                    }
                }
            }
        }
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
