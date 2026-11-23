//! Which changes on disk a new scan would show: the filter a file watcher
//! puts in front of a rescan, so a busy folder the ignore rules hide (a
//! build's `target/`, `node_modules/`, `.git/`) never causes one.
//!
//! The decision uses the scan's own rules. A changed path the tree lists
//! matters: something on screen changed or went. Otherwise the first part
//! of the path the tree does not list matters only if it still exists and
//! the walker, with the same ignore rules and filter, would list it. That
//! asks the walker about one path, reading only the folders on the way to
//! it, so a burst of changes under one ignored folder costs one small walk.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use super::{ALWAYS_SKIPPED, ScanOptions, always_skipped, is_special, rules};
use crate::{Kind, NodeId, Tree};

/// Whether a change at any of `changed` can alter what [`super::scan`] of
/// the folder at `folder` in `tree` lists with `options`: the root of a
/// scan, or a top-level folder of a joined tree. `changed` holds paths as a
/// file watcher on that folder's [`Tree::abs_path`] reports them: absolute,
/// under the folder.
///
/// - A path inside `.git`, or a `.DS_Store`, never matters: the scan skips
///   both.
/// - A path the tree lists matters, whatever happened to it.
/// - Otherwise only the first part of the path the tree lacks is asked
///   about: if it is gone, nothing the tree shows changed; if it exists, it
///   matters when the scan would list it, so a new file shows and a file
///   the ignore rules hide does not.
/// - The folder itself, and a path outside it, matter: something larger
///   happened than one entry.
///
/// Reads the disk: call it off the UI thread.
#[must_use]
pub fn changes_matter(
    tree: &Tree,
    folder: NodeId,
    options: ScanOptions,
    changed: &[PathBuf],
) -> bool {
    let root = tree.abs_path(folder);
    let root = root.as_path();
    let mut asked: HashSet<PathBuf> = HashSet::new();
    for path in changed {
        let Ok(relative) = path.strip_prefix(root) else {
            return true;
        };
        let Some(names) = names(relative) else {
            return true;
        };
        if names.is_empty() {
            return true;
        }
        if names.iter().any(|name| ALWAYS_SKIPPED.contains(name)) {
            continue;
        }
        let (deepest, listed) = deepest_listed(tree, folder, &names);
        if listed == names.len() {
            return true;
        }
        // Inside a file, a link or a folder the scan could not read: the
        // tree lists nothing there to change.
        if tree.kind(deepest) != Kind::Dir {
            continue;
        }
        let first_missing: PathBuf = root.join(names[..=listed].iter().collect::<PathBuf>());
        if asked.insert(first_missing.clone()) && would_list(root, &first_missing, options) {
            return true;
        }
    }
    false
}

