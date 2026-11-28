//! Building a [`Tree`] from disk in two steps: [`scan`] lists names and
//! kinds with `ignore`'s parallel walker, then [`read_sizes`] measures every
//! file.
//!
//! The split exists for speed. Listing reads each folder once; a size needs
//! one `lstat` per file, and on macOS those calls contend in the kernel. On
//! the 1.7 million entries of the reference checkout the listing takes about
//! 2 s and the sizes about 5 s more, so the tree reaches the screen first
//! and the Size column fills in after.
//!
//! Each walker thread records what it sees in its own [`Batch`]: a name, a
//! kind and the key of the parent folder. Folder keys come from one shared
//! map from absolute folder path to key; each thread remembers the last
//! parent it looked up, so the map's lock is taken about once per folder
//! rather than once per entry. When the walk ends the batches are joined
//! into a [`RawTree`], which sorts itself into canonical order.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use ignore::{DirEntry, ParallelVisitor, ParallelVisitorBuilder, WalkBuilder, WalkState};

use crate::tree::{NO_PARENT, RawTree};
use crate::{Kind, Tree, TreeError};

mod changes;
mod outline;
mod sizes;
#[cfg(target_os = "macos")]
pub(crate) mod smb;

pub use changes::changes_matter;
pub use outline::DirectoryOutline;
pub use sizes::read_sizes;

/// Names skipped in every scan, whatever the ignore rules say.
const ALWAYS_SKIPPED: [&str; 2] = [".git", ".DS_Store"];

/// The most walker threads worth starting. On macOS, listing contends in
/// the kernel: on a 16-core M4 Max, listing 1.7 million entries took 2.0 s
/// on 5 or 6 threads, 2.5 s on 8 and 3.4 s on 10. (With a stat per file, as
/// before sizes moved to [`read_sizes`], 12 threads took 11.6 s.)
const MAX_WALK_THREADS: usize = 6;

/// The root folder's key in the shared folder map.
const ROOT_KEY: usize = 0;

/// Amortizes progress messages over entries already recorded by a worker.
const PROGRESS_BATCH: usize = 1024;

/// How to scan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanOptions {
    /// List the entries that `.gitignore`, `.ignore`, `.git/info/exclude`
    /// and the global git excludes would hide. `.git` and `.DS_Store` stay
    /// skipped.
    pub show_ignored: bool,
}

/// How a scan uses the local directory listing cache on supported volumes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheMode {
    /// Validate every directory and reread any whose stamp changed.
    Validate,
    /// Reread all listings, including on servers with coarse timestamps.
    Rebuild,
}

/// Why a scan produced no tree.
#[derive(Debug)]
pub enum ScanError {
    /// The root is missing, is neither a folder nor a regular file, or
    /// cannot be read. Holds the operating system's error.
    Root(io::Error),
    /// The cancel flag was set before the scan finished.
    Cancelled,
    /// The folder holds more than a [`Tree`] can index.
    Tree(TreeError),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root(err) => err.fmt(f),
            Self::Cancelled => f.write_str("the scan was cancelled"),
            Self::Tree(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for ScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Root(err) => Some(err),
            Self::Cancelled => None,
            Self::Tree(err) => Some(err),
        }
    }
}

impl From<TreeError> for ScanError {
    fn from(err: TreeError) -> Self {
        Self::Tree(err)
    }
}

/// Scans the folder at `root` into a tree of names and kinds, walking on up
/// to six threads. Call it off the UI thread. The tree has no sizes; run
/// [`read_sizes`] on it next. A file is a tree of one node, with its size,
/// which [`read_sizes`] leaves as it is.
///
/// The rules: `.gitignore`, `.ignore`, `.git/info/exclude` and the global
/// git excludes apply unless [`ScanOptions::show_ignored`] is set, inside a
/// git repository or not. Which ignore files count depends on where `root`
/// is:
///
/// - Inside a git work tree (`root` or a folder above it holds `.git`):
///   git's rule. Every `.gitignore` from the repository root down applies,
///   with that repository's `.git/info/exclude`; `.gitignore` files above
///   the repository root do not. `ignore` still reads `.ignore` files from
///   every folder above `root`, as ripgrep does.
/// - Outside a repository: only ignore files at or below `root`.
///
/// Dotfiles are listed; `.git` and `.DS_Store` are always skipped; symlinks
/// are leaves and never followed; FIFOs, sockets and devices are left out,
/// since reading one can block forever; a folder that cannot be read becomes
/// a [`Kind::Unreadable`] leaf. Entries the walker cannot describe at all
/// are left out. There are no caps.
///
/// `root` is made absolute and canonical first, so [`Tree::root_path`] is
/// the real folder and the root node is named after it: `.` scans the
/// current folder under its own name, and a root that is a symlink is its
/// target.
///
/// Setting `cancel` stops the walk at the next entry each thread visits,
/// so a superseded scan ends early.
///
/// # Errors
///
/// [`ScanError::Root`] if the root is missing, neither a folder nor a
/// regular file, or unreadable;
/// [`ScanError::Cancelled`] if `cancel` was set; [`ScanError::Tree`] past
/// the tree's id limits.
pub fn scan(root: &Path, options: ScanOptions, cancel: &AtomicBool) -> Result<Tree, ScanError> {
    scan_observed(root, options, cancel, None)
}

