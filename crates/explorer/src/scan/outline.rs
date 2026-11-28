//! The directory listings needed to browse a folder before its recursive scan lands.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use ignore::IncrementalIgnore;

use super::{ALWAYS_SKIPPED, ScanError, ScanOptions, entry_kind, rules};
use crate::tree::RawTree;
use crate::{Kind, SavedFolder, Tree};

/// A partial tree grown by reading one directory at a time. The recursive
/// indexer runs independently. Only directories discovered in a listing
/// can be opened, using the same root-relative ignore rules as the indexer.
#[derive(Debug)]
pub struct DirectoryOutline {
    root: PathBuf,
    matcher: IncrementalIgnore,
    raw: RawTree,
    unread: HashMap<PathBuf, u32>,
}

impl DirectoryOutline {
    /// Lists the canonical root and the directories needed for saved open
    /// rows and individually checked files. Parents precede their children.
    /// Whole-folder selections require a full scan to restore.
    ///
    /// # Errors
    ///
    /// Returns the root's read error, cancellation, or a tree limit.
    pub fn open(
        root: &Path,
        options: ScanOptions,
        saved: Option<&SavedFolder>,
        cancel: &AtomicBool,
    ) -> Result<(Self, Tree), ScanError> {
        let name = root
            .file_name()
            .unwrap_or(root.as_os_str())
            .to_string_lossy();
        let mut outline = Self {
            root: root.to_owned(),
            matcher: rules(root, options).build_matchers().pop().ok_or_else(|| {
                ScanError::Root(std::io::Error::other("no matcher for the scan root"))
            })?,
            raw: RawTree::new(&name, false)?,
            unread: HashMap::from([(root.to_owned(), 0)]),
        };
        outline.list(root, cancel)?;
        if let Some(saved) = saved {
            let mut dirs = BTreeSet::new();
            let open = saved.open.iter().map(Path::new);
            let parents = saved
                .selection
                .files
                .iter()
                .filter_map(|file| Path::new(file).parent());
            for path in open.chain(parents) {
                for ancestor in path.ancestors().filter(|part| !part.as_os_str().is_empty()) {
                    dirs.insert(ancestor);
                }
            }
            for dir in dirs {
                outline.list(&root.join(dir), cancel)?;
            }
        }
        let tree = outline.snapshot()?;
        Ok((outline, tree))
    }

    /// Reads the direct children of a discovered directory, once, and
    /// returns the tree so far in its normal sort order. A directory that
    /// disappeared or became inaccessible shows as unreadable. After an
    /// error the caller must discard this outline.
    ///
    /// # Errors
    ///
    /// Returns the root's read error, cancellation, or a tree limit.
    pub fn read(&mut self, dir: &Path, cancel: &AtomicBool) -> Result<Option<Tree>, ScanError> {
        self.list(dir, cancel)?.then(|| self.snapshot()).transpose()
    }

    fn list(&mut self, dir: &Path, cancel: &AtomicBool) -> Result<bool, ScanError> {
        let Some(parent) = self.unread.remove(dir) else {
            return Ok(false);
        };
        if cancel.load(Ordering::Relaxed) {
            return Err(ScanError::Cancelled);
        }
        // A directory may have been replaced with a link since its row was drawn.
        let listing = crate::canonical(dir).and_then(|found| {
            if found != dir {
                return Err(std::io::Error::other("directory was replaced by a link"));
            }
            fs::read_dir(dir)
        });
        match listing {
            Err(err) if parent == 0 => return Err(ScanError::Root(err)),
            Err(_) => self.raw.set_kind(parent, Kind::Unreadable),
            Ok(entries) => {
                for entry in entries {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(ScanError::Cancelled);
                    }
                    let Ok(entry) = entry else { continue };
                    let name = entry.file_name();
                    if ALWAYS_SKIPPED.iter().any(|&skip| name == skip) {
                        continue;
                    }
                    let Some(kind) = entry.file_type().ok().and_then(entry_kind) else {
                        continue;
                    };
                    let path = entry.path();
                    let relative = path
                        .strip_prefix(&self.root)
                        .map_err(|err| ScanError::Root(std::io::Error::other(err)))?;
                    if self
                        .matcher
                        .matched(relative, kind == Kind::Dir)
                        .is_ignore()
                    {
                        continue;
                    }
                    let id = self.raw.push(parent, &name.to_string_lossy(), kind)?;
                    if kind == Kind::Dir {
                        self.unread.insert(path, id);
                    }
                }
            }
        }
        Ok(true)
    }

    fn snapshot(&self) -> Result<Tree, ScanError> {
        let mut tree = self.raw.clone().build(self.root.clone())?;
        tree.mark_partial();
        Ok(tree)
    }
}
