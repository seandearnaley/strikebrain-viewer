//! Turns nodes in arbitrary order into a [`Tree`] in canonical order.
//!
//! The steps, each linear apart from the sibling sorts: group children by
//! parent, sort each folder's children, number nodes in depth-first
//! pre-order with an explicit stack, copy every column into id order, then
//! one backwards pass for subtree ends (and folder sizes, when known).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use fixedbitset::FixedBitSet;

use super::names::{NameArena, extension_of};
use super::{Folder, Kind, Tree, TreeError, canonical_order, to_u32};
use crate::NodeId;
use crate::bits::Ranks;
use crate::parallel::sort_each;

/// The parent of the root, and of nodes the tree should drop.
pub(crate) const NO_PARENT: u32 = u32::MAX;

/// Nodes in any order, each naming its parent by raw index. Raw index 0 is
/// the root folder. A node whose parent is [`NO_PARENT`] or not a
/// [`Kind::Dir`] is left out of the tree, with its descendants.
#[derive(Debug)]
pub(crate) struct RawTree {
    names: NameArena,
    parent: Vec<u32>,
    kind: Vec<Kind>,
    /// One size per node when the sizes are known up front; `None` leaves
    /// the built tree without sizes.
    size: Option<Vec<u64>>,
}

impl RawTree {
    /// A raw tree holding only the root folder. With `sized`, each file's
    /// size is recorded with [`RawTree::set_size`] and the tree is built
    /// with sizes; without, the tree has none until they are read.
    pub(crate) fn new(root_name: &str, sized: bool) -> Result<Self, TreeError> {
        let mut raw = Self {
            names: NameArena::default(),
            parent: Vec::new(),
            kind: Vec::new(),
            size: sized.then(Vec::new),
        };
        raw.push(NO_PARENT, root_name, Kind::Dir)?;
        Ok(raw)
    }

    /// Adds a node and returns its raw index. Its size starts at 0.
    pub(crate) fn push(&mut self, parent: u32, name: &str, kind: Kind) -> Result<u32, TreeError> {
        // NodeId::MAX stays free so `end` can hold `node_count`.
        let raw = u32::try_from(self.parent.len())
            .ok()
            .filter(|&raw| raw < u32::MAX)
            .ok_or(TreeError::TooLarge)?;
        self.names.push(name)?;
        self.parent.push(parent);
        self.kind.push(kind);
        if let Some(size) = &mut self.size {
            size.push(0);
        }
        Ok(raw)
    }

    /// Records a file's size. Ignored when the tree was made without sizes.
    pub(crate) fn set_size(&mut self, raw: u32, bytes: u64) {
        if let Some(size) = &mut self.size {
            size[raw as usize] = bytes;
        }
    }

    /// The node's kind.
    pub(crate) fn kind(&self, raw: u32) -> Kind {
        self.kind[raw as usize]
    }

    /// Changes the node's kind, for a folder found unreadable after it was added.
    pub(crate) fn set_kind(&mut self, raw: u32, kind: Kind) {
        self.kind[raw as usize] = kind;
    }

    /// Builds the tree rooted at raw index 0.
    pub(crate) fn build(self, root_path: PathBuf) -> Result<Tree, TreeError> {
        let (child_start, child_ids) = self.sorted_children();
        let (order, depth) = self.pre_order(&child_start, &child_ids)?;
        self.into_tree(root_path, &order, depth)
    }

    /// Each folder's children as one sorted slice, indexed by raw id.
    fn sorted_children(&self) -> (Vec<u32>, Vec<u32>) {
        let n = self.parent.len();
        let mut start = vec![0_u32; n + 1];
        for &parent in &self.parent {
            if parent != NO_PARENT {
                start[parent as usize + 1] += 1;
            }
        }
        for i in 0..n {
            start[i + 1] += start[i];
        }
        let mut next = start.clone();
        let mut ids = vec![0_u32; start[n] as usize];
        for (raw, &parent) in self.parent.iter().enumerate() {
            if parent != NO_PARENT {
                let slot = &mut next[parent as usize];
                ids[*slot as usize] = to_u32(raw);
                *slot += 1;
            }
        }
        self.sort_siblings(&start, &mut ids);
        (start, ids)
    }

