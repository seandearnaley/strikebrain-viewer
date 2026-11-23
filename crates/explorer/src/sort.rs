//! Display order: how each folder's children line up on screen.
//!
//! The tree is shared and never changes, and its ids already follow the
//! default order (Name ascending). Any other order is a permuted copy of the
//! tree's children lists, laid out the same way, which the
//! [`crate::Explorer`] owns. Sorting compares precomputed [`Rank`]s: one
//! integer per node for the key, and the node's id for the name, since ids
//! follow natural name order among siblings of the same kind.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::parallel::sort_each;
use crate::{NodeId, Tree, compare_names};

/// The column a view sorts by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortKey {
    /// Natural, case-insensitive name order.
    #[default]
    Name,
    /// Extension (natural order), then name.
    Type,
    /// Bytes, then name. Folders compare by the sum of their files. Before
    /// a scanned tree has sizes, every size counts as 0.
    Size,
}

/// Which way the sort key runs. The name tiebreak is always ascending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Smallest first.
    #[default]
    Ascending,
    /// Largest first.
    Descending,
}

impl Direction {
    /// The other direction.
    #[must_use]
    pub fn flipped(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }
}

/// How a view orders each folder's children. Folders always come first.
/// The default is Name ascending, the tree's own order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Sort {
    /// The column.
    pub key: SortKey,
    /// The column's direction.
    pub direction: Direction,
}

impl Sort {
    /// The sort after a click on the `key` column header: the same column
    /// flips its direction, another column starts ascending.
    #[must_use]
    pub fn clicked(self, key: SortKey) -> Self {
        if key == self.key {
            Self {
                key,
                direction: self.direction.flipped(),
            }
        } else {
            Self {
                key,
                direction: Direction::Ascending,
            }
        }
    }
}

/// One sibling's place under a sort, computed once per node so that a
/// comparison reads three fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rank {
    /// Folders come before files, whatever the direction.
    pub(crate) folder: bool,
    /// The sort key: the name's position for Name, the extension's position
    /// in natural order for Type, bytes for Size.
    pub(crate) key: u64,
    /// The name's position in natural order among siblings of the same kind.
    pub(crate) name: u32,
}

/// The order of two siblings under a sort: folders before files in both
/// directions, then `key` in `direction`, then `name` ascending whatever the
/// direction.
///
/// Learning exercise: Sean may rewrite this body; the tests below define it.
pub(crate) fn sibling_order(a: Rank, b: Rank, direction: Direction) -> Ordering {
    let key = match direction {
        Direction::Ascending => a.key.cmp(&b.key),
        Direction::Descending => b.key.cmp(&a.key),
    };
    b.folder.cmp(&a.folder).then(key).then(a.name.cmp(&b.name))
}

/// Each folder's children in display order and, for any order but the
/// tree's own, every node's position in the fully expanded display order.
#[derive(Clone, Debug, Default)]
pub(crate) struct DisplayOrder {
    /// `None` for the tree's own order, which needs no copy.
    permuted: Option<Permuted>,
    /// A Size order built before the tree had sizes, so every size counted
    /// as 0. It is stale once the sizes arrive.
    awaiting_sizes: bool,
}

#[derive(Clone, Debug)]
struct Permuted {
    /// Laid out like the tree's children: folder by folder in id order.
    children: Vec<NodeId>,
    /// Every node in depth-first display order, the root first.
    pre_order: Vec<NodeId>,
    /// Each node's index in `pre_order`.
    position: Vec<u32>,
}

