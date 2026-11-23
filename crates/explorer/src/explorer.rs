//! [`Explorer`]: the one object the app talks to.
//!
//! It owns the shared tree and everything a view adds to it: the checked
//! files, the open folders, the filter's result, the sort and the rows on
//! screen. It is a plain struct, with no async and no UI types; the app
//! calls it on the UI thread and runs the slow parts (a scan, sizes, a
//! filter on a large tree) on background threads with the types this crate
//! gives it for that.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use fixedbitset::FixedBitSet;

use crate::remap::IdMap;
use crate::rows::flatten;
use crate::sort::DisplayOrder;
use crate::{Check, Kind, KindFilter, NodeId, Query, Selection, Sort, Tree, Visibility, filter};

mod filtering;
mod gallery;
mod row;
mod saved;
#[cfg(test)]
mod tests;

pub use filtering::{FilterJob, FilterResult};
pub use gallery::Gallery;
pub use row::RowView;
pub use saved::{SavedSelection, ViewSnapshot};

/// A view of one tree: checks, open folders, filter, sort and rows.
///
/// Every method taking a [`NodeId`] expects an id of the current tree and
/// panics on one past its end.
pub struct Explorer {
    tree: Arc<Tree>,
    selection: Selection,
    /// Folders the user opened. A filter leaves it alone; it returns when
    /// the filter clears.
    expanded: FixedBitSet,
    /// Folders closed by hand while a filter is active. Reset whenever the
    /// query text or the kind changes.
    collapsed: FixedBitSet,
    visibility: Visibility,
    /// The filter most recently asked for, and the one `visibility` shows.
    requested: (String, KindFilter),
    applied: (String, KindFilter),
    /// The newest filter generation. Jobs hold a handle and stop once it
    /// moves past theirs.
    generation: Arc<AtomicU64>,
    sort: Sort,
    order: DisplayOrder,
    rows: Vec<NodeId>,
    /// The checked files in display order under a sort other than the
    /// tree's own; made on first use, dropped on any change to the checks,
    /// the order or the tree.
    gallery: OnceLock<Box<[NodeId]>>,
}

impl fmt::Debug for Explorer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Explorer")
            .field("tree", &self.tree)
            .field("rows", &self.rows.len())
            .field("selected", &self.selection.count())
            .field("filter", &self.applied)
            .field("sort", &self.sort)
            .finish_non_exhaustive()
    }
}

impl Explorer {
    /// A view of `tree` with nothing checked, every folder closed, no
    /// filter, and Name ascending. The rows are the root's children.
    #[must_use]
    pub fn new(tree: Arc<Tree>) -> Self {
        let n = tree.node_count();
        let mut explorer = Self {
            selection: Selection::new(&tree),
            expanded: FixedBitSet::with_capacity(n),
            collapsed: FixedBitSet::with_capacity(n),
            visibility: Visibility::all(),
            requested: (String::new(), KindFilter::All),
            applied: (String::new(), KindFilter::All),
            generation: Arc::new(AtomicU64::new(0)),
            sort: Sort::default(),
            order: DisplayOrder::default(),
            rows: Vec::new(),
            gallery: OnceLock::new(),
            tree,
        };
        explorer.rebuild_rows();
        explorer
    }

    /// The tree on view. Clone the `Arc` to hand it to a background thread,
    /// for example for [`crate::read_sizes`].
    #[must_use]
    pub fn tree(&self) -> &Arc<Tree> {
        &self.tree
    }

    /// The nodes on screen, top to bottom. The root is never a row.
    #[must_use]
    pub fn rows(&self) -> &[NodeId] {
        &self.rows
    }

    /// The row showing `id`, if it is on screen. A linear search over the
    /// rows: fine for a click or a key press, not for every row each frame.
    #[must_use]
    pub fn row_of(&self, id: NodeId) -> Option<usize> {
        self.rows.iter().position(|&row| row == id)
    }

    /// Whether the folder shows its children: under an active filter, when
    /// something below it is visible and it was not closed by hand;
    /// otherwise when the user opened it. Files and unreadable folders are
    /// never open; the root, whose children are the top rows, always is.
    #[must_use]
    pub fn is_open(&self, id: NodeId) -> bool {
        if id == NodeId::ROOT {
            true
        } else if self.visibility.is_active() {
            self.visibility.opens(id) && !self.collapsed.contains(id.index())
        } else {
            self.tree.kind(id) == Kind::Dir && self.expanded.contains(id.index())
        }
    }

    /// Whether the folder has a chevron: a readable folder, or under an
    /// active filter a folder with something visible below it. False for
    /// the root, files, unreadable folders, and a folder the filter shows
    /// only for its name. An empty folder can open, showing nothing.
    #[must_use]
    pub fn can_open(&self, id: NodeId) -> bool {
        if id == NodeId::ROOT {
            false
        } else if self.visibility.is_active() {
            self.visibility.opens(id)
        } else {
            self.tree.kind(id) == Kind::Dir
        }
    }

