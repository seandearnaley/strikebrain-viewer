//! The scanned tree: parallel arrays indexed by [`NodeId`] in depth-first
//! pre-order.
//!
//! Within a folder, ids follow the canonical order: folders first, then names
//! in natural, case-insensitive order ([`compare_names`]). So a folder's
//! subtree is one contiguous id range, and a folder's state over its subtree
//! is a ranged bit count rather than a walk.
//!
//! A scan makes a tree of one folder, its root. A joined tree (`Tree::join`)
//! holds several scanned folders under a root of its own:
//! each is a top-level folder, in the order given, laid out one after
//! another, so ids stay pre-order and every subtree stays one range.

use std::cmp::Ordering;
use std::fmt;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, OnceLock};

use fixedbitset::FixedBitSet;
use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::preferences::CollationNumericOrdering;
use icu_collator::{CollatorBorrowed, CollatorPreferences};

use crate::NodeId;
use crate::bits::Ranks;

mod build;
mod join;
mod names;
mod paths;
#[cfg(test)]
pub(crate) mod tests;

pub(crate) use build::{NO_PARENT, RawTree};
use names::NameArena;
pub(crate) use names::{extension_of, push_lowercase};

/// What a node is on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// A folder whose entries were read.
    Dir,
    /// A regular file, or any other entry that is neither a folder nor a link.
    File,
    /// A symbolic link. It is never followed and is always a leaf.
    Symlink,
    /// A folder that could not be read. It is always a leaf.
    Unreadable,
}

impl Kind {
    /// Whether the node is a folder: [`Kind::Dir`] or [`Kind::Unreadable`].
    /// Folders sort before files.
    #[must_use]
    pub const fn is_folder(self) -> bool {
        matches!(self, Self::Dir | Self::Unreadable)
    }

    /// Whether the node is a file for selection: [`Kind::File`] or
    /// [`Kind::Symlink`].
    #[must_use]
    pub const fn is_file(self) -> bool {
        !self.is_folder()
    }
}

/// Why a tree could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    /// A path given to [`Tree::from_paths`] is empty, absolute, has an empty,
    /// `.` or `..` component, or contains a NUL byte.
    InvalidPath(String),
    /// A path given to [`Tree::from_paths`] names a file twice, or names both
    /// a file and a folder.
    Conflict(String),
    /// The tree has more nodes, or more name bytes, than a `u32` can index.
    TooLarge,
    /// The tree is deeper than a `u16` can count.
    TooDeep,
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(path) => write!(f, "invalid path {path:?}"),
            Self::Conflict(path) => {
                write!(f, "path {path:?} is given twice or as both file and folder")
            }
            Self::TooLarge => {
                f.write_str("the tree has more than 4,294,967,295 nodes or name bytes")
            }
            Self::TooDeep => f.write_str("the tree is more than 65,535 levels deep"),
        }
    }
}

impl std::error::Error for TreeError {}

/// One scanned folder. Immutable once its sizes are known; share it as
/// `Arc<Tree>`.
///
/// A scan lists names and kinds first, so a large folder reaches the screen
/// before every file has been measured. [`crate::read_sizes`] then fills the
/// sizes, exactly once, on whatever thread runs it; until then
/// [`Tree::size`] is `None`. [`Tree::children`] is always canonical order;
/// an [`crate::Explorer`] keeps any other display order beside the tree.
///
/// Every accessor taking a [`NodeId`] panics if the id is not a node of this
/// tree.
///
/// A tree is not `Clone`: a large one holds hundreds of megabytes, so it is
/// shared, never copied.
///
/// ```compile_fail
/// fn cloneable<T: Clone>() {}
/// cloneable::<explorer::Tree>();
/// ```
pub struct Tree {
    /// The scanned folders the tree holds, in id order: the root alone for
    /// a scan or a tree from paths, each top-level folder for a joined tree.
    folders: Box<[Folder]>,
    names: NameArena,
    /// Lowercased names, each followed by `'\0'`, for the filter.
    haystack: NameArena,
    /// The root's entry is the root itself.
    parent: Vec<NodeId>,
    /// The subtree of `id` is `id..end[id]`.
    end: Vec<u32>,
    depth: Vec<u16>,
    kind: Vec<Kind>,
    /// Index into `extensions`; 0 is "no extension".
    ext: Vec<u16>,
    extensions: Vec<Box<str>>,
    /// Set where the node is a file or a symlink.
    files: FixedBitSet,
    /// Running counts of `files`, so a folder's file count costs no pass.
    file_ranks: Ranks,
    /// Children of `id` are `child_ids[child_start[id]..child_start[id + 1]]`.
    child_start: Vec<u32>,
    /// Every folder's children, in canonical order.
    child_ids: Vec<NodeId>,
}