impl DisplayOrder {
    /// The display order of `tree` under `sort`.
    pub(crate) fn new(tree: &Tree, sort: Sort) -> Self {
        if sort == Sort::default() {
            return Self::default();
        }
        // One look at the sizes: another thread may store them meanwhile,
        // and the keys must not mix zeros with real sizes.
        let sizes = tree.sizes();
        let keys = keys(tree, sort.key, sizes);
        let rank = |id: NodeId| Rank {
            folder: tree.kind(id).is_folder(),
            key: keys[id.index()],
            name: u32::try_from(id.index()).expect("ids fit in u32"),
        };
        let mut children = tree.child_ids().to_vec();
        let mut slices = Vec::new();
        let mut rest = children.as_mut_slice();
        let mut taken = 0;
        for folder in tree.ids() {
            let range = tree.child_range(folder);
            if range.len() > 1 {
                let (_, tail) = std::mem::take(&mut rest).split_at_mut(range.start - taken);
                let (siblings, tail) = tail.split_at_mut(range.len());
                slices.push(siblings);
                rest = tail;
                taken = range.end;
            }
        }
        sort_each(slices, &|&a: &NodeId, &b: &NodeId| {
            sibling_order(rank(a), rank(b), sort.direction)
        });

        let pre_order = pre_order(tree, &children);
        let mut position = vec![0; pre_order.len()];
        for (at, id) in pre_order.iter().enumerate() {
            position[id.index()] = u32::try_from(at).expect("positions fit in u32");
        }
        Self {
            permuted: Some(Permuted {
                children,
                pre_order,
                position,
            }),
            awaiting_sizes: sort.key == SortKey::Size && sizes.is_none(),
        }
    }

    /// Whether the order was built by size before `tree` had sizes, and
    /// the sizes have since arrived.
    pub(crate) fn is_stale(&self, tree: &Tree) -> bool {
        self.awaiting_sizes && tree.has_sizes()
    }

    /// The folder's children in display order.
    pub(crate) fn children<'a>(&'a self, tree: &'a Tree, id: NodeId) -> &'a [NodeId] {
        match &self.permuted {
            None => tree.children(id),
            Some(permuted) => &permuted.children[tree.child_range(id)],
        }
    }

    /// The node's index in the fully expanded display order, the root at 0.
    pub(crate) fn position(&self, id: NodeId) -> usize {
        match &self.permuted {
            None => id.index(),
            Some(permuted) => permuted.position[id.index()] as usize,
        }
    }

    /// Every node in fully expanded display order, or `None` when that is
    /// id order.
    pub(crate) fn pre_order(&self) -> Option<&[NodeId]> {
        self.permuted
            .as_ref()
            .map(|permuted| permuted.pre_order.as_slice())
    }
}

/// Each node's sort key: see [`Rank::key`]. Without `sizes`, every size is 0.
fn keys(tree: &Tree, key: SortKey, sizes: Option<&[u64]>) -> Vec<u64> {
    match key {
        SortKey::Name => (0..tree.node_count()).map(|i| i as u64).collect(),
        SortKey::Size => sizes.map_or_else(|| vec![0; tree.node_count()], <[u64]>::to_vec),
        SortKey::Type => type_keys(tree),
    }
}

/// Each file's extension as its position among every distinct extension in
/// natural order; folders get 0, like files without an extension. The
/// interned table covers almost every tree; extensions past its end are
/// collected from the names.
fn type_keys(tree: &Tree) -> Vec<u64> {
    let table = tree.extensions();
    let mut extra: HashSet<&str> = HashSet::new();
    for file in tree.files().ones() {
        let id = NodeId::from_index(file);
        if tree.ext_id(id) == 0 && !tree.extension(id).is_empty() {
            extra.insert(tree.extension(id));
        }
    }
    let mut all: Vec<&str> = table.iter().map(|ext| &**ext).collect();
    all.extend(extra);
    all.sort_unstable_by(|a, b| compare_names(a, b));
    let position: HashMap<&str, u64> = all.iter().zip(0..).map(|(&ext, at)| (ext, at)).collect();
    let by_id: Vec<u64> = table.iter().map(|ext| position[&**ext]).collect();
    tree.ids()
        .map(|id| match tree.ext_id(id) {
            0 if tree.is_file(id) => position[tree.extension(id)],
            0 => 0,
            ext => by_id[usize::from(ext)],
        })
        .collect()
}

/// Every node in depth-first order over `children` (laid out like the
/// tree's), the root first. An explicit stack, so depth cannot overflow it.
fn pre_order(tree: &Tree, children: &[NodeId]) -> Vec<NodeId> {
    let mut order = Vec::with_capacity(tree.node_count());
    order.push(NodeId::ROOT);
    let mut stack = vec![children[tree.child_range(NodeId::ROOT)].iter()];
    while let Some(top) = stack.last_mut() {
        if let Some(&id) = top.next() {
            order.push(id);
            let below = &children[tree.child_range(id)];
            if !below.is_empty() {
                stack.push(below.iter());
            }
        } else {
            stack.pop();
        }
    }
    order
}

#[cfg(test)]
mod tests;