/// Scans as [`scan`] does. If supplied, `progress` accumulates the number
/// of entries recorded below the root. Workers add in batches so a UI can
/// sample the count without synchronizing on every file.
///
/// # Errors
///
/// The same errors as [`scan`].
pub fn scan_observed(
    root: &Path,
    options: ScanOptions,
    cancel: &AtomicBool,
    progress: Option<&AtomicUsize>,
) -> Result<Tree, ScanError> {
    let (root, file) = resolve_root(root)?;
    if let Some(size) = file {
        return Ok(Tree::file(&root, size));
    }
    let shared = Shared {
        cancel,
        progress,
        folders: Mutex::new(HashMap::from([(root.clone(), ROOT_KEY)])),
        batches: Mutex::new(Vec::new()),
    };
    walker(&root, options)
        .build_parallel()
        .visit(&mut Collectors(&shared));
    if cancel.load(Ordering::Relaxed) {
        return Err(ScanError::Cancelled);
    }
    let tree = assemble(&root, shared)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(ScanError::Cancelled);
    }
    Ok(tree)
}

/// Scans as [`scan_observed`] does. On macOS SMB mounts a configured local
/// cache can reuse directory listings. Progress on those mounts counts names
/// inspected before the current ignore rules are applied.
///
/// # Errors
///
/// The same errors as [`scan_observed`].
pub fn scan_with_cache(
    root: &Path,
    options: ScanOptions,
    cancel: &AtomicBool,
    progress: Option<&AtomicUsize>,
    cache: Option<(&Path, CacheMode)>,
) -> Result<Tree, ScanError> {
    #[cfg(target_os = "macos")]
    if let Some((cache, mode)) = cache
        && smb::is_smb(root)?
    {
        return smb::scan(root, options, cancel, progress, cache, mode);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = cache;
    scan_observed(root, options, cancel, progress)
}

/// What to scan: `root` made absolute and canonical, so the tree's root is
/// named after the real folder or file (not `.` or `..`) and a root that is
/// a symlink is its target; with its size when it is a regular file.
fn resolve_root(root: &Path) -> Result<(PathBuf, Option<u64>), ScanError> {
    let followed = fs::metadata(root).map_err(ScanError::Root)?;
    let file = if followed.is_dir() {
        None
    } else if followed.is_file() {
        Some(followed.len())
    } else {
        let message = format!("{} is not a folder or a file", root.display());
        return Err(ScanError::Root(io::Error::new(
            io::ErrorKind::NotADirectory,
            message,
        )));
    };
    Ok((canonical(root).map_err(ScanError::Root)?, file))
}

/// `path` made absolute and canonical, as a scan names its root: links
/// resolved, `.` and `..` gone. Folders are told apart by this path, so
/// every path compared with a tree's goes through it.
///
/// On Windows the system's answer starts with `\\?\`, which people do not
/// type and few programs take; it is dropped wherever the path means the
/// same without it. A path that needs it keeps it: a reserved name such as
/// `con`, a name ending in a dot, or more than 260 characters.
///
/// # Errors
///
/// The operating system's, when `path` does not exist or cannot be read.
#[cfg_attr(
    not(windows),
    expect(
        clippy::disallowed_methods,
        reason = "the one place that canonicalizes; elsewhere call this"
    )
)]
pub fn canonical(path: &Path) -> io::Result<PathBuf> {
    #[cfg(windows)]
    {
        dunce::canonicalize(path)
    }
    #[cfg(not(windows))]
    {
        fs::canonicalize(path)
    }
}

/// The walker with this app's rules, on up to [`MAX_WALK_THREADS`].
fn walker(root: &Path, options: ScanOptions) -> WalkBuilder {
    let mut builder = rules(root, options);
    builder
        .threads(walk_threads())
        .filter_entry(|entry| !always_skipped(entry));
    builder
}

