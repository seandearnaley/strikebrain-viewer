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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use ignore::{DirEntry, ParallelVisitor, ParallelVisitorBuilder, WalkBuilder, WalkState};

use crate::tree::{NO_PARENT, RawTree};
use crate::{Kind, Tree, TreeError};

mod changes;
mod sizes;

pub use changes::changes_matter;
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

/// How to scan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanOptions {
    /// List the entries that `.gitignore`, `.ignore`, `.git/info/exclude`
    /// and the global git excludes would hide. `.git` and `.DS_Store` stay
    /// skipped.
    pub show_ignored: bool,
}

/// Why a scan produced no tree.
#[derive(Debug)]
pub enum ScanError {
    /// The root is missing, is not a folder, or cannot be read. Holds the
    /// operating system's error.
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
/// [`read_sizes`] on it next.
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
/// [`ScanError::Root`] if the root is missing, not a folder or unreadable;
/// [`ScanError::Cancelled`] if `cancel` was set; [`ScanError::Tree`] past
/// the tree's id limits.
pub fn scan(root: &Path, options: ScanOptions, cancel: &AtomicBool) -> Result<Tree, ScanError> {
    let root = resolve_root(root)?;
    let shared = Shared {
        cancel,
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

/// The folder to walk: `root` made absolute and canonical, so the tree's
/// root is named after the real folder (not `.` or `..`) and a root that is
/// a symlink is its target.
fn resolve_root(root: &Path) -> Result<PathBuf, ScanError> {
    let followed = fs::metadata(root).map_err(ScanError::Root)?;
    if !followed.is_dir() {
        let message = format!("{} is not a folder", root.display());
        return Err(ScanError::Root(io::Error::new(
            io::ErrorKind::NotADirectory,
            message,
        )));
    }
    fs::canonicalize(root).map_err(ScanError::Root)
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
        let kind = match entry.file_type() {
            Some(t) if t.is_symlink() => Kind::Symlink,
            Some(t) if t.is_dir() => Kind::Dir,
            Some(t) if is_special(t) => return,
            _ => Kind::File,
        };
        let parent = self.parent_key(parent);
        let batch = &mut self.batch;
        let index = batch.len();
        batch.names.push_str(&entry.file_name().to_string_lossy());
        batch.name_ends.push(batch.names.len());
        batch.parent_key.push(parent);
        batch.kind.push(kind);
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
        self.shared
            .batches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(batch);
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
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::io;
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use ignore::{ParallelVisitorBuilder, WalkState};
    use tempfile::TempDir;

    use super::{Collectors, ScanError, ScanOptions, Shared, read_sizes, scan};
    use crate::{Kind, NodeId, Tree, TreeError};

    /// A temporary folder holding `files` (path, contents); a path ending
    /// in `/` is an empty folder.
    fn folder(files: &[(&str, &str)]) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, contents) in files {
            let full = dir.path().join(path);
            if path.ends_with('/') {
                fs::create_dir_all(&full).unwrap();
            } else {
                fs::create_dir_all(full.parent().unwrap()).unwrap();
                fs::write(&full, contents).unwrap();
            }
        }
        dir
    }

    fn scan_default(dir: &Path) -> Tree {
        scan(dir, ScanOptions::default(), &AtomicBool::new(false)).unwrap()
    }

    fn scan_ignored_too(dir: &Path) -> Tree {
        scan(
            dir,
            ScanOptions { show_ignored: true },
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    fn paths(tree: &Tree) -> Vec<String> {
        tree.ids().skip(1).map(|id| tree.path(id)).collect()
    }

    fn id(tree: &Tree, path: &str) -> NodeId {
        tree.find(path)
            .unwrap_or_else(|| panic!("no node at {path:?}"))
    }

    #[test]
    fn a_scan_reads_names_kinds_and_sizes_in_canonical_order() {
        let dir = folder(&[
            ("b.txt", "12345"),
            ("A/one.md", "1"),
            ("A/two/file10.rs", "1234567890"),
            ("A/two/file2.rs", "12"),
            ("empty/", ""),
        ]);
        let tree = scan_default(dir.path());
        let real = fs::canonicalize(dir.path()).unwrap();
        assert_eq!(
            paths(&tree),
            [
                "A",
                "A/two",
                "A/two/file2.rs",
                "A/two/file10.rs",
                "A/one.md",
                "empty",
                "b.txt"
            ]
        );
        assert_eq!(tree.root_path(), real);
        assert_eq!(
            tree.name(NodeId::ROOT),
            real.file_name().unwrap().to_str().unwrap()
        );
        assert_eq!(tree.kind(id(&tree, "A/two")), Kind::Dir);
        assert_eq!(tree.kind(id(&tree, "b.txt")), Kind::File);
        assert!(!tree.has_sizes());
        assert_eq!(tree.size(NodeId::ROOT), None);
        read_sizes(&tree, &AtomicBool::new(false)).unwrap();
        assert_eq!(tree.size(id(&tree, "A/two/file10.rs")), Some(10));
        assert_eq!(tree.size(id(&tree, "A")), Some(13));
        assert_eq!(tree.size(NodeId::ROOT), Some(18));
        assert_eq!(
            tree.abs_path(id(&tree, "A/one.md")),
            real.join("A").join("one.md")
        );
    }

    #[test]
    fn a_scan_matches_the_same_listing_built_from_paths() {
        let mut files = Vec::new();
        for d in 0..20 {
            for f in 0..50 {
                files.push((format!("dir{d}/sub{}/f{f}.txt", f % 3), "x".repeat(f)));
            }
        }
        let listing: Vec<(&str, &str)> = files
            .iter()
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .collect();
        let dir = folder(&listing);
        let scanned = scan_default(dir.path());
        read_sizes(&scanned, &AtomicBool::new(false)).unwrap();
        let sized: Vec<(&str, u64)> = files
            .iter()
            .map(|(p, c)| (p.as_str(), c.len() as u64))
            .collect();
        let expected = Tree::from_paths_with_sizes(&sized).unwrap();
        assert_eq!(paths(&scanned), paths(&expected));
        for node in expected.ids() {
            assert_eq!(scanned.size(node), expected.size(node));
            assert_eq!(scanned.kind(node), expected.kind(node));
        }
    }

    #[test]
    fn a_root_reached_through_dot_dot_is_named_after_the_real_folder() {
        let dir = folder(&[("a/b.txt", "x"), ("c.txt", "x")]);
        let real = fs::canonicalize(dir.path()).unwrap();
        let tree = scan_default(&dir.path().join("a").join(".."));
        assert_eq!(tree.root_path(), real);
        assert_eq!(
            tree.name(NodeId::ROOT),
            real.file_name().unwrap().to_str().unwrap()
        );
        assert_eq!(paths(&tree), ["a", "a/b.txt", "c.txt"]);
    }

    #[test]
    fn a_relative_root_is_made_absolute() {
        let cwd = fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
        let tree = scan(
            Path::new("."),
            ScanOptions::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(tree.root_path(), cwd);
        assert_eq!(
            tree.name(NodeId::ROOT),
            cwd.file_name().unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn a_root_with_a_trailing_separator_scans_the_same() {
        let dir = folder(&[("a/b.txt", "x"), ("c.txt", "x")]);
        let mut with_slash = dir.path().as_os_str().to_owned();
        with_slash.push("/");
        assert_eq!(
            paths(&scan_default(Path::new(&with_slash))),
            ["a", "a/b.txt", "c.txt"]
        );
    }

    #[test]
    fn dotfiles_are_shown() {
        let dir = folder(&[(".hidden-notes", "x"), (".config/settings.toml", "y")]);
        let tree = scan_default(dir.path());
        assert_eq!(
            paths(&tree),
            [".config", ".config/settings.toml", ".hidden-notes"]
        );
    }

    #[test]
    fn git_and_ds_store_are_always_skipped() {
        let dir = folder(&[
            (".git/HEAD", "ref"),
            (".DS_Store", "x"),
            ("sub/.DS_Store", "x"),
            ("sub/keep.txt", "k"),
        ]);
        assert_eq!(paths(&scan_default(dir.path())), ["sub", "sub/keep.txt"]);
        assert_eq!(
            paths(&scan_ignored_too(dir.path())),
            ["sub", "sub/keep.txt"]
        );
    }

    #[test]
    fn gitignore_rules_hide_entries_unless_ignored_entries_are_shown() {
        let dir = folder(&[
            (".gitignore", "*.log\nbuild/\n"),
            ("app.log", "x"),
            ("build/out.bin", "x"),
            ("keep.txt", "x"),
            ("nested/.gitignore", "local.tmp\n"),
            ("nested/local.tmp", "x"),
            ("local.tmp", "x"),
        ]);
        assert_eq!(
            paths(&scan_default(dir.path())),
            [
                "nested",
                "nested/.gitignore",
                ".gitignore",
                "keep.txt",
                "local.tmp"
            ]
        );
        assert_eq!(
            paths(&scan_ignored_too(dir.path())),
            [
                "build",
                "build/out.bin",
                "nested",
                "nested/.gitignore",
                "nested/local.tmp",
                ".gitignore",
                "app.log",
                "keep.txt",
                "local.tmp"
            ]
        );
    }

    #[test]
    fn ignore_files_above_a_folder_outside_git_do_not_apply() {
        let dir = folder(&[
            ("outer/.gitignore", "*\n"),
            ("outer/.ignore", "*.md\n"),
            ("outer/inner/notes.md", "x"),
            ("outer/inner/src/main.rs", "x"),
        ]);
        let inner = dir.path().join("outer/inner");
        assert_eq!(
            paths(&scan_default(&inner)),
            ["src", "src/main.rs", "notes.md"]
        );
    }

    #[test]
    fn ignore_files_above_a_repository_do_not_apply() {
        let dir = folder(&[
            ("outer/.gitignore", "*\n"),
            ("outer/inner/.git/HEAD", "ref"),
            ("outer/inner/.gitignore", "*.log\n"),
            ("outer/inner/notes.md", "x"),
            ("outer/inner/debug.log", "x"),
            ("outer/inner/src/main.rs", "x"),
        ]);
        let inner = dir.path().join("outer/inner");
        assert_eq!(
            paths(&scan_default(&inner)),
            ["src", "src/main.rs", ".gitignore", "notes.md"]
        );
    }

    #[test]
    fn ignore_files_above_a_folder_inside_the_same_repository_apply() {
        let dir = folder(&[
            ("repo/.git/HEAD", "ref"),
            ("repo/.gitignore", "*.log\n/sub/build/\n"),
            ("repo/sub/a.log", "x"),
            ("repo/sub/b.txt", "x"),
            ("repo/sub/build/out.txt", "x"),
            ("repo/sub/deeper/c.log", "x"),
        ]);
        let sub = dir.path().join("repo/sub");
        assert_eq!(paths(&scan_default(&sub)), ["deeper", "b.txt"]);
        assert_eq!(
            paths(&scan_ignored_too(&sub)),
            [
                "build",
                "build/out.txt",
                "deeper",
                "deeper/c.log",
                "a.log",
                "b.txt"
            ]
        );
    }

    #[test]
    fn dot_ignore_rules_hide_entries() {
        let dir = folder(&[
            (".ignore", "secret.txt\n"),
            ("secret.txt", "x"),
            ("public.txt", "x"),
        ]);
        assert_eq!(paths(&scan_default(dir.path())), [".ignore", "public.txt"]);
        assert_eq!(
            paths(&scan_ignored_too(dir.path())),
            [".ignore", "public.txt", "secret.txt"]
        );
    }

    #[test]
    fn a_missing_root_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = scan(
            &dir.path().join("absent"),
            ScanOptions::default(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(
            matches!(&err, ScanError::Root(e) if e.kind() == io::ErrorKind::NotFound),
            "{err:?}"
        );
    }

    #[test]
    fn a_file_root_is_an_error() {
        let dir = folder(&[("file.txt", "x")]);
        let err = scan(
            &dir.path().join("file.txt"),
            ScanOptions::default(),
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(
            matches!(&err, ScanError::Root(e) if e.kind() == io::ErrorKind::NotADirectory),
            "{err:?}"
        );
    }

    #[test]
    fn a_cancelled_scan_returns_no_tree() {
        let dir = folder(&[("a/b.txt", "x")]);
        let err = scan(dir.path(), ScanOptions::default(), &AtomicBool::new(true)).unwrap_err();
        assert!(matches!(err, ScanError::Cancelled), "{err:?}");
    }

    #[test]
    fn a_visitor_quits_once_cancelled() {
        let cancel = AtomicBool::new(false);
        let shared = Shared {
            cancel: &cancel,
            folders: Mutex::new(HashMap::new()),
            batches: Mutex::new(Vec::new()),
        };
        let mut visitor = Collectors(&shared).build();
        let error = || Err(ignore::Error::Io(io::Error::other("x")));
        assert_eq!(visitor.visit(error()), WalkState::Continue);
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(visitor.visit(error()), WalkState::Quit);
        drop(visitor);
        let batches = shared.batches.into_inner().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].errors.len(), 1);
    }

    #[test]
    fn errors_explain_themselves() {
        let root = ScanError::Root(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
        assert_eq!(root.to_string(), "denied");
        assert!(std::error::Error::source(&root).is_some());
        assert_eq!(ScanError::Cancelled.to_string(), "the scan was cancelled");
        assert!(std::error::Error::source(&ScanError::Cancelled).is_none());
        let tree = ScanError::from(TreeError::TooDeep);
        assert_eq!(tree.to_string(), TreeError::TooDeep.to_string());
        assert!(std::error::Error::source(&tree).is_some());
    }

    #[cfg(unix)]
    mod unix {
        use std::fs;
        use std::os::unix::fs::{PermissionsExt, symlink};
        use std::os::unix::net::UnixListener;
        use std::path::{Path, PathBuf};
        use std::process::Command;
        use std::sync::atomic::AtomicBool;

        use super::{folder, id, paths, scan_default};
        use crate::scan::{ScanError, ScanOptions, read_sizes, scan};
        use crate::{Kind, NodeId};

        /// Removes all permissions from a folder and restores them on drop,
        /// so the temporary folder can be deleted even if the test fails.
        struct Locked(PathBuf);

        impl Locked {
            /// `None` when the lock does not stop reads, as for root.
            fn new(path: &Path) -> Option<Self> {
                fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
                let locked = Self(path.to_path_buf());
                fs::read_dir(path).is_err().then_some(locked)
            }
        }

        impl Drop for Locked {
            fn drop(&mut self) {
                fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }

        #[test]
        fn a_symlink_is_a_leaf_and_counts_as_a_file() {
            let dir = folder(&[("real/inside.txt", "x"), ("target.txt", "hello")]);
            symlink(dir.path().join("real"), dir.path().join("link-dir")).unwrap();
            symlink(dir.path().join("target.txt"), dir.path().join("link-file")).unwrap();
            symlink(dir.path().join("nowhere"), dir.path().join("dangling")).unwrap();
            let tree = scan_default(dir.path());
            assert_eq!(
                paths(&tree),
                [
                    "real",
                    "real/inside.txt",
                    "dangling",
                    "link-dir",
                    "link-file",
                    "target.txt"
                ]
            );
            for link in ["link-dir", "link-file", "dangling"] {
                let node = id(&tree, link);
                assert_eq!(tree.kind(node), Kind::Symlink, "{link}");
                assert!(tree.is_file(node), "{link}");
                assert_eq!(tree.subtree(node), node.index()..node.index() + 1);
            }
        }

        #[test]
        fn an_unreadable_folder_becomes_an_unreadable_leaf() {
            let dir = folder(&[("locked/secret.txt", "x"), ("open.txt", "x")]);
            let Some(_lock) = Locked::new(&dir.path().join("locked")) else {
                return;
            };
            let tree = scan_default(dir.path());
            assert_eq!(paths(&tree), ["locked", "open.txt"]);
            let locked = id(&tree, "locked");
            assert_eq!(tree.kind(locked), Kind::Unreadable);
            assert!(tree.kind(locked).is_folder());
            read_sizes(&tree, &AtomicBool::new(false)).unwrap();
            assert_eq!(tree.size(locked), Some(0));
        }

        #[test]
        fn an_unreadable_root_is_an_error() {
            let dir = folder(&[("inner/x.txt", "x")]);
            let root = dir.path().join("inner");
            let Some(_lock) = Locked::new(&root) else {
                return;
            };
            let err = scan(&root, ScanOptions::default(), &AtomicBool::new(false)).unwrap_err();
            assert!(
                matches!(&err, ScanError::Root(e) if e.kind() == std::io::ErrorKind::PermissionDenied),
                "{err:?}"
            );
        }

        #[test]
        fn a_symlinked_root_is_scanned_as_its_target() {
            let dir = folder(&[("real/a.txt", "x")]);
            let link = dir.path().join("link");
            symlink(dir.path().join("real"), &link).unwrap();
            let tree = scan_default(&link);
            assert_eq!(paths(&tree), ["a.txt"]);
            assert_eq!(tree.name(NodeId::ROOT), "real");
        }

        #[test]
        fn fifos_and_sockets_are_left_out() {
            let dir = folder(&[("keep.txt", "x"), ("sub/also.txt", "x")]);
            let _socket = UnixListener::bind(dir.path().join("socket")).unwrap();
            // `mkfifo` may be missing; the socket still tests the rule.
            let fifo = Command::new("mkfifo")
                .arg(dir.path().join("sub/pipe"))
                .status()
                .is_ok_and(|status| status.success());
            assert_eq!(
                fs::symlink_metadata(dir.path().join("sub/pipe")).is_ok(),
                fifo
            );
            let tree = scan_default(dir.path());
            assert_eq!(paths(&tree), ["sub", "sub/also.txt", "keep.txt"]);
            read_sizes(&tree, &AtomicBool::new(false)).unwrap();
            assert_eq!(tree.size(NodeId::ROOT), Some(2));
        }
    }
}