/// One scanned folder inside a [`Tree`]: its top node, where it is on disk,
/// and its sizes once they are read.
#[derive(Clone, Debug)]
struct Folder {
    /// The folder's node; the folder is this node's subtree.
    top: NodeId,
    /// Only directories opened so far have been listed.
    partial: bool,
    /// The top node's absolute path; for a tree from paths, its path
    /// relative to that tree's root, `""` for the root.
    path: PathBuf,
    /// A file's bytes, a folder's the sum over its files, by `id - top`.
    /// Set once: at build for [`Tree::from_paths_with_sizes`], or by
    /// [`crate::read_sizes`]. Shared by every tree joined from the same
    /// scan, so sizes read into one reach them all.
    sizes: Arc<OnceLock<Box<[u64]>>>,
}

impl fmt::Debug for Tree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tree")
            .field("root_path", &self.root_path())
            .field("folders", &self.folders.len())
            .field("nodes", &self.node_count())
            .field("files", &self.files.count_ones(..))
            .field("sizes", &self.has_sizes())
            .finish_non_exhaustive()
    }
}

impl Tree {
    /// Builds a tree from relative paths, with no disk access. Folders named
    /// on the way to a path are created; a trailing `/` makes a folder, which
    /// may be empty. Every file has size 0.
    ///
    /// ```
    /// # use explorer::{NodeId, Tree};
    /// let tree = Tree::from_paths(&["src/main.rs", "docs/", "README.md"]).unwrap();
    /// assert_eq!(tree.node_count(), 5);
    /// assert_eq!(tree.path(NodeId::new(1)), "docs");
    /// ```
    ///
    /// # Errors
    ///
    /// [`TreeError::InvalidPath`] or [`TreeError::Conflict`] for a bad path,
    /// [`TreeError::TooLarge`] or [`TreeError::TooDeep`] past the id limits.
    pub fn from_paths(paths: &[&str]) -> Result<Self, TreeError> {
        paths::build(Path::new(""), paths.iter().map(|&path| (path, 0)))
    }

    /// Like [`Tree::from_paths`], with a byte size for each file. Sizes given
    /// for folders are ignored; a folder's size is the sum of its files.
    ///
    /// # Errors
    ///
    /// As [`Tree::from_paths`].
    pub fn from_paths_with_sizes(paths: &[(&str, u64)]) -> Result<Self, TreeError> {
        paths::build(Path::new(""), paths.iter().copied())
    }

    /// Like [`Tree::from_paths_with_sizes`], as if scanned from the folder at
    /// `root`: the root is named after it and [`Tree::abs_path`] starts
    /// there. Nothing touches the disk; for tests of several folders.
    #[cfg(test)]
    pub(crate) fn from_paths_at(root: &Path, paths: &[(&str, u64)]) -> Result<Self, TreeError> {
        paths::build(root, paths.iter().copied())
    }

    /// A folder that could not be read, as a tree of one node: an
    /// [`Kind::Unreadable`] root named after the folder's last part, at
    /// `path`, of size 0. Joined with the folders that could be read, it
    /// keeps the folder's place in the tree.
    #[must_use]
    pub fn unreadable(path: &Path) -> Self {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        one_node(&name, Kind::Unreadable, 0, path)
    }

    /// A file on its own, as a tree of one node: a [`Kind::File`] root
    /// named after the file, at `path`, with its size. Joined with folders,
    /// it is a top-level file: a file opened without its folder.
    #[must_use]
    pub fn file(path: &Path, size: u64) -> Self {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        one_node(&name, Kind::File, size, path)
    }

    /// Marks a single-folder tree as a partial directory listing.
    pub(crate) fn mark_partial(&mut self) {
        self.folders[0].partial = true;
    }

    /// Whether the folder containing `id` is still being indexed.
    #[must_use]
    pub fn is_partial(&self, id: NodeId) -> bool {
        self.folder_holding(id).is_some_and(|folder| folder.partial)
    }

    /// Absolute path of the root folder. Empty for a tree built from paths,
    /// and for a joined tree, whose root is no folder on disk.
    #[must_use]
    pub fn root_path(&self) -> &Path {
        match self.folders.first() {
            Some(folder) if folder.top == NodeId::ROOT => &folder.path,
            _ => Path::new(""),
        }
    }