/// Whether the scan leaves `entry` out whatever the ignore rules say.
fn always_skipped(entry: &DirEntry) -> bool {
    ALWAYS_SKIPPED.iter().any(|&name| entry.file_name() == name)
}

/// A walker over `root` with this app's ignore rules and no other filter.
fn rules(root: &Path, options: ScanOptions) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    if options.show_ignored {
        builder.standard_filters(false);
    } else if inside_git_work_tree(root) {
        // ignore's git mode (`require_git` and `parents`, both on by
        // default): `.gitignore` and `.git/info/exclude` files apply from
        // the repository root down, as in git; those above it do not.
        builder.hidden(false);
    } else {
        // Outside a repository, git's rules still apply, as in brainstrike,
        // but only from files at or below the root: nothing above it, where
        // a folder's ignore files belong to some other project.
        builder.hidden(false).parents(false).require_git(false);
    }
    builder.follow_links(false);
    builder
}

/// Whether `root` or a folder above it holds `.git`, a folder or (for a
/// worktree or submodule) a file: the test `ignore` makes for a repository.
fn inside_git_work_tree(root: &Path) -> bool {
    root.ancestors()
        .any(|dir| fs::metadata(dir.join(".git")).is_ok())
}

/// Walker threads: one per core, up to [`MAX_WALK_THREADS`].
fn walk_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().min(MAX_WALK_THREADS))
}

/// State the walker threads share.
struct Shared<'a> {
    cancel: &'a AtomicBool,
    progress: Option<&'a AtomicUsize>,
    /// Every folder seen, by absolute path, with its key.
    folders: Mutex<HashMap<PathBuf, usize>>,
    /// Each finished thread's batch.
    batches: Mutex<Vec<Batch>>,
}

impl Shared<'_> {
    /// The key of the folder at `path`, made on first sight. The walker
    /// visits a folder before its entries, but taking the key from either
    /// side keeps the result independent of thread timing.
    fn folder_key(&self, path: &Path) -> usize {
        let mut folders = self.folders.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(&key) = folders.get(path) {
            return key;
        }
        let key = folders.len();
        folders.insert(path.to_path_buf(), key);
        key
    }
}

/// What one walker thread recorded, one entry per visited path below the
/// root. Names share one buffer.
#[derive(Debug, Default)]
struct Batch {
    names: String,
    name_ends: Vec<usize>,
    parent_key: Vec<usize>,
    kind: Vec<Kind>,
    /// `(entry, key)` for each folder entry.
    folders: Vec<(usize, usize)>,
    /// Errors the walker reported to this thread.
    errors: Vec<ignore::Error>,
}

impl Batch {
    fn len(&self) -> usize {
        self.kind.len()
    }

    fn name(&self, entry: usize) -> &str {
        let start = entry.checked_sub(1).map_or(0, |prev| self.name_ends[prev]);
        self.names
            .get(start..self.name_ends[entry])
            .expect("ends are recorded after whole names, so they lie on char boundaries")
    }
}

/// Makes one [`Collector`] per walker thread.
struct Collectors<'s>(&'s Shared<'s>);

impl<'s> ParallelVisitorBuilder<'s> for Collectors<'s> {
    fn build(&mut self) -> Box<dyn ParallelVisitor + 's> {
        Box::new(Collector {
            shared: self.0,
            batch: Batch::default(),
            last_parent: None,
        })
    }
}

/// One walker thread's visitor. Its batch joins the shared list on drop.
struct Collector<'s> {
    shared: &'s Shared<'s>,
    batch: Batch,
    /// The parent folder of the previous entry, and its key.
    last_parent: Option<(PathBuf, usize)>,
}

impl ParallelVisitor for Collector<'_> {
    fn visit(&mut self, entry: Result<DirEntry, ignore::Error>) -> WalkState {
        if self.shared.cancel.load(Ordering::Relaxed) {
            return WalkState::Quit;
        }
        match entry {
            Ok(entry) => self.record(&entry),
            Err(err) => self.batch.errors.push(err),
        }
        WalkState::Continue
    }
}

