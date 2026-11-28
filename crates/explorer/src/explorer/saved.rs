//! Saving and restoring a view by path: the open folders and the checked
//! files as paths relative to a folder, which outlive the tree's ids.
//!
//! Two forms. [`Explorer::open_paths`] and [`Explorer::selected_paths`] list
//! every path from the root, as do [`Explorer::restore_open`],
//! [`Explorer::restore_selection`] and [`Explorer::restore_saved_selection`]
//! and the snapshot's own `open_paths` and `selection`: in a joined tree
//! those paths start with a top-level folder's name, so they cannot tell
//! two folders of one name apart. The app saves and restores per folder,
//! with [`ViewSnapshot::folders`] and [`Explorer::restore_folder`]. [`Explorer::snapshot`] copies the open folders
//! and the checks out in constant time per word of bits, so a background
//! thread can name them; its [`ViewSnapshot::folders`] gives each scanned
//! folder's part, [`SavedFolder`], with its checks written compactly
//! ([`SavedSelection`]): a folder whose every file is checked is one entry,
//! so "select all" over a million files saves as one path.

use std::collections::HashSet;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use fixedbitset::FixedBitSet;

use super::Explorer;
use crate::{Check, Kind, NodeId, Selection, Tree, Visibility, bits};

/// Checked files as a saved state holds them: folders whose every file is
/// checked, then the other checked files. Each is a path relative to the
/// root with `/` separators; the root itself is `""`. Paths are in id
/// order, so the same checks always save the same way.
///
/// Restoring a folder entry checks every file in that folder at the time
/// of the restore, which includes files added to it since it was saved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedSelection {
    /// Folders whose every file was checked. A folder here has none of its
    /// files or folders listed again.
    pub folders: Vec<String>,
    /// The other checked files.
    pub files: Vec<String>,
}

impl SavedSelection {
    /// Whether nothing is checked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.files.is_empty()
    }
}

/// One scanned folder's part of a saved view, by paths relative to it with
/// `/` separators: `""` is the folder itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavedFolder {
    /// The folder, as [`Tree::abs_path`] gives it.
    pub path: PathBuf,
    /// Its open folders, itself included when it is a top-level folder of
    /// a joined tree and open.
    pub open: Vec<String>,
    /// Its checked files, compactly.
    pub selection: SavedSelection,
}

/// The open folders and the checks of an [`Explorer`], copied out so that
/// naming them, which costs a path per entry, can run off the UI thread.
/// `Send`; holds the tree.
#[derive(Clone)]
pub struct ViewSnapshot {
    tree: Arc<Tree>,
    open: FixedBitSet,
    selection: Selection,
}

impl fmt::Debug for ViewSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ViewSnapshot")
            .field("open", &self.open.count_ones(..))
            .field("selected", &self.selection.count())
            .finish_non_exhaustive()
    }
}

impl ViewSnapshot {
    /// The tree the snapshot was taken of.
    #[must_use]
    pub fn tree(&self) -> &Arc<Tree> {
        &self.tree
    }

    /// The stored open folders, as [`Explorer::open_paths`] lists them.
    #[must_use]
    pub fn open_paths(&self) -> Vec<String> {
        open_paths(&self.tree, &self.open).collect()
    }

    /// The checked files, compactly: one entry for a folder whose every
    /// file is checked, without its contents. Costs one entry per folder
    /// that is partly checked, plus the entries written.
    #[must_use]
    pub fn selection(&self) -> SavedSelection {
        let tree = &*self.tree;
        let (folders, files) = compact(tree, &self.selection);
        SavedSelection {
            folders: folders.into_iter().map(|id| tree.path(id)).collect(),
            files: files.into_iter().map(|id| tree.path(id)).collect(),
        }
    }

    /// Each scanned folder's open folders and checks, by paths relative to
    /// it, in id order: one entry per top-level folder of a joined tree,
    /// or the root's alone.
    #[must_use]
    pub fn folders(&self) -> Vec<SavedFolder> {
        self.tree.folders().map(|top| self.folder(top)).collect()
    }

    /// The open folders and checks of the scanned folder at `top`, one of
    /// [`Tree::folders`], by paths relative to it.
    #[must_use]
    pub fn folder(&self, top: NodeId) -> SavedFolder {
        let tree = &*self.tree;
        let range = tree.subtree(top);
        // Only this folder's bits: one walk per folder, not per folder and
        // open folder.
        let next = |from: usize| bits::next_one(&self.open, from).filter(|&bit| bit < range.end);
        let open = std::iter::successors(next(range.start), |&bit| next(bit + 1))
            .map(NodeId::from_index)
            .filter(|&id| id != NodeId::ROOT && tree.kind(id) == Kind::Dir)
            .map(|id| tree.path_from(top, id))
            .collect();
        let (mut folders, mut files) = (Vec::new(), Vec::new());
        compact_from(tree, &self.selection, top, &mut folders, &mut files);
        let relative = |ids: Vec<NodeId>| -> Vec<String> {
            ids.into_iter().map(|id| tree.path_from(top, id)).collect()
        };
        SavedFolder {
            path: tree.abs_path(top),
            open,
            selection: SavedSelection {
                folders: relative(folders),
                files: relative(files),
            },
        }
    }
}