    /// Sorts each folder's slice of `ids` into canonical order, on threads
    /// when there are many.
    fn sort_siblings(&self, start: &[u32], ids: &mut [u32]) {
        let mut folders = Vec::new();
        let mut rest = ids;
        for window in start.windows(2) {
            let (siblings, tail) =
                std::mem::take(&mut rest).split_at_mut((window[1] - window[0]) as usize);
            rest = tail;
            if siblings.len() > 1 {
                folders.push(siblings);
            }
        }
        sort_each(folders, &|&a: &u32, &b: &u32| {
            canonical_order(
                self.kind[a as usize],
                self.names.get(a as usize),
                self.kind[b as usize],
                self.names.get(b as usize),
            )
        });
    }

    /// Raw indices in depth-first pre-order from the root, with each depth.
    fn pre_order(
        &self,
        child_start: &[u32],
        child_ids: &[u32],
    ) -> Result<(Vec<u32>, Vec<u16>), TreeError> {
        let n = self.parent.len();
        let mut order = Vec::with_capacity(n);
        let mut depths = Vec::with_capacity(n);
        let mut stack = vec![(0_u32, 0_u16)];
        while let Some((raw, depth)) = stack.pop() {
            order.push(raw);
            depths.push(depth);
            let i = raw as usize;
            let children = &child_ids[child_start[i] as usize..child_start[i + 1] as usize];
            if self.kind[i] == Kind::Dir && !children.is_empty() {
                let child_depth = depth.checked_add(1).ok_or(TreeError::TooDeep)?;
                stack.extend(children.iter().rev().map(|&child| (child, child_depth)));
            }
        }
        Ok((order, depths))
    }

    /// Copies every column into id order and derives the rest.
    fn into_tree(
        self,
        root_path: PathBuf,
        order: &[u32],
        depth: Vec<u16>,
    ) -> Result<Tree, TreeError> {
        let n = order.len();
        let mut id_of_raw = vec![0_u32; self.parent.len()];
        for (id, &raw) in order.iter().enumerate() {
            id_of_raw[raw as usize] = to_u32(id);
        }

        let bytes = self.names.byte_len();
        let mut names = NameArena::with_capacity(n, bytes);
        let mut haystack = NameArena::with_capacity(n, bytes + n);
        let mut parent = Vec::with_capacity(n);
        let mut kind = Vec::with_capacity(n);
        let mut size = self.size.as_ref().map(|_| Vec::with_capacity(n));
        let mut ext = Vec::with_capacity(n);
        let mut extensions = Extensions::default();
        let mut files = FixedBitSet::with_capacity(n);
        for (id, &raw) in order.iter().enumerate() {
            let raw = raw as usize;
            let name = self.names.get(raw);
            names.push(name)?;
            // Lowercasing can lengthen a name, so this arena checks again.
            haystack.push_lowercase_terminated(name)?;
            let k = self.kind[raw];
            kind.push(k);
            parent.push(NodeId::new(match id {
                0 => 0,
                _ => id_of_raw[self.parent[raw] as usize],
            }));
            if let (Some(size), Some(raw_size)) = (&mut size, &self.size) {
                size.push(raw_size[raw]);
            }
            if k.is_file() {
                files.insert(id);
                let lower = haystack.get(id);
                ext.push(
                    extensions.intern(extension_of(lower.strip_suffix('\0').unwrap_or(lower))),
                );
            } else {
                ext.push(0);
            }
        }

        // Children have larger ids than their parent, so one backwards pass
        // sees every node's subtree complete before it reaches the parent.
        let mut end: Vec<u32> = (1..=n).map(to_u32).collect();
        for id in (1..n).rev() {
            let p = parent[id].index();
            end[p] = end[p].max(end[id]);
        }

        let (child_start, child_ids) = children_by_id(&parent);
        let tree = Tree {
            folders: Box::new([Folder {
                top: NodeId::ROOT,
                path: root_path,
                sizes: Arc::default(),
            }]),
            names,
            haystack,
            parent,
            end,
            depth,
            kind,
            ext,
            extensions: extensions.list,
            file_ranks: Ranks::new(files.as_slice().iter().copied()),
            files,
            child_start,
            child_ids,
        };
        if let Some(size) = size {
            tree.set_sizes(&size);
        }
        Ok(tree)
    }
}