impl Collector<'_> {
    fn record(&mut self, entry: &DirEntry) {
        // The root is made by `assemble`; its key is fixed.
        let Some(parent) = entry.path().parent().filter(|_| entry.depth() > 0) else {
            return;
        };
        // The type comes from the folder listing; nothing here stats the file.
        let Some(kind) = entry.file_type().map_or(Some(Kind::File), entry_kind) else {
            return;
        };
        let parent = self.parent_key(parent);
        let batch = &mut self.batch;
        let index = batch.len();
        batch.names.push_str(&entry.file_name().to_string_lossy());
        batch.name_ends.push(batch.names.len());
        batch.parent_key.push(parent);
        batch.kind.push(kind);
        if batch.len().is_multiple_of(PROGRESS_BATCH)
            && let Some(report) = self.shared.progress
        {
            report.fetch_add(PROGRESS_BATCH, Ordering::Relaxed);
        }
        if kind == Kind::Dir {
            batch
                .folders
                .push((index, self.shared.folder_key(entry.path())));
        }
    }

    fn parent_key(&mut self, parent: &Path) -> usize {
        match &self.last_parent {
            Some((path, key)) if path == parent => *key,
            _ => {
                let key = self.shared.folder_key(parent);
                self.last_parent = Some((parent.to_path_buf(), key));
                key
            }
        }
    }
}

impl Drop for Collector<'_> {
    fn drop(&mut self) {
        let batch = std::mem::take(&mut self.batch);
        let remaining = batch.len() % PROGRESS_BATCH;
        if remaining != 0
            && let Some(report) = self.shared.progress
        {
            report.fetch_add(remaining, Ordering::Relaxed);
        }
        self.shared
            .batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(batch);
    }
}

/// Kinds the explorer can display without opening a special file.
fn entry_kind(file_type: fs::FileType) -> Option<Kind> {
    if file_type.is_symlink() {
        Some(Kind::Symlink)
    } else if file_type.is_dir() {
        Some(Kind::Dir)
    } else if is_special(file_type) {
        None
    } else {
        Some(Kind::File)
    }
}

/// Whether the entry is a FIFO, a socket or a device: none is a file to
/// show, and opening a FIFO blocks until something writes to it.
#[cfg(unix)]
fn is_special(file_type: fs::FileType) -> bool {
    use std::os::unix::fs::FileTypeExt;
    file_type.is_fifo()
        || file_type.is_socket()
        || file_type.is_block_device()
        || file_type.is_char_device()
}

/// Elsewhere every entry that is neither a folder nor a link is a file.
#[cfg(not(unix))]
fn is_special(_: fs::FileType) -> bool {
    false
}

/// Joins the batches into a tree.
fn assemble(root: &Path, shared: Shared<'_>) -> Result<Tree, ScanError> {
    let folders = shared
        .folders
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let mut batches = shared
        .batches
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);

    let mut unreadable = Vec::new();
    for err in batches.iter_mut().flat_map(|batch| batch.errors.drain(..)) {
        match unreadable_folder(root, &folders, &err) {
            Some(ROOT_KEY) => {
                let message = err.to_string();
                let io = err
                    .into_io_error()
                    .unwrap_or_else(|| io::Error::other(message));
                return Err(ScanError::Root(io));
            }
            Some(key) => unreadable.push(key),
            None => {}
        }
    }

    // Raw index 0 is the root; each batch's entries follow in order.
    let mut folder_raw = vec![NO_PARENT; folders.len()];
    folder_raw[ROOT_KEY] = 0;
    let mut next = 1;
    for batch in &batches {
        for &(entry, key) in &batch.folders {
            folder_raw[key] = u32::try_from(next + entry).map_err(|_| TreeError::TooLarge)?;
        }
        next += batch.len();
    }

    let root_name = root
        .file_name()
        .unwrap_or(root.as_os_str())
        .to_string_lossy();
    let mut raw = RawTree::new(&root_name, false)?;
    for batch in &batches {
        for entry in 0..batch.len() {
            let parent = folder_raw[batch.parent_key[entry]];
            raw.push(parent, batch.name(entry), batch.kind[entry])?;
        }
    }
    for key in unreadable {
        let folder = folder_raw[key];
        if folder != NO_PARENT && raw.kind(folder) == Kind::Dir {
            raw.set_kind(folder, Kind::Unreadable);
        }
    }
    Ok(raw.build(root.to_path_buf())?)
}

/// The key of the folder whose listing failed, if `err` reports that.
/// Errors about single entries inside a folder carry the folder's path one
/// level deeper, and are not failures of the folder.
fn unreadable_folder(
    root: &Path,
    folders: &HashMap<PathBuf, usize>,
    err: &ignore::Error,
) -> Option<usize> {
    let path = error_path(err)?;
    let depth = path.strip_prefix(root).ok()?.components().count();
    (err.depth() == Some(depth))
        .then(|| folders.get(path).copied())
        .flatten()
}

/// The path an error names, through its wrappers.
fn error_path(err: &ignore::Error) -> Option<&Path> {
    match err {
        ignore::Error::WithPath { path, .. } => Some(path),
        ignore::Error::WithDepth { err, .. } | ignore::Error::WithLineNumber { err, .. } => {
            error_path(err)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