/// The checks of `selection` compactly, in id order: the folders whose
/// every file is checked, none inside another (the root alone when every
/// file of a tree that is not joined is), then the other checked files.
/// Each scanned folder counts on its own, so the root of a joined tree is
/// never one. Opens only partly checked folders.
pub(super) fn compact(tree: &Tree, selection: &Selection) -> (Vec<NodeId>, Vec<NodeId>) {
    let (mut folders, mut files) = (Vec::new(), Vec::new());
    for top in tree.folders() {
        compact_from(tree, selection, top, &mut folders, &mut files);
    }
    (folders, files)
}

/// [`compact`] for the subtree of `top`, added to `folders` and `files`.
fn compact_from(
    tree: &Tree,
    selection: &Selection,
    top: NodeId,
    folders: &mut Vec<NodeId>,
    files: &mut Vec<NodeId>,
) {
    // An outline may list just one checked file in a directory with unseen
    // descendants. Only checked paths can carry across or be persisted.
    if tree.is_partial(top) {
        let range = tree.subtree(top);
        let next =
            |from| bits::next_one(selection.as_bitset(), from).filter(|&bit| bit < range.end);
        files.extend(
            std::iter::successors(next(range.start), |bit| next(bit + 1)).map(NodeId::from_index),
        );
        return;
    }
    let all = Visibility::all();
    let check = |id| selection.check(tree, &all, id);
    if tree.is_file(top) {
        if selection.is_selected(top) {
            files.push(top);
        }
        return;
    }
    match check(top) {
        Check::Off => return,
        Check::On => return folders.push(top),
        Check::Mixed => {}
    }
    // Depth first, children pushed in reverse so the first comes off
    // first: nodes leave the stack in id order.
    let mut stack: Vec<NodeId> = tree.children(top).iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if tree.is_file(id) {
            if selection.is_selected(id) {
                files.push(id);
            }
            continue;
        }
        match check(id) {
            Check::Off => {}
            Check::On => folders.push(id),
            Check::Mixed => stack.extend(tree.children(id).iter().rev()),
        }
    }
}

/// The stored open folders among `open`, in id order.
fn open_ids<'a>(tree: &'a Tree, open: &'a FixedBitSet) -> impl Iterator<Item = NodeId> + 'a {
    open.ones()
        .map(NodeId::from_index)
        .filter(move |&id| id != NodeId::ROOT && tree.kind(id) == Kind::Dir)
}

/// The stored open folders among `open`, as relative paths in id order.
fn open_paths<'a>(tree: &'a Tree, open: &'a FixedBitSet) -> impl Iterator<Item = String> + 'a {
    open_ids(tree, open).map(move |id| tree.path(id))
}