    /// The chevron: opens or closes a folder. See [`Explorer::set_open`].
    pub fn toggle_folder(&mut self, id: NodeId) {
        self.set_open(id, !self.is_open(id));
    }

    /// Opens or closes a folder that [`Explorer::can_open`]; anything else
    /// is ignored. Under an active filter this changes only the filter's
    /// temporary state, which lasts until the query or kind changes; the
    /// stored open folders return when the filter clears.
    pub fn set_open(&mut self, id: NodeId, open: bool) {
        self.catch_up();
        if !self.can_open(id) || self.is_open(id) == open {
            return;
        }
        if self.visibility.is_active() {
            self.collapsed.set(id.index(), !open);
        } else {
            self.expanded.set(id.index(), open);
        }
        self.rebuild_rows();
    }

    /// Opens every folder. Under an active filter, reopens the folders
    /// closed by hand and leaves the stored open folders as they are.
    pub fn expand_all(&mut self) {
        self.catch_up();
        if self.visibility.is_active() {
            self.collapsed.clear();
        } else {
            self.expanded = self.all_folders();
        }
        self.rebuild_rows();
    }

    /// Closes every folder. Under an active filter, closes them only until
    /// the query or kind changes, and leaves the stored open folders as they
    /// are.
    pub fn collapse_all(&mut self) {
        self.catch_up();
        if self.visibility.is_active() {
            self.collapsed = self.all_folders();
        } else {
            self.expanded.clear();
        }
        self.rebuild_rows();
    }

    /// Opens every folder above `id` so it is on screen, and returns its
    /// row. `None` for the root, or for a node the active filter hides.
    pub fn reveal(&mut self, id: NodeId) -> Option<usize> {
        self.catch_up();
        if id == NodeId::ROOT || !self.visibility.is_visible(id) {
            return None;
        }
        let active = self.visibility.is_active();
        let mut changed = false;
        let mut node = id;
        while let Some(parent) = self.tree.parent(node).filter(|&p| p != NodeId::ROOT) {
            let bit = parent.index();
            if active {
                changed |= self.collapsed.contains(bit);
                self.collapsed.remove(bit);
            } else {
                changed |= !self.expanded.contains(bit);
                self.expanded.insert(bit);
            }
            node = parent;
        }
        if changed {
            self.rebuild_rows();
        }
        self.row_of(id)
    }

    /// The checkbox of `id`. A folder counts only the files the active
    /// filter shows.
    #[must_use]
    pub fn check(&self, id: NodeId) -> Check {
        self.selection.check(&self.tree, &self.visibility, id)
    }

    /// Whether the file `id` is checked.
    #[must_use]
    pub fn is_selected(&self, id: NodeId) -> bool {
        self.selection.is_selected(id)
    }

    /// The checked files.
    #[must_use]
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// A click on a checkbox: flips a file; clears a checked folder, hidden
    /// files included; otherwise checks every file of the folder that the
    /// filter shows.
    pub fn toggle_check(&mut self, id: NodeId) {
        self.catch_up();
        self.selection.toggle(&self.tree, &self.visibility, id);
        self.forget_gallery();
    }

    /// Checks every file the filter shows. Checked files stay checked.
    pub fn select_all(&mut self) {
        self.catch_up();
        self.selection.select_all(&self.tree, &self.visibility);
        self.forget_gallery();
    }

    /// Unchecks everything.
    pub fn clear_selection(&mut self) {
        self.catch_up();
        self.selection.clear();
        self.forget_gallery();
    }

    /// The current sort.
    #[must_use]
    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// Re-sorts every folder's children and rebuilds the rows.
    pub fn set_sort(&mut self, sort: Sort) {
        self.catch_up();
        if sort != self.sort {
            self.sort = sort;
            self.order = DisplayOrder::new(&self.tree, sort);
            self.forget_gallery();
            self.rebuild_rows();
        }
    }

    /// Call after [`crate::read_sizes`] filled this tree's sizes: a Size
    /// sort made before they arrived is redone with them. Every other
    /// method that changes the view checks for that too, so a forgotten
    /// call leaves the order stale only until the next change.
    pub fn sizes_loaded(&mut self) {
        self.catch_up();
    }

    /// The filter's result on the current tree.
    #[must_use]
    pub fn visibility(&self) -> &Visibility {
        &self.visibility
    }

    /// The filter text last asked for, by [`Explorer::filter_job`] or
    /// [`Explorer::set_filter`], exactly as typed. It may not be shown yet.
    #[must_use]
    pub fn filter_text(&self) -> &str {
        &self.requested.0
    }

    /// The kind last asked for, alongside [`Explorer::filter_text`].
    #[must_use]
    pub fn filter_kind(&self) -> KindFilter {
        self.requested.1
    }

