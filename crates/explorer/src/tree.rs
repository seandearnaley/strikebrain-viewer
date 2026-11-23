//! The scanned tree: parallel arrays indexed by [`NodeId`] in depth-first
//! pre-order.
//!
//! Within a folder, ids follow the canonical order: folders first, then names
//! in natural, case-insensitive order ([`compare_names`]). So a folder's
//! subtree is one contiguous id range, and a folder's state over its subtree
//! is a ranged bit count rather than a walk.

use std::cmp::Ordering;
use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock};

use fixedbitset::FixedBitSet;
use icu_collator::options::{CollatorOptions, Strength};
use icu_collator::preferences::CollationNumericOrdering;
use icu_collator::{CollatorBorrowed, CollatorPreferences};

use crate::NodeId;

mod build;
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
    /// Absolute path of the root folder; empty for [`Tree::from_paths`].
    root_path: PathBuf,
    names: NameArena,
    /// Lowercased names, each followed by `'\0'`, for the filter.
    haystack: NameArena,
    /// The root's entry is the root itself.
    parent: Vec<NodeId>,
    /// The subtree of `id` is `id..end[id]`.
    end: Vec<u32>,
    depth: Vec<u16>,
    kind: Vec<Kind>,
    /// A file's bytes; a folder's is the sum over the files in its subtree.
    /// Set once: at build for [`Tree::from_paths_with_sizes`], or later by
    /// [`crate::read_sizes`] for a scanned tree.
    sizes: OnceLock<Box<[u64]>>,
    /// Index into `extensions`; 0 is "no extension".
    ext: Vec<u16>,
    extensions: Vec<Box<str>>,
    /// Set where the node is a file or a symlink.
    files: FixedBitSet,
    /// Children of `id` are `child_ids[child_start[id]..child_start[id + 1]]`.
    child_start: Vec<u32>,
    /// Every folder's children, in canonical order.
    child_ids: Vec<NodeId>,
}

impl fmt::Debug for Tree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tree")
            .field("root_path", &self.root_path)
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
        paths::build(paths.iter().map(|&path| (path, 0)))
    }

    /// Like [`Tree::from_paths`], with a byte size for each file. Sizes given
    /// for folders are ignored; a folder's size is the sum of its files.
    ///
    /// # Errors
    ///
    /// As [`Tree::from_paths`].
    pub fn from_paths_with_sizes(paths: &[(&str, u64)]) -> Result<Self, TreeError> {
        paths::build(paths.iter().copied())
    }

    /// Absolute path of the root folder. Empty for a tree built from paths.
    #[must_use]
    pub fn root_path(&self) -> &Path {
        &self.root_path
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
    /// until the sizes are known: a scanned tree has none until
    /// [`crate::read_sizes`] runs.
    #[must_use]
    pub fn size(&self, id: NodeId) -> Option<u64> {
        self.sizes().map(|sizes| sizes[id.index()])
    }

    /// Whether [`Tree::size`] has values.
    #[must_use]
    pub fn has_sizes(&self) -> bool {
        self.sizes.get().is_some()
    }

    /// Every node's size in id order, once known. Sizes are written once,
    /// so a caller that needs many takes this one snapshot.
    pub(crate) fn sizes(&self) -> Option<&[u64]> {
        self.sizes.get().map(|sizes| &sizes[..])
    }

    /// Stores sizes given for files (folders' entries are ignored) and sums
    /// each folder. Returns `false`, changing nothing, if sizes were already
    /// set.
    ///
    /// # Panics
    ///
    /// If `sizes` does not hold one entry per node.
    pub(crate) fn set_sizes(&self, mut sizes: Vec<u64>) -> bool {
        assert_eq!(sizes.len(), self.node_count(), "one size per node");
        if self.has_sizes() {
            return false;
        }
        for (id, size) in sizes.iter_mut().enumerate() {
            if !self.files.contains(id) {
                *size = 0;
            }
        }
        // Children have larger ids than their parent, so one backwards pass
        // completes every subtree before it reaches the parent.
        for id in (1..sizes.len()).rev() {
            let parent = self.parent[id].index();
            sizes[parent] = sizes[parent].saturating_add(sizes[id]);
        }
        self.sizes.set(sizes.into_boxed_slice()).is_ok()
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

    /// The node's path relative to the root, with `/` separators. The root's
    /// is `""`.
    #[must_use]
    pub fn path(&self, id: NodeId) -> String {
        let chain = self.ancestry(id);
        let mut path = String::new();
        for (i, &node) in chain.iter().rev().enumerate() {
            if i > 0 {
                path.push('/');
            }
            path.push_str(self.name(node));
        }
        path
    }

    /// The node's absolute path: [`Tree::root_path`] joined with each name.
    #[must_use]
    pub fn abs_path(&self, id: NodeId) -> PathBuf {
        let mut path = self.root_path.clone();
        for &node in self.ancestry(id).iter().rev() {
            path.push(self.name(node));
        }
        path
    }

    /// The node at a relative path with `/` separators, as [`Tree::path`]
    /// writes it. `""` is the root. Each step scans the siblings, so it costs
    /// the number of siblings along the path.
    #[must_use]
    pub fn find(&self, path: &str) -> Option<NodeId> {
        let mut node = NodeId::ROOT;
        for part in path.split('/').filter(|part| !part.is_empty()) {
            node = self.child_named(node, part)?;
        }
        Some(node)
    }

    /// The node and its ancestors up to, but not including, the root.
    fn ancestry(&self, id: NodeId) -> Vec<NodeId> {
        let mut chain = Vec::with_capacity(usize::from(self.depth(id)));
        let mut node = id;
        while let Some(parent) = self.parent(node) {
            chain.push(node);
            node = parent;
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

/// Converts an index the builder has already bounded to `u32`.
fn to_u32(i: usize) -> u32 {
    u32::try_from(i).expect("the builder rejects trees past u32 ids")
}