impl Explorer {
    /// The folders the user opened, as paths relative to the root with `/`
    /// separators, in id order. Folders closed for a while under a filter
    /// count as open: that state is temporary and not saved.
    pub fn open_paths(&self) -> impl Iterator<Item = String> + '_ {
        open_paths(&self.tree, &self.expanded)
    }

    /// The checked files, as paths relative to the root, in id order.
    pub fn selected_paths(&self) -> impl Iterator<Item = String> + '_ {
        self.selection.iter().map(|id| self.tree.path(id))
    }

    /// The open folders and the checks, copied out for saving: two bit sets
    /// and the tree's `Arc`, so cheap enough for the UI thread at any size.
    #[must_use]
    pub fn snapshot(&self) -> ViewSnapshot {
        ViewSnapshot {
            tree: Arc::clone(&self.tree),
            open: self.expanded.clone(),
            selection: self.selection.clone(),
        }
    }

    /// Opens exactly the folders at `paths`, as [`Explorer::open_paths`]
    /// wrote them, and closes every other. A path that names nothing in this
    /// tree, or names a file, is ignored.
    pub fn restore_open<S: AsRef<str>>(&mut self, paths: impl IntoIterator<Item = S>) {
        self.catch_up();
        let mut open = FixedBitSet::with_capacity(self.tree.node_count());
        for id in find_all(&self.tree, NodeId::ROOT, paths) {
            if self.tree.kind(id) == Kind::Dir {
                open.insert(id.index());
            }
        }
        self.expanded = open;
        self.rebuild_rows();
    }

    /// Checks exactly the files at `paths`, as [`Explorer::selected_paths`]
    /// wrote them, and unchecks every other. A path that names nothing in
    /// this tree, or names a folder, is ignored.
    pub fn restore_selection<S: AsRef<str>>(&mut self, paths: impl IntoIterator<Item = S>) {
        self.catch_up();
        let mut checked = FixedBitSet::with_capacity(self.tree.node_count());
        for id in find_all(&self.tree, NodeId::ROOT, paths) {
            checked.insert(id.index());
        }
        self.selection = Selection::from_bits(&self.tree, checked);
        self.forget_gallery();
    }

    /// Checks exactly what `saved` holds, as [`ViewSnapshot::selection`]
    /// wrote it: every file in each of its folders, and each of its files.
    /// Paths that name nothing in this tree, a file listed as a folder and
    /// a folder listed as a file are ignored.
    pub fn restore_saved_selection(&mut self, saved: &SavedSelection) {
        self.catch_up();
        let mut checked = FixedBitSet::with_capacity(self.tree.node_count());
        check_saved(&self.tree, NodeId::ROOT, saved, &mut checked);
        // Folder bits are dropped here, leaving the files.
        self.selection = Selection::from_bits(&self.tree, checked);
        self.forget_gallery();
    }

    /// Restores one scanned folder's part of a saved view, as
    /// [`ViewSnapshot::folders`] wrote it, to the folder at `saved.path`:
    /// opens exactly its open folders and checks exactly its checks, as
    /// [`Explorer::restore_open`] and [`Explorer::restore_saved_selection`]
    /// do, leaving every other folder as it is. Returns `false`, changing
    /// nothing, when no folder of the tree is at that path.
    pub fn restore_folder(&mut self, saved: &SavedFolder) -> bool {
        self.catch_up();
        let tree = Arc::clone(&self.tree);
        let Some(top) = tree.folders().find(|&top| tree.abs_path(top) == saved.path) else {
            return false;
        };
        let range = tree.subtree(top);
        self.expanded.remove_range(range.clone());
        let mut open: Vec<NodeId> = find_all(&tree, top, &saved.open);
        if saved.open.iter().any(String::is_empty) {
            open.push(top);
        }
        for id in open {
            if tree.kind(id) == Kind::Dir {
                self.expanded.insert(id.index());
            }
        }
        let mut checked = self.selection.as_bitset().clone();
        checked.remove_range(range);
        check_saved(&tree, top, &saved.selection, &mut checked);
        self.selection = Selection::from_bits(&tree, checked);
        self.forget_gallery();
        self.rebuild_rows();
        true
    }
}

/// Sets in `checked` every file `saved` names below `from`: each file, and
/// every file in each folder, `""` being `from` itself.
fn check_saved(tree: &Tree, from: NodeId, saved: &SavedSelection, checked: &mut FixedBitSet) {
    let mut folder_ids = find_all(tree, from, &saved.folders);
    if saved.folders.iter().any(String::is_empty) {
        folder_ids.push(from);
    }
    for folder in folder_ids {
        if !tree.is_file(folder) {
            checked.insert_range(tree.subtree(folder));
        }
    }
    for file in find_all(tree, from, &saved.files) {
        checked.insert(file.index());
    }
}

/// The nodes below `from` at `paths`: relative to it, `/`-separated, as
/// [`Tree::path_from`] writes them; a trailing `/` is allowed. Paths that
/// name no node, and `from`'s own `""`, are skipped. One walk down only the
/// folders on the way to some path, looking each child's path up in a hash
/// set, so the cost is the children of those folders, not a search per
/// path.
fn find_all<S: AsRef<str>>(
    tree: &Tree,
    from: NodeId,
    paths: impl IntoIterator<Item = S>,
) -> Vec<NodeId> {
    let paths: Vec<S> = paths.into_iter().collect();
    let wanted: HashSet<&str> = paths
        .iter()
        .map(|path| path.as_ref().trim_end_matches('/'))
        .filter(|path| !path.is_empty())
        .collect();
    let mut on_the_way: HashSet<&str> = HashSet::new();
    for path in &wanted {
        for (slash, _) in path.match_indices('/') {
            on_the_way.extend(path.get(..slash));
        }
    }
    let mut found = Vec::new();
    let mut folders = vec![(from, String::new())];
    let mut path = String::new();
    while let Some((folder, folder_path)) = folders.pop() {
        for &child in tree.children(folder) {
            path.clear();
            path.push_str(&folder_path);
            if !folder_path.is_empty() {
                path.push('/');
            }
            path.push_str(tree.name(child));
            if wanted.contains(path.as_str()) {
                found.push(child);
            }
            if !tree.children(child).is_empty() && on_the_way.contains(path.as_str()) {
                folders.push((child, path.clone()));
            }
        }
    }
    found
}