    /// Whether the rows on screen are filtered: a query or a kind other
    /// than [`KindFilter::All`] has been applied.
    #[must_use]
    pub fn is_filter_active(&self) -> bool {
        self.visibility.is_active()
    }

    /// Switches to a new scan of the same folder, carrying the checks, the
    /// open folders and the folders closed under the filter across by path.
    /// The filter last asked for is run on the new tree here, synchronously,
    /// and any filter job still running for the old tree is superseded.
    /// Returns the old-to-new id map, for the app's own ids such as the file
    /// on the stage.
    pub fn replace_tree(&mut self, tree: Arc<Tree>) -> IdMap {
        let map = IdMap::new(&self.tree, &tree);
        self.install_tree(tree, &map);
        map
    }

    /// [`Explorer::replace_tree`] for a view with a file on the stage:
    /// also returns where `current` goes. It keeps its path when that is
    /// still a file. Otherwise it moves to the nearest gallery file that
    /// still exists: the first after it in the old display order, else the
    /// last before it; else `None`. `current` need not be checked.
    pub fn replace_tree_carrying(
        &mut self,
        tree: Arc<Tree>,
        current: NodeId,
    ) -> (IdMap, Option<NodeId>) {
        let map = IdMap::new(&self.tree, &tree);
        let survives = |file: &NodeId| map.get(*file).is_some();
        let carried = map.get(current).or_else(|| {
            let after =
                std::iter::successors(self.gallery_next(current), |&file| self.gallery_next(file))
                    .find(survives);
            let nearest = after.or_else(|| {
                std::iter::successors(self.gallery_previous(current), |&file| {
                    self.gallery_previous(file)
                })
                .find(survives)
            });
            nearest.and_then(|old| map.get(old))
        });
        self.install_tree(tree, &map);
        (map, carried)
    }

    /// Moves every piece of state to `tree` through `map`, and reruns the
    /// filter asked for on it. A folder whose every file was checked has
    /// every file checked in the new tree, files added to it included, as
    /// a saved state brings it back.
    fn install_tree(&mut self, tree: Arc<Tree>, map: &IdMap) {
        let n = tree.node_count();
        let (full, _) = saved::compact(&self.tree, &self.selection);
        let mut checked = map.carry(self.selection.as_bitset(), n);
        for folder in full.into_iter().filter_map(|folder| map.get(folder)) {
            if !tree.is_file(folder) {
                checked.insert_range(tree.subtree(folder));
            }
        }
        // Folder bits are dropped here, leaving the files.
        self.selection = Selection::from_bits(&tree, checked);
        self.expanded = map.carry(&self.expanded, n);
        self.collapsed = map.carry(&self.collapsed, n);
        self.generation.fetch_add(1, Ordering::Relaxed);
        let (text, kind) = &self.requested;
        let visibility = filter(&tree, &Query::parse(text), *kind);
        self.order = DisplayOrder::new(&tree, self.sort);
        self.forget_gallery();
        self.tree = tree;
        let (text, kind) = self.requested.clone();
        self.install(text, kind, visibility);
    }

    /// Shows a filter result for `text` and `kind`. A change of either
    /// reopens the folders closed under the previous filter.
    fn install(&mut self, text: String, kind: KindFilter, visibility: Visibility) {
        let applied = (text, kind);
        if applied != self.applied {
            self.collapsed.clear();
            self.applied = applied;
        }
        self.visibility = visibility;
        self.rebuild_rows();
    }

    /// Redoes a Size sort that was made before the tree had sizes, once
    /// they are in. Every method that changes the view calls this first.
    fn catch_up(&mut self) {
        if self.order.is_stale(&self.tree) {
            self.order = DisplayOrder::new(&self.tree, self.sort);
            self.forget_gallery();
            self.rebuild_rows();
        }
    }

    /// Every folder's bit.
    fn all_folders(&self) -> FixedBitSet {
        let mut folders = FixedBitSet::with_capacity(self.tree.node_count());
        folders.insert_range(..);
        folders.difference_with(self.tree.files());
        folders
    }

    /// Rebuilds the rows from the open folders, the filter and the sort.
    fn rebuild_rows(&mut self) {
        let mut rows = std::mem::take(&mut self.rows);
        let (tree, order) = (&*self.tree, &self.order);
        let (visibility, expanded, collapsed) = (&self.visibility, &self.expanded, &self.collapsed);
        if visibility.is_active() {
            flatten(
                tree,
                order,
                |id| visibility.is_visible(id),
                |id| visibility.opens(id) && !collapsed.contains(id.index()),
                &mut rows,
            );
        } else {
            flatten(
                tree,
                order,
                |_| true,
                |id| expanded.contains(id.index()),
                &mut rows,
            );
        }
        self.rows = rows;
    }
}
