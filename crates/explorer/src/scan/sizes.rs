//! [`read_sizes`]: the second step of a scan, one `lstat` per file.
//!
//! The cost is the kernel's, not ours. On a 16-core M4 Max, measuring the
//! 1.56 million files of the reference checkout took 12.6 s on one thread,
//! 5.1 s on 4, 4.8 s on 5, 5.4 s on 6, 6.2 s on 8 and 12 s on 12: with more
//! files than the vnode cache holds (`kern.maxvnodes`, 263,168 here), every
//! stat makes the kernel recycle a vnode, and those calls contend. `find .
//! -type f -size +0` on the same folder takes 8.1 s. Listing each folder
//! again and measuring through `std::fs::DirEntry::metadata`, which std
//! answers with `fstatat` relative to the open folder, was slower (5.3 s on
//! 5 threads), so the lookup by full path stays.

use std::fs;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use super::ScanError;
use crate::{NodeId, Tree};

/// Folders a thread takes at a time.
const BATCH: usize = 32;

/// Files measured between looks at the cancel flag.
const CANCEL_EVERY: usize = 256;

/// The most threads worth starting; see the module notes.
const MAX_SIZE_THREADS: usize = 5;

/// Measures every file of a scanned tree and stores the sizes in it, so
/// [`Tree::size`] answers from then on. Call it off the UI thread, after
/// [`super::scan`]. Each call measures every file again unless the tree
/// already has sizes, such as one from [`Tree::from_paths_with_sizes`]; then
/// it returns at once.
///
/// A file's size is its `lstat` size: a symlink counts as the link itself.
/// A file that cannot be measured, for example because it was deleted
/// after the scan, counts as 0.
///
/// # Errors
///
/// [`ScanError::Cancelled`] if `cancel` was set; the tree then has no sizes.
pub fn read_sizes(tree: &Tree, cancel: &AtomicBool) -> Result<(), ScanError> {
    if tree.has_sizes() {
        return Ok(());
    }
    // Folders with at least one file. Files come after folders among a
    // folder's children, so the last child tells.
    let folders: Vec<NodeId> = tree
        .ids()
        .filter(|&id| {
            tree.children(id)
                .last()
                .is_some_and(|&last| tree.is_file(last))
        })
        .collect();
    let sizes: Vec<AtomicU64> = (0..tree.node_count()).map(|_| AtomicU64::new(0)).collect();
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads() {
            scope.spawn(|| {
                loop {
                    let start = next.fetch_add(BATCH, Ordering::Relaxed);
                    let batch = folders.get(start..folders.len().min(start + BATCH));
                    match batch {
                        Some(batch) if !batch.is_empty() && !cancel.load(Ordering::Relaxed) => {
                            for &folder in batch {
                                measure(tree, folder, &sizes, cancel);
                            }
                        }
                        _ => break,
                    }
                }
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(ScanError::Cancelled);
    }
    tree.set_sizes(sizes.into_iter().map(AtomicU64::into_inner).collect());
    Ok(())
}

/// Threads to measure on: one per core, up to [`MAX_SIZE_THREADS`].
fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| NonZeroUsize::get(n).min(MAX_SIZE_THREADS))
}

/// Stores the size of each file directly in `folder`. The folder's path is
/// built once; each file's name is pushed onto it and popped again.
fn measure(tree: &Tree, folder: NodeId, sizes: &[AtomicU64], cancel: &AtomicBool) {
    let children = tree.children(folder);
    let files = &children[children.partition_point(|&child| !tree.is_file(child))..];
    let mut path = tree.abs_path(folder);
    for (i, &file) in files.iter().enumerate() {
        if i % CANCEL_EVERY == CANCEL_EVERY - 1 && cancel.load(Ordering::Relaxed) {
            return;
        }
        path.push(tree.name(file));
        if let Ok(meta) = fs::symlink_metadata(&path) {
            sizes[file.index()].store(meta.len(), Ordering::Relaxed);
        }
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::AtomicBool;

    use super::read_sizes;
    use crate::{NodeId, ScanError, ScanOptions, Tree, scan};

    #[test]
    fn sizes_arrive_after_the_listing_and_sum_into_folders() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("a/b/one.txt"), "1").unwrap();
        fs::write(dir.path().join("a/two.txt"), "22").unwrap();
        fs::write(dir.path().join("three.txt"), "333").unwrap();
        fs::create_dir(dir.path().join("empty")).unwrap();
        let tree = scan(dir.path(), ScanOptions::default(), &AtomicBool::new(false)).unwrap();
        assert!(!tree.has_sizes());
        read_sizes(&tree, &AtomicBool::new(false)).unwrap();
        let size = |path: &str| tree.size(tree.find(path).unwrap());
        assert_eq!(size("a/b/one.txt"), Some(1));
        assert_eq!(size("a/b"), Some(1));
        assert_eq!(size("a"), Some(3));
        assert_eq!(size("empty"), Some(0));
        assert_eq!(size(""), Some(6));
    }

    #[test]
    fn a_file_deleted_after_the_listing_counts_as_zero() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("gone.txt"), "12345").unwrap();
        fs::write(dir.path().join("kept.txt"), "12").unwrap();
        let tree = scan(dir.path(), ScanOptions::default(), &AtomicBool::new(false)).unwrap();
        fs::remove_file(dir.path().join("gone.txt")).unwrap();
        read_sizes(&tree, &AtomicBool::new(false)).unwrap();
        assert_eq!(tree.size(tree.find("gone.txt").unwrap()), Some(0));
        assert_eq!(tree.size(NodeId::ROOT), Some(2));
    }

    #[test]
    fn a_cancelled_read_leaves_the_tree_without_sizes() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..600 {
            fs::write(dir.path().join(format!("f{i}")), "x").unwrap();
        }
        let tree = scan(dir.path(), ScanOptions::default(), &AtomicBool::new(false)).unwrap();
        let err = read_sizes(&tree, &AtomicBool::new(true)).unwrap_err();
        assert!(matches!(err, ScanError::Cancelled), "{err:?}");
        assert!(!tree.has_sizes());
    }

    #[test]
    fn a_tree_with_sizes_is_left_alone() {
        let tree = Tree::from_paths_with_sizes(&[("a", 7)]).unwrap();
        read_sizes(&tree, &AtomicBool::new(true)).unwrap();
        assert_eq!(tree.size(NodeId::ROOT), Some(7));
    }
}