/// Each node's children in ascending id order, which is canonical order.
pub(super) fn children_by_id(parent: &[NodeId]) -> (Vec<u32>, Vec<NodeId>) {
    let n = parent.len();
    let mut start = vec![0_u32; n + 1];
    for p in parent.iter().skip(1) {
        start[p.index() + 1] += 1;
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    let mut next = start.clone();
    let mut ids = vec![NodeId::ROOT; n.saturating_sub(1)];
    for (id, p) in parent.iter().enumerate().skip(1) {
        let slot = &mut next[p.index()];
        ids[*slot as usize] = NodeId::new(to_u32(id));
        *slot += 1;
    }
    (start, ids)
}

/// The interned extension table. Entry 0 is "no extension".
pub(super) struct Extensions {
    pub(super) list: Vec<Box<str>>,
    index: HashMap<Box<str>, u16>,
}

impl Default for Extensions {
    fn default() -> Self {
        Self {
            list: vec![Box::from("")],
            index: HashMap::new(),
        }
    }
}

impl Extensions {
    /// The extension's index, adding it if new. A full table gives 0.
    pub(super) fn intern(&mut self, ext: &str) -> u16 {
        if ext.is_empty() {
            return 0;
        }
        if let Some(&i) = self.index.get(ext) {
            return i;
        }
        let Ok(i) = u16::try_from(self.list.len()) else {
            return 0;
        };
        self.list.push(Box::from(ext));
        self.index.insert(Box::from(ext), i);
        i
    }
}

#[cfg(test)]
mod tests {
    use super::{Extensions, NO_PARENT, RawTree};
    use crate::{Kind, NodeId};

    #[test]
    fn a_node_under_a_file_or_without_a_parent_is_dropped() {
        let mut raw = RawTree::new("root", true).unwrap();
        let file = raw.push(0, "file", Kind::File).unwrap();
        raw.set_size(file, 3);
        let under = raw.push(file, "under-file", Kind::File).unwrap();
        raw.set_size(under, 5);
        let orphan = raw.push(NO_PARENT, "orphan", Kind::File).unwrap();
        raw.set_size(orphan, 7);
        let tree = raw.build("/r".into()).unwrap();
        assert_eq!(tree.node_count(), 2);
        assert_eq!(tree.name(NodeId::ROOT), "root");
        assert_eq!(tree.size(NodeId::ROOT), Some(3));
    }

    #[test]
    fn an_unreadable_folder_is_a_leaf() {
        let mut raw = RawTree::new("root", false).unwrap();
        let dir = raw.push(0, "locked", Kind::Dir).unwrap();
        raw.push(dir, "inside", Kind::File).unwrap();
        raw.set_kind(dir, Kind::Unreadable);
        assert_eq!(raw.kind(dir), Kind::Unreadable);
        let tree = raw.build(std::path::PathBuf::new()).unwrap();
        assert_eq!(tree.node_count(), 2);
        assert!(!tree.has_sizes());
        assert_eq!(tree.size(NodeId::ROOT), None);
        assert_eq!(tree.kind(NodeId::new(1)), Kind::Unreadable);
        assert_eq!(tree.subtree(NodeId::new(1)), 1..2);
    }

    #[test]
    fn a_tree_big_enough_to_sort_on_threads_keeps_canonical_order() {
        let mut raw = RawTree::new("", false).unwrap();
        // Two folders, each past the threshold for sorting on threads,
        // filled in reverse order.
        let size = (1 << 14) + 10;
        for folder in ["b", "a"] {
            let dir = raw.push(0, folder, Kind::Dir).unwrap();
            for i in (0..size).rev() {
                raw.push(dir, &format!("f{i}"), Kind::File).unwrap();
            }
        }
        let tree = raw.build(std::path::PathBuf::new()).unwrap();
        assert_eq!(tree.path(NodeId::new(1)), "a");
        assert_eq!(tree.path(NodeId::new(2)), "a/f0");
        assert_eq!(tree.path(NodeId::new(3)), "a/f1");
        let b = tree.find("b").unwrap();
        let names: Vec<&str> = tree
            .children(b)
            .iter()
            .take(3)
            .map(|&c| tree.name(c))
            .collect();
        assert_eq!(names, ["f0", "f1", "f2"]);
        let last = *tree.children(b).last().unwrap();
        assert_eq!(tree.name(last), format!("f{}", size - 1));
    }

    #[test]
    fn a_full_extension_table_maps_new_extensions_to_zero() {
        let mut table = Extensions::default();
        for i in 1..=u32::from(u16::MAX) {
            assert_eq!(u32::from(table.intern(&format!("e{i}"))), i);
        }
        assert_eq!(table.intern("one-too-many"), 0);
        assert_eq!(table.intern("e7"), 7);
        assert_eq!(table.intern(""), 0);
    }
}
