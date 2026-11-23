//! Building a [`Tree`] from disk with `ignore`'s parallel walker.
//!
//! Each walker thread records what it sees in its own [`Batch`]: a name, a
//! kind, a size and the key of the parent folder. Folder keys come from one
//! shared map from absolute folder path to key; each thread remembers the
//! last parent it looked up, so the map's lock is taken about once per
//! folder rather than once per entry. When the walk ends the batches are
//! joined into a [`RawTree`], which sorts itself into canonical order.

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

/// Names skipped in every scan, whatever the ignore rules say.
const ALWAYS_SKIPPED: [&str; 2] = [".git", ".DS_Store"];

/// The most walker threads worth starting. On macOS, listing and `lstat`
/// contend in the kernel: on a 16-core M4 Max, scanning 1.7 million entries
/// took 6.1 s on 6 or 8 threads and 11.6 s on 12, the `ignore` default.
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

/// Scans the folder at `root` into a tree, walking on up to six threads.
/// Call it off the UI thread.
///
/// The rules: `.gitignore`, `.ignore`, `.git/info/exclude` and the global
/// git excludes apply unless [`ScanOptions::show_ignored`] is set, inside a
/// git repository or not; dotfiles are listed; `.git` and `.DS_Store` are
/// always skipped; symlinks are leaves and never followed, except that a
/// root which is itself a symlink is resolved first; a folder that cannot
/// be read becomes a [`Kind::Unreadable`] leaf; sizes come from the
/// walker's metadata. Entries the walker cannot describe at all are left
/// out. There are no caps.
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

/// The folder to walk: `root` itself, or its target when `root` is a symlink.
fn resolve_root(root: &Path) -> Result<PathBuf, ScanError> {
    let followed = fs::metadata(root).map_err(ScanError::Root)?;
    if !followed.is_dir() {
        let message = format!("{} is not a folder", root.display());
        return Err(ScanError::Root(io::Error::new(
            io::ErrorKind::NotADirectory,
            message,
        )));
    }
    let link = fs::symlink_metadata(root).map_err(ScanError::Root)?;
    if link.file_type().is_symlink() {
        fs::canonicalize(root).map_err(ScanError::Root)
    } else {
        Ok(root.to_path_buf())
    }
}

/// The walker with this app's rules.
fn walker(root: &Path, options: ScanOptions) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    if options.show_ignored {
        builder.standard_filters(false);
    } else {
        // Dotfiles show. Git's rules apply outside a repository too, as in
        // brainstrike.
        builder.hidden(false).require_git(false);
    }
    builder
        .threads(walk_threads())
        .follow_links(false)
        .filter_entry(|entry| !ALWAYS_SKIPPED.iter().any(|&name| entry.file_name() == name));
    builder
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
    size: Vec<u64>,
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
        let parent = self.parent_key(parent);
        let kind = match entry.file_type() {
            Some(t) if t.is_symlink() => Kind::Symlink,
            Some(t) if t.is_dir() => Kind::Dir,
            _ => Kind::File,
        };
        let size = match kind {
            Kind::File | Kind::Symlink => entry.metadata().map_or(0, |meta| meta.len()),
            Kind::Dir | Kind::Unreadable => 0,
        };
        let batch = &mut self.batch;
        let index = batch.len();
        batch.names.push_str(&entry.file_name().to_string_lossy());
        batch.name_ends.push(batch.names.len());
        batch.parent_key.push(parent);
        batch.kind.push(kind);
        batch.size.push(size);
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
    let mut raw = RawTree::new(&root_name)?;
    for batch in &batches {
        for entry in 0..batch.len() {
            let parent = folder_raw[batch.parent_key[entry]];
            raw.push(
                parent,
                batch.name(entry),
                batch.kind[entry],
                batch.size[entry],
            )?;
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

    use super::{Collectors, ScanError, ScanOptions, Shared, scan};
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
        assert_eq!(tree.root_path(), dir.path());
        assert_eq!(
            tree.name(NodeId::ROOT),
            dir.path().file_name().unwrap().to_str().unwrap()
        );
        assert_eq!(tree.kind(id(&tree, "A/two")), Kind::Dir);
        assert_eq!(tree.kind(id(&tree, "b.txt")), Kind::File);
        assert_eq!(tree.size(id(&tree, "A/two/file10.rs")), 10);
        assert_eq!(tree.size(id(&tree, "A")), 13);
        assert_eq!(tree.size(NodeId::ROOT), 18);
        assert_eq!(
            tree.abs_path(id(&tree, "A/one.md")),
            dir.path().join("A").join("one.md")
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
        use std::path::{Path, PathBuf};
        use std::sync::atomic::AtomicBool;

        use super::{folder, id, paths, scan_default};
        use crate::scan::{ScanError, ScanOptions, scan};
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
            assert_eq!(tree.size(locked), 0);
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
    }
}
