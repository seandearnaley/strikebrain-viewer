//! Saving and restoring a view by path: the open folders and the checked
//! files as paths relative to the root, which outlive the tree's ids.

use std::collections::HashSet;

use fixedbitset::FixedBitSet;

use super::Explorer;
use crate::{Kind, NodeId, Selection, Tree};

impl Explorer {
    /// The folders the user opened, as paths relative to the root with `/`
    /// separators, in id order. Folders closed for a while under a filter
    /// count as open: that state is temporary and not saved.
    pub fn open_paths(&self) -> impl Iterator<Item = String> + '_ {
        let tree = &*self.tree;
        self.expanded
            .ones()
            .map(NodeId::from_index)
            .filter(move |&id| id != NodeId::ROOT && tree.kind(id) == Kind::Dir)
            .map(move |id| tree.path(id))
    }

    /// The checked files, as paths relative to the root, in id order.
    pub fn selected_paths(&self) -> impl Iterator<Item = String> + '_ {
        self.selection.iter().map(|id| self.tree.path(id))
    }

    /// Opens exactly the folders at `paths`, as [`Explorer::open_paths`]
    /// wrote them, and closes every other. A path that names nothing in this
    /// tree, or names a file, is ignored.
    pub fn restore_open<S: AsRef<str>>(&mut self, paths: impl IntoIterator<Item = S>) {
        self.catch_up();
        let mut open = FixedBitSet::with_capacity(self.tree.node_count());
        for id in find_all(&self.tree, paths) {
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
        for id in find_all(&self.tree, paths) {
            checked.insert(id.index());
        }
        self.selection = Selection::from_bits(&self.tree, checked);
        self.forget_gallery();
    }
}

/// The nodes of `tree` at `paths`: relative, `/`-separated, as
/// [`Tree::path`] writes them; a trailing `/` is allowed. Paths that name no
/// node, and the root's `""`, are skipped. One walk down only the folders on
/// the way to some path, looking each child's path up in a hash set, so the
/// cost is the children of those folders, not a search per path.
fn find_all<S: AsRef<str>>(tree: &Tree, paths: impl IntoIterator<Item = S>) -> Vec<NodeId> {
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
    let mut folders = vec![(NodeId::ROOT, String::new())];
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
