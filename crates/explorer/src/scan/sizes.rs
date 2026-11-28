//! [`read_sizes`]: live sizes, batched by directory on indexed macOS SMB mounts.
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

/// Measures every file of each folder of the tree that has no sizes yet
/// and stores the sizes in it, so [`Tree::size`] answers from then on. Call
/// it off the UI thread, after [`super::scan`]. A folder that already has
/// sizes, such as one from [`Tree::from_paths_with_sizes`] or one read
/// before through another tree that holds it, is not measured again; with
/// every folder sized it returns at once.
///
/// A file's size is its `lstat` size: a symlink counts as the link itself.
/// A file that cannot be measured, for example because it was deleted
/// after the scan, counts as 0.
///
/// # Errors
///
/// Returns [`ScanError::Cancelled`] on cancellation and an I/O error if a
/// bulk directory read fails. Neither publishes a partial measurement.
///
/// # Panics
///
/// Propagates a size worker panic.
pub fn read_sizes(tree: &Tree, cancel: &AtomicBool) -> Result<(), ScanError> {
    let unmeasured = tree.unsized_ranges();
    if unmeasured.is_empty() {
        return Ok(());
    }
    // Folders with at least one file. Files come after folders among a
    // folder's children, so the last child tells.
    let folders: Vec<NodeId> = unmeasured
        .iter()
        .cloned()
        .flatten()
        .map(NodeId::from_index)
        .filter(|&id| {
            tree.children(id)
                .last()
                .is_some_and(|&last| tree.is_file(last))
        })
        .collect();
    let sizes: Vec<AtomicU64> = (0..tree.node_count()).map(|_| AtomicU64::new(0)).collect();
    #[cfg(target_os = "macos")]
    let bulk = folders.iter().any(|&id| tree.size_cache(id).is_some());
    #[cfg(not(target_os = "macos"))]
    let bulk = false;
    let batch_size = if bulk { 1 } else { BATCH };
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..threads().min(folders.len()) {
            workers.push(scope.spawn(|| -> Result<(), ScanError> {
                while !cancel.load(Ordering::Relaxed) {
                    let start = next.fetch_add(batch_size, Ordering::Relaxed);
                    if start >= folders.len() {
                        break;
                    }
                    for &folder in &folders[start..folders.len().min(start + batch_size)] {
                        #[cfg(target_os = "macos")]
                        if tree.size_cache(folder).is_some() {
                            measure_smb(tree, folder, &sizes, cancel)?;
                            continue;
                        }
                        measure(tree, folder, &sizes, cancel);
                    }
                }
                Ok(())
            }));
        }
        for worker in workers {
            worker.join().expect("size worker panicked")?;
        }
        Ok::<(), ScanError>(())
    })?;
    if cancel.load(Ordering::Relaxed) {
        return Err(ScanError::Cancelled);
    }
    let sizes: Vec<u64> = sizes.into_iter().map(AtomicU64::into_inner).collect();
    tree.set_sizes(&sizes);
    #[cfg(target_os = "macos")]
    for range in unmeasured {
        let top = NodeId::from_index(range.start);
        if let Some(cache) = tree.size_cache(top) {
            cache.store(tree, top, cancel);
        }
    }
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

/// The SMB directory response carries data-fork lengths for an entire batch.
#[cfg(target_os = "macos")]
fn measure_smb(
    tree: &Tree,
    folder: NodeId,
    sizes: &[AtomicU64],
    cancel: &AtomicBool,
) -> Result<(), ScanError> {
    use rustix::fs::{AtFlags, Mode, OFlags, open, statat};
    use std::collections::HashMap;
    use std::ffi::OsStr;
    use std::os::fd::AsFd as _;
    use std::os::unix::ffi::OsStrExt as _;

    let path = tree.abs_path(folder);
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW_ANY;
    let fd = match open(&path, flags, Mode::empty()) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT | rustix::io::Errno::NOTDIR | rustix::io::Errno::LOOP) => {
            return Ok(());
        }
        Err(err) => {
            return Err(ScanError::Root(std::io::Error::other(format!(
                "{}: {err}",
                path.display()
            ))));
        }
    };
    let wanted: HashMap<_, _> = tree
        .children(folder)
        .iter()
        .copied()
        .filter(|&id| tree.is_file(id))
        .map(|id| (tree.name(id).as_bytes(), id))
        .collect();
    let mut buffer = vec![0; 64 * 1024];
    while !cancel.load(Ordering::Relaxed) {
        let count = dfdu_sys::read_bulk(fd.as_fd(), &mut buffer).map_err(ScanError::Root)?;
        if count == 0 {
            break;
        }
        let mut offset = 0;
        for _ in 0..count {
            let (entry, next) =
                dfdu_sys::parse_bulk_record(&buffer, offset).map_err(ScanError::Root)?;
            offset = next;
            let Some(&id) = wanted.get(entry.name) else {
                continue;
            };
            // Link lengths and attributes omitted by the server need an individual
            // no-follow query, with the same missing-file semantics as local scans.
            let size = if entry.obj_type == Some(dfdu_sys::VREG) && entry.error == 0 {
                entry.size.and_then(|n| u64::try_from(n).ok())
            } else {
                None
            };
            let bytes = size
                .or_else(|| {
                    statat(
                        &fd,
                        OsStr::from_bytes(entry.name),
                        AtFlags::SYMLINK_NOFOLLOW,
                    )
                    .ok()
                    .and_then(|s| u64::try_from(s.st_size).ok())
                })
                .unwrap_or(0);
            sizes[id.index()].store(bytes, Ordering::Relaxed);
        }
    }
    Ok(())
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
    fn only_folders_without_sizes_are_measured() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("new.txt"), "12345").unwrap();
        let scanned = scan(dir.path(), ScanOptions::default(), &AtomicBool::new(false)).unwrap();
        let known = Tree::from_paths_at(std::path::Path::new("/nowhere"), &[("a", 7)]).unwrap();
        let joined = Tree::join(&[(&known, NodeId::ROOT), (&scanned, NodeId::ROOT)]).unwrap();
        assert!(!joined.has_sizes());
        assert_eq!(joined.size(NodeId::ROOT), None);
        read_sizes(&joined, &AtomicBool::new(false)).unwrap();
        assert_eq!(joined.size(NodeId::ROOT), Some(12));
        // The scan shares the sizes read through the joined tree.
        assert_eq!(scanned.size(NodeId::ROOT), Some(5));
    }

    #[test]
    fn a_tree_with_sizes_is_left_alone() {
        let tree = Tree::from_paths_with_sizes(&[("a", 7)]).unwrap();
        read_sizes(&tree, &AtomicBool::new(true)).unwrap();
        assert_eq!(tree.size(NodeId::ROOT), Some(7));
    }
}
