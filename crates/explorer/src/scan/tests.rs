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
    let real = crate::canonical(dir.path()).unwrap();
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
    let real = crate::canonical(dir.path()).unwrap();
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
    let cwd = crate::canonical(&std::env::current_dir().unwrap()).unwrap();
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
fn a_file_scans_as_one_node_with_its_size() {
    let dir = folder(&[("photo.png", "12345")]);
    let path = dir.path().join("photo.png");

    let tree = scan(&path, ScanOptions::default(), &AtomicBool::new(false)).unwrap();
    assert_eq!(tree.node_count(), 1);
    assert!(tree.is_file(NodeId::ROOT));
    assert_eq!(tree.name(NodeId::ROOT), "photo.png");
    assert_eq!(tree.size(NodeId::ROOT), Some(5));
    assert!(tree.has_sizes());
    assert_eq!(tree.root_path(), crate::canonical(&path).unwrap());
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
        progress: None,
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
    fn a_fifo_root_is_an_error() {
        let dir = folder(&[]);
        let pipe = dir.path().join("pipe");
        let made = Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .is_ok_and(|status| status.success());
        if !made {
            return;
        }
        let err = scan(&pipe, ScanOptions::default(), &AtomicBool::new(false)).unwrap_err();
        assert!(
            matches!(&err, ScanError::Root(e) if e.kind() == std::io::ErrorKind::NotADirectory),
            "{err:?}"
        );
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

#[test]
fn an_outline_reads_only_requested_directories_with_the_full_scans_rules() {
    let dir = folder(&[
        (".gitignore", "*.tmp"),
        ("a/hidden.tmp", ""),
        ("a/b/x.txt", ""),
        ("root.txt", ""),
    ]);
    let root = crate::canonical(dir.path()).unwrap();
    let cancel = AtomicBool::new(false);
    let (mut outline, first) =
        super::DirectoryOutline::open(&root, ScanOptions::default(), None, &cancel).unwrap();
    assert_eq!(paths(&first), ["a", ".gitignore", "root.txt"]);
    // Changing an unvisited directory before expanding it proves the read is on demand.
    fs::write(root.join("a/new.txt"), "").unwrap();
    let expanded = outline.read(&root.join("a"), &cancel).unwrap().unwrap();
    assert_eq!(
        paths(&expanded),
        ["a", "a/b", "a/new.txt", ".gitignore", "root.txt"]
    );
    assert!(outline.read(&root.join("a"), &cancel).unwrap().is_none());
    let last = outline.read(&root.join("a/b"), &cancel).unwrap().unwrap();
    assert_eq!(paths(&last), paths(&scan_default(&root)));
}

#[test]
fn restoring_an_outline_reads_the_saved_branches_and_retains_explicit_file_checks() {
    let dir = folder(&[
        ("a/b/read.txt", ""),
        ("c/chosen.txt", ""),
        ("other/skipped.txt", ""),
    ]);
    let root = crate::canonical(dir.path()).unwrap();
    let saved = crate::SavedFolder {
        path: root.clone(),
        open: vec![String::new(), "a/b".into()],
        selection: crate::SavedSelection {
            folders: vec![],
            files: vec!["c/chosen.txt".into()],
        },
    };
    let (_, tree) = super::DirectoryOutline::open(
        &root,
        ScanOptions::default(),
        Some(&saved),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(tree.find("a/b/read.txt").is_some());
    assert!(tree.find("c/chosen.txt").is_some());
    assert!(tree.find("other/skipped.txt").is_none());
}

#[test]
fn outline_cancellation_and_missing_directories_do_not_publish_a_complete_tree() {
    let dir = folder(&[("gone/", "")]);
    let root = crate::canonical(dir.path()).unwrap();
    let cancel = AtomicBool::new(false);
    let (mut outline, _) =
        super::DirectoryOutline::open(&root, ScanOptions::default(), None, &cancel).unwrap();
    let gone = root.join("gone");
    fs::remove_dir(&gone).unwrap();
    let tree = outline.read(&gone, &cancel).unwrap().unwrap();
    assert_eq!(tree.kind(id(&tree, "gone")), Kind::Unreadable);
    assert!(matches!(
        super::DirectoryOutline::open(&gone, ScanOptions::default(), None, &cancel),
        Err(ScanError::Root(_))
    ));
    cancel.store(true, Ordering::Relaxed);
    assert!(matches!(
        super::DirectoryOutline::open(&root, ScanOptions::default(), None, &cancel),
        Err(ScanError::Cancelled)
    ));
}

#[test]
fn walker_progress_counts_each_listed_entry_once() {
    let dir = folder(&[("nested/", ""), (".gitignore", "*.tmp"), ("skip.tmp", "")]);
    for n in 0..1100 {
        fs::write(dir.path().join("nested").join(n.to_string()), "").unwrap();
    }
    let count = std::sync::atomic::AtomicUsize::new(0);
    let tree = super::scan_observed(
        dir.path(),
        ScanOptions::default(),
        &AtomicBool::new(false),
        Some(&count),
    )
    .unwrap();
    assert_eq!(count.load(Ordering::Relaxed), tree.node_count() - 1);
}

#[cfg(target_os = "macos")]
#[test]
fn reopening_reuses_only_unchanged_directory_listings_and_reads_rules_and_sizes_live() {
    let dir = folder(&[
        ("a/deep/old", "x"),
        ("b/keep", "a"),
        ("b/hide", "h"),
        (".ignore", "hide\n"),
    ]);
    let cache = tempfile::tempdir().unwrap();
    let manifest = cache.path().join("index");
    let root = crate::canonical(dir.path()).unwrap();
    let cancel = AtomicBool::new(false);
    let reopen = || {
        super::smb::indexed(
            &root,
            ScanOptions::default(),
            &cancel,
            None,
            &manifest,
            super::CacheMode::Validate,
        )
        .unwrap()
    };
    let (first, read) = reopen();
    assert_eq!(read, 4);
    assert_eq!(paths(&first), paths(&scan_default(&root)));
    assert_eq!(reopen().1, 0);
    fs::rename(root.join("a/deep/old"), root.join("a/deep/new")).unwrap();
    fs::write(root.join(".ignore"), "keep\n").unwrap();
    fs::write(root.join("b/hide"), "updated").unwrap();
    let (next, read) = reopen();
    assert_eq!(read, 1);
    assert_eq!(paths(&next), paths(&scan_default(&root)));
    read_sizes(&next, &cancel).unwrap();
    assert_eq!(next.size(id(&next, "b/hide")), Some(7));
}

#[cfg(target_os = "macos")]
#[test]
fn damaged_cache_is_rebuilt_and_cancelled_scans_do_not_replace_it() {
    use super::{CacheMode, smb::indexed};
    let dir = folder(&[("a/b", "b")]);
    let root = crate::canonical(dir.path()).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let file = cache.path().join("cache");
    fs::write(&file, b"incomplete").unwrap();
    let scan = |mode, stop| {
        indexed(
            &root,
            ScanOptions::default(),
            &AtomicBool::new(stop),
            None,
            &file,
            mode,
        )
    };
    assert_eq!(scan(CacheMode::Validate, false).unwrap().1, 2);
    let before = fs::read(&file).unwrap();
    assert!(matches!(
        scan(CacheMode::Rebuild, true),
        Err(ScanError::Cancelled)
    ));
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(scan(CacheMode::Validate, false).unwrap().1, 0);
    assert_eq!(scan(CacheMode::Rebuild, false).unwrap().1, 2);
}

#[cfg(target_os = "macos")]
#[test]
fn reopening_after_directory_replacement_treats_links_as_leaves_and_prunes_deleted_folders() {
    use super::{CacheMode, smb::indexed};
    use std::os::unix::fs::symlink;
    let dir = folder(&[("a/b/old", "x"), ("delete/gone", "y")]);
    let outside = folder(&[("external", "z")]);
    let root = crate::canonical(dir.path()).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let file = cache.path().join("cache");
    let reopen = || {
        indexed(
            &root,
            ScanOptions::default(),
            &AtomicBool::new(false),
            None,
            &file,
            CacheMode::Validate,
        )
        .unwrap()
    };
    assert_eq!(reopen().1, 4);
    fs::remove_dir_all(root.join("a/b")).unwrap();
    symlink(outside.path(), root.join("a/b")).unwrap();
    fs::remove_dir_all(root.join("delete")).unwrap();
    fs::create_dir(root.join("new")).unwrap();
    fs::write(root.join("new/file"), "z").unwrap();
    let (tree, reads) = reopen();
    assert_eq!(reads, 3);
    assert_eq!(tree.kind(id(&tree, "a/b")), Kind::Symlink);
    assert_eq!(paths(&tree), paths(&scan_default(&root)));
    assert_eq!(reopen().1, 0);
}

#[cfg(target_os = "macos")]
#[test]
fn reopen_displays_previous_sizes_until_offline_edits_are_verified() {
    use std::os::unix::fs::MetadataExt as _;

    let dir = folder(&[("images/a", "abc"), ("images/b", "12345")]);
    let root = crate::canonical(dir.path()).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let cancel = AtomicBool::new(false);
    let reopen = || {
        super::smb::indexed(
            &root,
            ScanOptions::default(),
            &cancel,
            None,
            &cache.path().join("index"),
            super::CacheMode::Validate,
        )
        .unwrap()
        .0
    };
    let first = reopen();
    read_sizes(&first, &cancel).unwrap();
    fs::write(root.join("images/a"), "abcdef").unwrap();
    drop(first);
    let next = reopen();
    assert!(!next.has_sizes());
    assert_eq!(next.display_size(id(&next, "images/a")), Some(3));
    let joined = Tree::join(&[(&next, NodeId::ROOT)]).unwrap();
    assert_eq!(joined.display_size(NodeId::ROOT), Some(8));
    read_sizes(&next, &cancel).unwrap();
    assert_eq!(joined.display_size(NodeId::ROOT), Some(11));
    assert_eq!(next.size(id(&next, "images/a")), Some(6));
    let snapshot = cache.path().join("index.sizes-v1");
    let written = fs::metadata(&snapshot).unwrap();
    let unchanged = reopen();
    read_sizes(&unchanged, &cancel).unwrap();
    assert_eq!(written.ino(), fs::metadata(&snapshot).unwrap().ino());
    fs::rename(root.join("images/b"), root.join("images/c")).unwrap();
    let changed = reopen();
    assert_eq!(changed.display_size(id(&changed, "images/a")), Some(6));
    assert_eq!(changed.display_size(id(&changed, "images/c")), None);
    assert_eq!(changed.display_size(NodeId::ROOT), None);
}
