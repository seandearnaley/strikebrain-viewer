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
use crate::{Kind, NodeId, Tree, TreeError};

/// Where a scanned folder goes in a new join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// Before the top-level folder at this index, or last.
    At(usize),
    /// In place of this top-level folder.
    For(NodeId),
}

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
    /// tree; one that was not counts as a single folder. Each top-level
    /// folder the new scan lists, as a folder, leaves, its checks and open
    /// folders carried into the new one by path, so no file is in the tree
    /// twice; one inside it that its scan does not list (its ignore rules
    /// hide it) stays as it is. A folder that could not be read
    /// ([`Tree::unreadable`]) lists nothing and takes nothing in.
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
        let (plan, carried) = self.plan(folder, Place::At(at));
        let (map, current, added) = self.rejoin(&plan, Some(folder), carried, current)?;
        if let Some(added) = added {
            self.expanded.insert(added.index());
            self.rebuild_rows();
        }
        Ok((map, current))
    }

    /// Puts `folder`, a new scan of the top-level folder `top`, in its
    /// place, carrying checks, open folders and `current` by path as
    /// [`Explorer::replace_tree_carrying`] does. Like
    /// [`Explorer::add_folder`] it takes in every other top-level folder it
    /// now lists: one it could not read before, or whose ignore rules hid
    /// one, may list more. `top` is one of the tree's [`Tree::folders`];
    /// for a tree that is not joined, its root, and the tree becomes a
    /// joined tree of that one folder.
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
        let (plan, carried) = self.plan(folder, Place::For(top));
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

    /// The top-level folders of a join that puts `folder` in its `place`:
    /// every top-level folder on view except the one it replaces and those
    /// it lists, which it takes in. Also gives each old node whose state
    /// goes to `folder` with its node there.
    fn plan(&self, folder: &Tree, place: Place) -> (Vec<Source>, Vec<(NodeId, NodeId)>) {
        let takes_in = folder.root_path().is_absolute() && folder.kind(NodeId::ROOT) == Kind::Dir;
        let listed = |top: NodeId| {
            let path = self.tree.abs_path(top);
            folder
                .find_abs(&path)
                .filter(|&node| takes_in && folder.kind(node).is_folder())
        };
        let tops = self.tops();
        let (mut plan, mut carried) = (Vec::new(), Vec::new());
        for (ix, &top) in tops.iter().enumerate() {
            match place {
                Place::At(at) if ix == at => plan.push(Source::Scanned),
                Place::For(replaced) if replaced == top => {
                    plan.push(Source::Scanned);
                    carried.push((top, NodeId::ROOT));
                    continue;
                }
                _ => {}
            }
            match listed(top) {
                Some(node) => carried.push((top, node)),
                None => plan.push(Source::Kept(top)),
            }
        }
        if matches!(place, Place::At(at) if at >= tops.len()) {
            plan.push(Source::Scanned);
        }
        (plan, carried)
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
