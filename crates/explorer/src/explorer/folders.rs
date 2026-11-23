//! Several folders: adding, rescanning and removing one top-level folder.
//!
//! Each change joins a new tree from the folders that stay, as they are,
//! and the folder scanned (see [`Tree::join`]), then moves the view across:
//! the folders that stay move whole, so their ids shift and nothing is
//! compared; a rescanned folder, and a folder an added one takes in, merge
//! with the new scan by name, as a refresh does. Checks, open folders and
//! the file on the stage carry across by path.

use std::path::Path;
use std::sync::Arc;

use super::Explorer;
use crate::remap::IdMap;
use crate::{NodeId, Tree, TreeError};

/// Where a top-level folder of a new join comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// A top-level folder of the tree on view, as it is.
    Kept(NodeId),
    /// The folder just scanned.
    Scanned,
}

impl Explorer {
    /// A view of no folders: no rows, nothing to check. Folders come in
    /// with [`Explorer::add_folder`].
    #[must_use]
    pub fn empty() -> Self {
        Self::new(Arc::new(no_folders()))
    }

    /// Adds `folder`, a scan, as a top-level folder, open, before the
    /// top-level folder at index `at` (or last). The tree becomes a joined
    /// tree; one that was not counts as a single folder. Top-level folders
    /// inside the new one leave, their checks and open folders carried into
    /// it by path, so no file is in the tree twice. The caller makes sure
    /// the new folder is not inside one already on view.
    ///
    /// Returns the old-to-new id map, and where `current`, the file on the
    /// stage, goes: as [`Explorer::replace_tree_carrying`] says.
    ///
    /// # Errors
    ///
    /// [`TreeError`] when the joined tree would pass the id limits; the
    /// view is left as it was.
    pub fn add_folder(
        &mut self,
        folder: &Tree,
        at: usize,
        current: Option<NodeId>,
    ) -> Result<(IdMap, Option<NodeId>), TreeError> {
        let path = folder.root_path();
        let mut plan = Vec::new();
        let mut carried = Vec::new();
        let tops = self.tops();
        for (ix, &top) in tops.iter().enumerate() {
            if ix == at {
                plan.push(Source::Scanned);
            }
            let top_path = self.tree.abs_path(top);
            if path.is_absolute() && top_path.starts_with(path) {
                // Taken in: its state carries over if the scan lists it.
                carried.extend(folder.find_abs(&top_path).map(|node| (top, node)));
            } else {
                plan.push(Source::Kept(top));
            }
        }
        if at >= tops.len() {
            plan.push(Source::Scanned);
        }
        let (map, current, added) = self.rejoin(&plan, Some(folder), carried, current)?;
        if let Some(added) = added {
            self.expanded.insert(added.index());
            self.rebuild_rows();
        }
        Ok((map, current))
    }

    /// Puts `folder`, a new scan of the top-level folder `top`, in its
    /// place, carrying checks, open folders and `current` by path as
    /// [`Explorer::replace_tree_carrying`] does. `top` is one of the tree's
    /// [`Tree::folders`]; for a tree that is not joined, its root, and the
    /// tree becomes a joined tree of that one folder.
    ///
    /// # Errors
    ///
    /// [`TreeError`] when the joined tree would pass the id limits; the
    /// view is left as it was.
    pub fn replace_folder(
        &mut self,
        top: NodeId,
        folder: &Tree,
        current: Option<NodeId>,
    ) -> Result<(IdMap, Option<NodeId>), TreeError> {
        let plan: Vec<Source> = self
            .tops()
            .into_iter()
            .map(|each| {
                if each == top {
                    Source::Scanned
                } else {
                    Source::Kept(each)
                }
            })
            .collect();
        let carried = vec![(top, NodeId::ROOT)];
        let (map, current, _) = self.rejoin(&plan, Some(folder), carried, current)?;
        Ok((map, current))
    }

    /// Removes the top-level folder `top` with everything in it, its checks
    /// included. `current` moves to the nearest checked file left, as
    /// [`Explorer::replace_tree_carrying`] says, if it was in the folder.
    pub fn remove_folder(
        &mut self,
        top: NodeId,
        current: Option<NodeId>,
    ) -> (IdMap, Option<NodeId>) {
        let plan: Vec<Source> = self
            .tops()
            .into_iter()
            .filter(|&each| each != top)
            .map(Source::Kept)
            .collect();
        self.keep_only(&plan, current)
    }

    /// [`Explorer::rejoin`] for a plan that only keeps folders: the tree
    /// can only shrink, so it always fits.
    fn keep_only(&mut self, plan: &[Source], current: Option<NodeId>) -> (IdMap, Option<NodeId>) {
        let (map, current, _) = self
            .rejoin(plan, None, Vec::new(), current)
            .expect("fewer nodes than the tree on view fit");
        (map, current)
    }

    /// The top-level folders a change keeps or replaces: those of a joined
    /// tree, or the root of one that is not.
    fn tops(&self) -> Vec<NodeId> {
        if self.tree.is_joined() {
            self.tree.folders().collect()
        } else {
            vec![NodeId::ROOT]
        }
    }

    /// Joins the folders of `plan` into a new tree and moves the view to
    /// it. `carried` pairs old nodes with their nodes in `scanned`, whose
    /// state they hand on. Returns the id map, where `current` goes, and
    /// the scanned folder's new top.
    fn rejoin(
        &mut self,
        plan: &[Source],
        scanned: Option<&Tree>,
        carried: Vec<(NodeId, NodeId)>,
        current: Option<NodeId>,
    ) -> Result<(IdMap, Option<NodeId>, Option<NodeId>), TreeError> {
        let parts: Vec<(&Tree, NodeId)> = plan
            .iter()
            .filter_map(|source| match *source {
                Source::Kept(top) => Some((&*self.tree, top)),
                Source::Scanned => scanned.map(|tree| (tree, NodeId::ROOT)),
            })
            .collect();
        let tree = Tree::join(&parts)?;
        let mut kept = Vec::new();
        let mut added = None;
        for (source, &top) in plan.iter().zip(tree.children(NodeId::ROOT)) {
            match *source {
                Source::Kept(old) => kept.push((old, top)),
                Source::Scanned => added = Some(top),
            }
        }
        let merged = added.map_or_else(Vec::new, |added| {
            carried
                .into_iter()
                .map(|(old, node)| (old, NodeId::from_index(added.index() + node.index())))
                .collect()
        });
        let map = IdMap::rejoined(&self.tree, &tree, &kept, merged);
        let current = current.and_then(|current| self.carry_current(&map, current));
        self.install_tree(Arc::new(tree), &map);
        Ok((map, current, added))
    }

    /// The top-level folder at `path`, exactly: one of [`Tree::folders`].
    #[must_use]
    pub fn folder_at(&self, path: &Path) -> Option<NodeId> {
        self.tree
            .folders()
            .find(|&top| self.tree.abs_path(top) == path)
    }
}

/// The tree of no folders: a root alone.
fn no_folders() -> Tree {
    Tree::join(&[]).expect("a tree of just a root fits")
}
