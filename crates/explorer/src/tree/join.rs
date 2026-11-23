//! [`Tree::join`]: several scanned folders as the top-level folders of one
//! tree.
//!
//! Each folder's nodes keep their order and come one folder after another,
//! so ids stay depth-first pre-order and every subtree stays one range.
//! Every column is copied in bulk with its ids shifted; nothing is sorted
//! again, so a join costs a pass over the nodes.

use std::sync::{Arc, OnceLock};

use fixedbitset::FixedBitSet;

use super::build::{Extensions, children_by_id};
use super::names::NameArena;
use super::{Folder, Kind, Tree, TreeError, to_u32};
use crate::NodeId;
use crate::bits::Ranks;

impl Tree {
    /// A tree whose top-level folders are `parts`, in order, under a root
    /// of its own that is no folder on disk: each part is a node of some
    /// tree with everything below it, usually the root of a scan. Sizes
    /// already read come along, and a part that is a whole scanned folder
    /// shares its sizes with the tree it comes from, so sizes read later
    /// into either reach both.
    ///
    /// # Errors
    ///
    /// [`TreeError::TooLarge`] or [`TreeError::TooDeep`] past the id limits.
    ///
    /// # Panics
    ///
    /// If a part is the root of a joined tree, which no folder holds.
    pub(crate) fn join(parts: &[(&Self, NodeId)]) -> Result<Self, TreeError> {
        let n = 1 + parts
            .iter()
            .map(|&(tree, top)| tree.subtree(top).len())
            .sum::<usize>();
        // As for a scan, `NodeId::MAX` stays free so `end` can hold `n`.
        let n32 = u32::try_from(n)
            .ok()
            .filter(|&n| n < u32::MAX)
            .ok_or(TreeError::TooLarge)?;
        let bytes: usize = parts.iter().map(|&(tree, _)| tree.names.byte_len()).sum();
        let mut names = NameArena::with_capacity(n, bytes);
        let mut haystack = NameArena::with_capacity(n, bytes + n);
        names.push("")?;
        haystack.push_lowercase_terminated("")?;
        let mut parent = Vec::with_capacity(n);
        let mut end = Vec::with_capacity(n);
        let mut depth = Vec::with_capacity(n);
        let mut kind = Vec::with_capacity(n);
        let mut ext = Vec::with_capacity(n);
        parent.push(NodeId::ROOT);
        end.push(n32);
        depth.push(0);
        kind.push(Kind::Dir);
        ext.push(0);
        let mut extensions = Extensions::default();
        let mut folders = Vec::with_capacity(parts.len());

        for &(tree, top) in parts {
            let range = tree.subtree(top);
            let start = parent.len();
            // `range.start <= id` for every id in the part.
            let shift = |id: usize| to_u32(id - range.start + start);
            names.extend_from(&tree.names, range.clone())?;
            haystack.extend_from(&tree.haystack, range.clone())?;
            parent.push(NodeId::ROOT);
            parent.extend(
                tree.parent[range.start + 1..range.end]
                    .iter()
                    .map(|p| NodeId::new(shift(p.index()))),
            );
            end.extend(tree.end[range.clone()].iter().map(|&e| shift(e as usize)));
            let below = tree.depth[top.index()];
            for &d in &tree.depth[range.clone()] {
                depth.push((d - below).checked_add(1).ok_or(TreeError::TooDeep)?);
            }
            kind.extend_from_slice(&tree.kind[range.clone()]);
            let table: Vec<u16> = tree
                .extensions
                .iter()
                .map(|e| extensions.intern(e))
                .collect();
            ext.extend(tree.ext[range].iter().map(|&e| table[usize::from(e)]));
            folders.push(tree.folder_part(top, NodeId::new(to_u32(start))));
        }

        let mut files = FixedBitSet::with_capacity(n);
        for (id, k) in kind.iter().enumerate() {
            if k.is_file() {
                files.insert(id);
            }
        }
        let (child_start, child_ids) = children_by_id(&parent);
        Ok(Self {
            folders: folders.into_boxed_slice(),
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
        })
    }

    /// The folder that the subtree at `top` becomes when joined with its
    /// top at `at`: the whole folder shares its sizes; a folder inside one
    /// takes a copy of those it has.
    fn folder_part(&self, top: NodeId, at: NodeId) -> Folder {
        let folder = self
            .folder_holding(top)
            .expect("a part is a node of a scanned folder");
        if folder.top == top {
            return Folder {
                top: at,
                path: folder.path.clone(),
                sizes: Arc::clone(&folder.sizes),
            };
        }
        let sizes = OnceLock::new();
        if let Some(all) = folder.sizes.get() {
            let from = top.index() - folder.top.index();
            let len = self.subtree(top).len();
            sizes
                .set(all[from..from + len].into())
                .expect("a new cell is empty");
        }
        Folder {
            top: at,
            path: folder.path.join(self.path_from(folder.top, top)),
            sizes: Arc::new(sizes),
        }
    }
}