/// The names of `relative`'s parts, or `None` if a part is not a plain
/// name in UTF-8 (`..`, or bytes the tree cannot hold as they are).
fn names(relative: &Path) -> Option<Vec<&str>> {
    relative
        .components()
        .map(|part| match part {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect()
}

/// The deepest node on the way down `names` from `folder` and how many
/// names it took to reach it: `names.len()` when the tree lists the whole
/// path.
fn deepest_listed(tree: &Tree, folder: NodeId, names: &[&str]) -> (NodeId, usize) {
    let mut node = folder;
    for (depth, name) in names.iter().enumerate() {
        let child = tree
            .children(node)
            .iter()
            .copied()
            .find(|&child| tree.name(child) == *name);
        match child {
            Some(child) => node = child,
            None => return (node, depth),
        }
    }
    (node, names.len())
}

/// Whether a scan of `root` with `options` would list `target`, a path
/// under it that exists: the same ignore rules and filter as the scan,
/// walking only the folders on the way to `target`.
fn would_list(root: &Path, target: &Path, options: ScanOptions) -> bool {
    if fs::symlink_metadata(target).is_err() {
        return false;
    }
    let depth = target
        .strip_prefix(root)
        .map_or(0, |relative| relative.components().count());
    let mut walker = rules(root, options);
    let on_the_way = target.to_path_buf();
    walker
        .threads(1)
        .max_depth(Some(depth))
        .filter_entry(move |entry| !always_skipped(entry) && on_the_way.starts_with(entry.path()));
    walker
        .build()
        .filter_map(Result::ok)
        .any(|entry| entry.path() == target && !entry.file_type().is_some_and(is_special))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::AtomicBool;

    use tempfile::TempDir;

    use crate::{NodeId, ScanOptions, Tree, scan};

    /// Whether the changes matter to the scan `tree`, whose root is the folder.
    fn changes_matter(tree: &Tree, options: ScanOptions, changed: &[PathBuf]) -> bool {
        super::changes_matter(tree, NodeId::ROOT, options, changed)
    }

    const RULES: ScanOptions = ScanOptions {
        show_ignored: false,
    };
    const EVERYTHING: ScanOptions = ScanOptions { show_ignored: true };

    /// A folder with a `.gitignore` hiding `target/` and `*.log`, scanned.
    fn project() -> (TempDir, Tree) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in [
            (".gitignore", "target/\n*.log\n"),
            ("src/main.rs", "fn main() {}"),
            ("target/debug/app", "binary"),
            ("README.md", "hi"),
        ] {
            let full = dir.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, text).unwrap();
        }
        let tree = scan(dir.path(), RULES, &AtomicBool::new(false)).unwrap();
        (dir, tree)
    }

    fn at(tree: &Tree, path: &str) -> Vec<PathBuf> {
        vec![tree.root_path().join(path)]
    }

    fn touch(root: &Path, path: &str) {
        let full = root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, "x").unwrap();
    }

    #[test]
    fn a_change_to_a_listed_path_matters_even_once_it_is_gone() {
        let (dir, tree) = project();
        assert!(changes_matter(&tree, RULES, &at(&tree, "src/main.rs")));
        assert!(changes_matter(&tree, RULES, &at(&tree, "src")));
        fs::remove_file(dir.path().join("README.md")).unwrap();
        assert!(changes_matter(&tree, RULES, &at(&tree, "README.md")));
        // So does the root, and anything outside it.
        assert!(changes_matter(&tree, RULES, &at(&tree, "")));
        assert!(changes_matter(
            &tree,
            RULES,
            &[PathBuf::from("/elsewhere/x")]
        ));
        assert!(changes_matter(&tree, RULES, &at(&tree, "src/../x")));
    }

    #[test]
    fn a_new_entry_matters_when_the_scan_would_list_it() {
        let (dir, tree) = project();
        touch(dir.path(), "src/new.rs");
        touch(dir.path(), "docs/deep/guide.md");
        assert!(changes_matter(&tree, RULES, &at(&tree, "src/new.rs")));
        assert!(changes_matter(
            &tree,
            RULES,
            &at(&tree, "docs/deep/guide.md")
        ));
        // Gone again before the batch was looked at: nothing to show.
        assert!(!changes_matter(&tree, RULES, &at(&tree, "src/brief.rs")));
    }

    #[test]
    fn changes_the_ignore_rules_hide_do_not_matter() {
        let (dir, tree) = project();
        touch(dir.path(), "target/debug/deps/lib.rlib");
        touch(dir.path(), "server.log");
        touch(dir.path(), "src/trace.log");
        let changed: Vec<PathBuf> = [
            "target/debug/deps/lib.rlib",
            "target/debug/app",
            "server.log",
        ]
        .iter()
        .chain(&["src/trace.log", ".git/index", "src/.DS_Store"])
        .flat_map(|path| at(&tree, path))
        .collect();
        assert!(!changes_matter(&tree, RULES, &changed));
        // With ignored files shown they are listed, so they matter; `.git`
        // and `.DS_Store` still do not.
        assert!(changes_matter(
            &tree,
            EVERYTHING,
            &at(&tree, "target/debug/app")
        ));
        assert!(!changes_matter(&tree, EVERYTHING, &at(&tree, ".git/index")));
    }

    #[test]
    fn nothing_inside_a_file_or_a_link_matters() {
        let (dir, tree) = project();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("src"), dir.path().join("link")).unwrap();
            let tree = scan(dir.path(), RULES, &AtomicBool::new(false)).unwrap();
            assert!(!changes_matter(&tree, RULES, &at(&tree, "link/main.rs")));
        }
        assert!(!changes_matter(&tree, RULES, &at(&tree, "README.md/inner")));
    }

    #[test]
    fn a_change_matters_to_the_joined_folder_it_is_in() {
        let (dir, tree) = project();
        let other = Tree::from_paths_at(Path::new("/elsewhere"), &[("x.txt", 1)]).unwrap();
        let joined = Tree::join(&[(&other, NodeId::ROOT), (&tree, NodeId::ROOT)]).unwrap();
        let folder = joined.find_abs(tree.root_path()).unwrap();
        touch(dir.path(), "src/new.rs");
        let changed = at(&tree, "src/new.rs");
        assert!(super::changes_matter(&joined, folder, RULES, &changed));
        assert!(!super::changes_matter(
            &joined,
            folder,
            RULES,
            &at(&tree, "target/debug/app")
        ));
        // To the other folder it is a path outside: something larger.
        let first = joined.find_abs(Path::new("/elsewhere")).unwrap();
        assert!(super::changes_matter(&joined, first, RULES, &changed));
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_the_scan_leaves_out_does_not_matter() {
        let (dir, tree) = project();
        let fifo = dir.path().join("pipe");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        assert!(!changes_matter(&tree, RULES, &at(&tree, "pipe")));
    }
}