    /// Whether the tree joins scanned folders under a root of its own, as
    /// [`crate::Explorer::add_folder`] makes it: its top-level folders keep
    /// the order they were joined in, and its root is no folder on disk.
    #[must_use]
    pub fn is_joined(&self) -> bool {
        self.folders
            .first()
            .is_none_or(|folder| folder.top != NodeId::ROOT)
    }

    /// The top node of each scanned folder the tree holds, in id order: the
    /// root for a scan or a tree from paths; each top-level folder for a
    /// joined tree.
    pub fn folders(&self) -> impl ExactSizeIterator<Item = NodeId> + '_ {
        self.folders.iter().map(|folder| folder.top)
    }

    /// The top node of the scanned folder holding `id`. `None` for the root
    /// of a joined tree, which no folder holds.
    #[must_use]
    pub fn folder_of(&self, id: NodeId) -> Option<NodeId> {
        self.folder_holding(id).map(|folder| folder.top)
    }

    /// The scanned folder whose subtree holds `id`.
    fn folder_holding(&self, id: NodeId) -> Option<&Folder> {
        let after = self.folders.partition_point(|folder| folder.top <= id);
        let folder = self.folders.get(after.checked_sub(1)?)?;
        (id.index() < self.end[folder.top.index()] as usize).then_some(folder)
    }

    /// Number of nodes, the root included. Never zero.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.kind.len()
    }

    /// Every id, in canonical order.
    pub fn ids(&self) -> impl DoubleEndedIterator<Item = NodeId> + ExactSizeIterator + use<> {
        (0..to_u32(self.node_count())).map(NodeId::new)
    }

    /// The node's name. The root's is the folder's own name, or `""` for a
    /// tree built from paths.
    #[must_use]
    pub fn name(&self, id: NodeId) -> &str {
        self.names.get(id.index())
    }

    /// The node's name in lowercase, as the filter matches it.
    #[must_use]
    pub fn lower_name(&self, id: NodeId) -> &str {
        let entry = self.haystack.get(id.index());
        entry.strip_suffix('\0').unwrap_or(entry)
    }

    /// The bytes the names hold and the bytes reserved for them.
    #[cfg(test)]
    pub(crate) fn name_bytes(&self) -> (usize, usize) {
        (self.names.byte_len(), self.names.capacity())
    }

    /// Every lowercased name in id order, each followed by `'\0'`.
    pub(crate) fn haystack(&self) -> &str {
        self.haystack.as_str()
    }

    /// Where each haystack entry ends: entry `i` is `ends[i - 1]..ends[i]`
    /// (or `0..ends[0]`), its `'\0'` included.
    pub(crate) fn haystack_ends(&self) -> &[u32] {
        self.haystack.ends()
    }

    /// The parent folder, or `None` for the root.
    #[must_use]
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        (id != NodeId::ROOT).then(|| self.parent[id.index()])
    }

    /// Levels below the root; the root is 0.
    #[must_use]
    pub fn depth(&self, id: NodeId) -> u16 {
        self.depth[id.index()]
    }

    /// What the node is on disk.
    #[must_use]
    pub fn kind(&self, id: NodeId) -> Kind {
        self.kind[id.index()]
    }

    /// Whether the node is a file or a symlink.
    #[must_use]
    pub fn is_file(&self, id: NodeId) -> bool {
        self.files.contains(id.index())
    }

    /// A file's size in bytes; a folder's is the sum over its files. `None`
    /// until the sizes are known: a scanned folder has none until
    /// [`crate::read_sizes`] runs. The root of a joined tree has a size once
    /// every folder has.
    #[must_use]
    pub fn size(&self, id: NodeId) -> Option<u64> {
        match self.folder_holding(id) {
            Some(folder) => folder
                .sizes
                .get()
                .map(|sizes| sizes[id.index() - folder.top.index()]),
            None => self.folders.iter().try_fold(0_u64, |sum, folder| {
                Some(sum.saturating_add(*folder.sizes.get()?.first()?))
            }),
        }
    }

    /// Whether [`Tree::size`] has values for every node.
    #[must_use]
    pub fn has_sizes(&self) -> bool {
        self.sized_folders() == self.folders.len()
    }

    /// How many of the tree's folders have their sizes. It only grows.
    pub(crate) fn sized_folders(&self) -> usize {
        self.folders
            .iter()
            .filter(|folder| folder.sizes.get().is_some())
            .count()
    }

    /// The id ranges of the folders that have no sizes yet.
    pub(crate) fn unsized_ranges(&self) -> Vec<Range<usize>> {
        self.folders
            .iter()
            .filter(|folder| folder.sizes.get().is_none())
            .map(|folder| self.subtree(folder.top))
            .collect()
    }

    /// Every node's size in id order, 0 where not yet known. A caller that
    /// needs many sizes takes this one copy, so it never mixes sizes read
    /// at different times.
    pub(crate) fn size_list(&self) -> Vec<u64> {
        let mut list = vec![0; self.node_count()];
        for folder in &self.folders {
            if let Some(sizes) = folder.sizes.get() {
                let start = folder.top.index();
                list[start..start + sizes.len()].copy_from_slice(sizes);
            }
        }
        list
    }

    /// Stores sizes given for files (folders' entries are ignored) in every
    /// folder that has none yet, and sums each folder. Returns `false`,
    /// changing nothing, if every folder already had them.
    ///
    /// # Panics
    ///
    /// If `sizes` does not hold one entry per node.
    pub(crate) fn set_sizes(&self, sizes: &[u64]) -> bool {
        assert_eq!(sizes.len(), self.node_count(), "one size per node");
        let mut stored = false;
        for folder in &self.folders {
            if folder.sizes.get().is_some() {
                continue;
            }
            let range = self.subtree(folder.top);
            let mut local = sizes[range.clone()].to_vec();
            for (at, size) in range.clone().zip(local.iter_mut()) {
                if !self.files.contains(at) {
                    *size = 0;
                }
            }
            // Children have larger ids than their parent, so one backwards
            // pass completes every subtree before it reaches the parent.
            for id in (range.start + 1..range.end).rev() {
                let (parent, child) = (self.parent[id].index() - range.start, id - range.start);
                local[parent] = local[parent].saturating_add(local[child]);
            }
            stored |= folder.sizes.set(local.into_boxed_slice()).is_ok();
        }
        stored
    }

    /// The node's index into [`Tree::extensions`]. 0 means no extension. A
    /// folder is always 0. Past 65,535 distinct extensions, later ones also
    /// get 0; [`Tree::extension`] still reads them correctly.
    pub(crate) fn ext_id(&self, id: NodeId) -> u16 {
        self.ext[id.index()]
    }

    /// The interned lowercase extensions; entry 0 is `""`.
    pub(crate) fn extensions(&self) -> &[Box<str>] {
        &self.extensions
    }

    /// A file's lowercase extension without the dot, or `""`. Folders have
    /// none.
    #[must_use]
    pub fn extension(&self, id: NodeId) -> &str {
        if self.kind(id).is_folder() {
            ""
        } else {
            extension_of(self.lower_name(id))
        }
    }

    /// The folder's children in canonical order; empty for a file.
    #[must_use]
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.child_ids[self.child_range(id)]
    }

    /// Every folder's children, folder by folder in id order, each folder's
    /// in canonical order.
    pub(crate) fn child_ids(&self) -> &[NodeId] {
        &self.child_ids
    }

    /// Where the node's children sit in [`Tree::child_ids`].
    pub(crate) fn child_range(&self, id: NodeId) -> Range<usize> {
        let i = id.index();
        self.child_start[i] as usize..self.child_start[i + 1] as usize
    }

    /// The ids of the node and all its descendants, as array indices.
    #[must_use]
    pub fn subtree(&self, id: NodeId) -> Range<usize> {
        id.index()..self.end[id.index()] as usize
    }

    /// One bit per node, set for files and symlinks.
    #[must_use]
    pub fn files(&self) -> &FixedBitSet {
        &self.files
    }

    /// Files and symlinks among the ids in `range`, in constant time.
    pub(crate) fn file_count(&self, range: Range<usize>) -> usize {
        let words = self.files.as_slice();
        self.file_ranks.count(|w| words[w], range)
    }

    /// The node's path relative to the root, with `/` separators. The root's
    /// is `""`. In a joined tree it starts with the top-level folder's name, so two
    /// folders of one name give the same paths; [`Tree::path_from`] and
    /// [`Tree::abs_path`] tell them apart.
    #[must_use]
    pub fn path(&self, id: NodeId) -> String {
        self.path_from(NodeId::ROOT, id)
    }

    /// The path of `id` relative to `from`, one of its ancestors or itself,
    /// with `/` separators, as [`Tree::find_from`] reads it. `""` when they
    /// are the same node.
    ///
    /// # Panics
    ///
    /// If `from` is not `id` or above it.
    #[must_use]
    pub fn path_from(&self, from: NodeId, id: NodeId) -> String {
        let chain = self.ancestry(from, id);
        let mut path = String::new();
        for (i, &node) in chain.iter().rev().enumerate() {
            if i > 0 {
                path.push('/');
            }
            path.push_str(self.name(node));
        }
        path
    }

    /// The node's path on disk: its folder's path joined with each name
    /// below the folder. For a tree from paths, which is not on disk, the
    /// same relative path as [`Tree::path`]; for the root of a joined
    /// tree, an empty path.
    #[must_use]
    pub fn abs_path(&self, id: NodeId) -> PathBuf {
        let Some(folder) = self.folder_holding(id) else {
            return PathBuf::new();
        };
        let mut path = folder.path.clone();
        for &node in self.ancestry(folder.top, id).iter().rev() {
            path.push(self.name(node));
        }
        path
    }

    /// The node at a relative path with `/` separators, as [`Tree::path`]
    /// writes it. `""` is the root. Each step scans the siblings, so it costs
    /// the number of siblings along the path. In a joined tree with two
    /// folders of one name it finds in the first; [`Tree::find_abs`] tells
    /// them apart.
    #[must_use]
    pub fn find(&self, path: &str) -> Option<NodeId> {
        self.find_from(NodeId::ROOT, path)
    }

    /// The node at `path` below `from`, as [`Tree::path_from`] writes it.
    /// `""` is `from` itself.
    #[must_use]
    pub fn find_from(&self, from: NodeId, path: &str) -> Option<NodeId> {
        let mut node = from;
        for part in path.split('/').filter(|part| !part.is_empty()) {
            node = self.child_named(node, part)?;
        }
        Some(node)
    }

    /// The node at `path` as [`Tree::abs_path`] writes it: inside the
    /// folder whose path holds it. `None` for a path in no folder of the
    /// tree, or one the tree does not list.
    #[must_use]
    pub fn find_abs(&self, path: &Path) -> Option<NodeId> {
        self.folders.iter().find_map(|folder| {
            let below = path.strip_prefix(&folder.path).ok()?;
            let mut node = folder.top;
            for part in below.components() {
                let Component::Normal(name) = part else {
                    return None;
                };
                node = self.child_named(node, name.to_str()?)?;
            }
            Some(node)
        })
    }

    /// The node and its ancestors up to, but not including, `from`.
    fn ancestry(&self, from: NodeId, id: NodeId) -> Vec<NodeId> {
        let mut chain = Vec::with_capacity(usize::from(self.depth(id)));
        let mut node = id;
        while node != from {
            chain.push(node);
            node = self
                .parent(node)
                .expect("`from` is the node or one of its ancestors");
        }
        chain
    }

    /// The child of `folder` named exactly `name`, walking siblings in id
    /// order.
    fn child_named(&self, folder: NodeId, name: &str) -> Option<NodeId> {
        let stop = self.end[folder.index()];
        let mut child = folder.index() + 1;
        while child < stop as usize {
            let id = NodeId::new(to_u32(child));
            if self.name(id) == name {
                return Some(id);
            }
            child = self.end[child] as usize;
        }
        None
    }
}

/// Natural, case-insensitive name order, as brainstrike's
/// `Intl.Collator(undefined, { numeric: true, sensitivity: "base" })`:
/// `file2` before `file10`, `apple` before `Banana`. Names the collator
/// finds equal, such as `a` and `A`, fall back to byte order, so distinct
/// names never compare equal.
#[must_use]
pub fn compare_names(a: &str, b: &str) -> Ordering {
    COLLATOR.compare(a, b).then_with(|| a.cmp(b))
}

/// The canonical order of two siblings: folders first, then [`compare_names`].
/// Tree ids follow it; a Name sort ascending reproduces it.
pub(crate) fn canonical_order(a_kind: Kind, a_name: &str, b_kind: Kind, b_name: &str) -> Ordering {
    b_kind
        .is_folder()
        .cmp(&a_kind.is_folder())
        .then_with(|| compare_names(a_name, b_name))
}

/// The root-locale collator at primary strength with numeric ordering.
static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    let mut prefs = CollatorPreferences::default();
    prefs.numeric_ordering = Some(CollationNumericOrdering::True);
    let mut options = CollatorOptions::default();
    options.strength = Some(Strength::Primary);
    CollatorBorrowed::try_new(prefs, options)
        .expect("the root collation data is compiled into icu_collator")
});

/// A tree of one node, its root, with `size`.
fn one_node(name: &str, kind: Kind, size: u64, path: &Path) -> Tree {
    let mut raw = RawTree::new(name, true).expect("one node fits in a tree");
    raw.set_kind(0, kind);
    raw.set_size(0, size);
    raw.build(path.to_path_buf())
        .expect("one node fits in a tree")
}

/// Converts an index the builder has already bounded to `u32`.
fn to_u32(i: usize) -> u32 {
    u32::try_from(i).expect("the builder rejects trees past u32 ids")
}
