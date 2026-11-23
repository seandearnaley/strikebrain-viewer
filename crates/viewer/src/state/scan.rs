//! Scanning the folders, off the UI thread, each on its own.
//!
//! A folder's first scan puts its tree on screen as soon as its names are
//! read; sizes follow. A rescan (Refresh, Show ignored, a change on disk)
//! keeps the tree on screen, reads the sizes too when the tree on screen
//! has them, then swaps that folder's part of the tree, carrying checks,
//! open folders and the stage across by path. A folder that cannot be read
//! shows as one unreadable row with the system's message, keeping the view
//! it had until a scan can read it again.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use explorer::{
    Explorer, IdMap, Kind, NodeId, SavedFolder, ScanError, ScanOptions, SortKey, Tree, TreeError,
    read_sizes, scan,
};
use gpui_kit::{AppContext as _, Context};

use super::AppState;

/// A scan that takes longer than this shows the progress bar.
const SLOW_SCAN: Duration = Duration::from_millis(200);

impl AppState {
    /// Whether scans list the files the ignore rules leave out.
    #[must_use]
    pub fn shows_ignored(&self) -> bool {
        self.show_ignored
    }

    /// Rescans every folder with the current rules, keeping the trees on
    /// screen until the new ones are ready. Does nothing for a tree with no
    /// folder. A folder not followed on disk because its watcher could not
    /// start is followed again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<u64> = self.folders.iter().map(|folder| folder.key).collect();
        for key in keys {
            self.refresh_folder(key, cx);
        }
    }

    /// Rescans one folder, following it again if its watcher failed.
    pub(super) fn refresh_folder(&mut self, key: u64, cx: &mut Context<Self>) {
        if self
            .folder(key)
            .is_some_and(|folder| folder.watch_error.is_some())
        {
            self.follow(key, cx);
        }
        self.start_scan(key, cx);
    }

    /// Shows or hides the files the ignore rules leave out, by rescanning
    /// every folder. Does nothing for a tree with no folder.
    pub fn toggle_show_ignored(&mut self, cx: &mut Context<Self>) {
        if self.folders.is_empty() {
            return;
        }
        self.show_ignored = !self.show_ignored;
        self.refresh(cx);
        cx.notify();
    }

    /// Scans the folder `key` on the background executor and puts its tree
    /// on screen, superseding its running scan.
    pub(super) fn start_scan(&mut self, key: u64, cx: &mut Context<Self>) {
        let options = ScanOptions {
            show_ignored: self.show_ignored,
        };
        self.scans += 1;
        let generation = self.scans;
        // Sizes first when the tree on screen has them, so the Size column
        // never blanks.
        let sizes_first = self.folder(key).is_some_and(|folder| {
            let top = self.folder_node(&folder.path);
            let tree = self.explorer.as_ref().map(Explorer::tree);
            top.zip(tree)
                .is_some_and(|(top, tree)| tree.size(top).is_some())
        });
        let Some(folder) = self.folder_mut(key) else {
            return;
        };
        folder.rescan_pending = false;
        folder.cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        folder.cancel = Arc::clone(&cancel);
        folder.generation = generation;
        folder.scanning = Some(false);
        let root = folder.path.clone();

        // Each task ends early with an error only when the state is gone.
        let timer = cx.background_executor().timer(SLOW_SCAN);
        folder.slow_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| this.mark_slow(key, generation, cx))
        }));
        folder.scan_task = Some(cx.spawn(async move |this, cx| {
            let worker_cancel = Arc::clone(&cancel);
            let mut scanned = cx
                .background_spawn(async move { scan(&root, options, &worker_cancel) })
                .await
                .map(Arc::new);
            if sizes_first && let Ok(tree) = &scanned {
                // A cancelled read means a newer scan has replaced this one.
                let (sized, sizes_cancel) = (Arc::clone(tree), Arc::clone(&cancel));
                if let Err(err) = cx
                    .background_spawn(async move { read_sizes(&sized, &sizes_cancel) })
                    .await
                {
                    scanned = Err(err);
                }
            }
            let shown = this.update(cx, |this, cx| {
                this.finish_scan(key, generation, scanned, cx)
            })?;
            let Some(tree) = shown else {
                return Ok(());
            };
            // A cancelled read leaves the folder's Size cells blank; a newer
            // scan has replaced this tree by then.
            let sized = cx
                .background_spawn(async move { read_sizes(&tree, &cancel) })
                .await;
            if sized.is_ok() {
                this.update(cx, |this, cx| this.finish_sizes(key, generation, cx))?;
            }
            Ok(())
        }));
        cx.notify();
    }

    fn mark_slow(&mut self, key: u64, generation: u64, cx: &mut Context<Self>) {
        if let Some(folder) = self.folder_mut(key)
            && folder.generation == generation
            && folder.scanning.is_some()
        {
            folder.scanning = Some(true);
            cx.notify();
        }
    }

    /// Puts a finished scan of the folder `key` on screen, if it is still
    /// that folder's latest. Returns the tree when its sizes are still to be
    /// read.
    pub(super) fn finish_scan(
        &mut self,
        key: u64,
        generation: u64,
        scanned: Result<Arc<Tree>, ScanError>,
        cx: &mut Context<Self>,
    ) -> Option<Arc<Tree>> {
        let folder = self
            .folder_mut(key)
            .filter(|folder| folder.generation == generation)?;
        folder.scanning = None;
        folder.slow_task = None;
        let old = folder.path.clone();
        let (tree, error) = match scanned {
            Ok(tree) => (tree, None),
            Err(ScanError::Cancelled) => return None,
            Err(err) => (
                Arc::new(Tree::unreadable(&old)),
                Some(os_message(&err.to_string()).to_owned()),
            ),
        };
        let path = tree.root_path().to_path_buf();
        // A first scan of a folder a tree on screen lists already: its files
        // are there.
        let first = !folder.shown;
        if first && self.listed(&path).is_some() {
            let asked = self.take_folder(key);
            self.settle_asked_inside(asked, cx);
            self.landed(cx);
            return None;
        }
        if error.is_none() {
            // Folders this scan lists leave, with what was asked inside
            // them; the join takes in those on screen, with their checks
            // and open folders.
            let lists = |other: &super::Folder| {
                other.key != key
                    && tree
                        .find_abs(&other.path)
                        .is_some_and(|node| tree.kind(node).is_folder())
            };
            let taken: Vec<u64> = self
                .folders
                .iter()
                .filter(|other| lists(other))
                .map(|other| other.key)
                .collect();
            for each in taken {
                let asked = self.take_folder(each);
                if let Some(folder) = self.folder_mut(key) {
                    folder.asked_inside.extend(asked);
                }
            }
        }
        let before = self.before();
        let joined = self.join(key, &old, &tree, error.is_some());
        let (map, current) = match joined {
            Ok(joined) => joined,
            Err(err) => {
                // Past the tree's limits: the folder stays as it was.
                if let Some(folder) = self.folder_mut(key) {
                    folder.error = Some(err.to_string().into());
                }
                cx.notify();
                return None;
            }
        };
        self.current = current;
        self.focus = self.focus.and_then(|focus| map.get(focus));
        self.tree_replaced();
        let folder = self.folder_mut(key)?;
        folder.shown = true;
        folder.path.clone_from(&path);
        let readable = error.is_none();
        folder.error = error.map(Into::into);
        let waiting = if readable {
            folder.waiting.take()
        } else {
            None
        };
        if let (Some(saved), Some(explorer)) = (waiting, self.explorer.as_mut()) {
            explorer.restore_folder(&SavedFolder {
                path: path.clone(),
                ..saved
            });
        }
        self.selection_changed();
        let settling = self.restore.is_some() || self.restore_top.is_some();
        match self.folder_node(&path) {
            // Added while the app runs: its row is the focus row, on screen.
            Some(top) if first && !settling => {
                let row = self.explorer.as_ref().and_then(|e| e.row_of(top));
                if let Some(row) = row {
                    self.set_focus(top, row);
                }
                self.rows_replaced(cx);
            }
            _ => self.landed_keeping(before, &map, cx),
        }
        let asked = self
            .folder_mut(key)
            .map_or_else(Vec::new, |folder| std::mem::take(&mut folder.asked_inside));
        self.settle_asked_inside(asked, cx);
        self.scan_settled(key, cx);
        (readable && !tree.has_sizes()).then_some(tree)
    }

    /// Joins `tree`, the folder `key` last seen at `old`, into the tree on
    /// screen: in place of its tree there, or as a new top-level folder in
    /// its place among the folders. A folder that could no longer be read
    /// keeps the view it had, to restore when it can be again.
    fn join(
        &mut self,
        key: u64,
        old: &Path,
        tree: &Tree,
        unreadable: bool,
    ) -> Result<(IdMap, Option<NodeId>), TreeError> {
        let at = self
            .folders
            .iter()
            .position(|folder| folder.key == key)
            .unwrap_or(self.folders.len());
        let shown_after = self.folders[at..]
            .iter()
            .skip(1)
            .filter(|folder| folder.shown)
            .count();
        let fresh = self.explorer.is_none().then(|| self.new_explorer());
        let (current, folders) = (self.current, &mut self.folders);
        let explorer = self
            .explorer
            .get_or_insert_with(|| fresh.unwrap_or_else(Explorer::empty));
        let Some(top) = explorer.folder_at(old) else {
            let index = explorer.tree().folders().len().saturating_sub(shown_after);
            return explorer.add_folder(tree, index, current);
        };
        if unreadable
            && explorer.tree().kind(top) == Kind::Dir
            && let Some(folder) = folders.get_mut(at)
            && folder.waiting.is_none()
        {
            folder.waiting = Some(explorer.snapshot().folder(top));
        }
        explorer.replace_folder(top, tree, current)
    }

    /// Takes the folder `key` out of the list, stopping its scan and its
    /// watcher, and gives back what was asked for inside it meanwhile.
    fn take_folder(&mut self, key: u64) -> Vec<(PathBuf, Option<SavedFolder>)> {
        let Some(at) = self.folders.iter().position(|folder| folder.key == key) else {
            return Vec::new();
        };
        std::mem::take(&mut self.folders.remove(at).asked_inside)
    }

    /// The sizes of the folder `key` on screen are in. Under a Size sort
    /// they reorder the rows, which keeps the focus row on screen as any
    /// command that rebuilds the rows does.
    fn finish_sizes(&mut self, key: u64, generation: u64, cx: &mut Context<Self>) {
        if self
            .folder(key)
            .is_none_or(|folder| folder.generation != generation)
        {
            return;
        }
        let Some(explorer) = &mut self.explorer else {
            return;
        };
        explorer.sizes_loaded();
        let sorted_by_size = explorer.sort().key == SortKey::Size;
        if self.restore_top.is_some() {
            self.landed(cx);
        } else if sorted_by_size {
            self.rows_replaced(cx);
        } else {
            cx.notify();
        }
    }

    /// A scan of the folder `key` landed or failed: if its folder changed
    /// on disk meanwhile, rescan it once.
    fn scan_settled(&mut self, key: u64, cx: &mut Context<Self>) {
        let pending = self
            .folder_mut(key)
            .is_some_and(|folder| std::mem::take(&mut folder.rescan_pending));
        if pending {
            self.refresh_folder(key, cx);
        }
    }

    /// Puts a tree built in memory on screen, as it is, with the filter
    /// asked for.
    pub(super) fn show_tree(&mut self, tree: Arc<Tree>) {
        let mut explorer = Explorer::new(tree);
        if self.filter_asked() {
            // Synchronous: this runs once.
            explorer.set_filter(&self.query, self.kind);
        }
        self.focus = explorer.rows().first().copied();
        self.focus_row = 0;
        self.explorer = Some(explorer);
        self.tree_replaced();
        self.current = None;
        self.selected = 0;
    }
}

/// The operating system's message alone: Rust's `io::Error` ends it with
/// " (os error N)", which the interface does not show.
pub(super) fn os_message(text: &str) -> &str {
    text.rsplit_once(" (os error ")
        .filter(|(_, code)| {
            code.strip_suffix(')')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .map_or(text, |(message, _)| message)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use explorer::{ScanOptions, scan};
    use gpui_kit::{AppContext as _, TestAppContext};

    use super::os_message;
    use crate::state::{AppState, Launch, StateEvent};

    #[gpui_kit::test]
    fn folders_keep_the_order_they_were_added_in_whichever_scan_lands_first(
        cx: &mut TestAppContext,
    ) {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        std::fs::write(a.path().join("a.txt"), "a").unwrap();
        std::fs::write(b.path().join("b.txt"), "b").unwrap();
        let launch = Launch::Folders(vec![a.path().to_owned(), b.path().to_owned()]);
        let state = cx.new(|cx| AppState::new(launch, cx));
        let tops: std::rc::Rc<std::cell::RefCell<Vec<usize>>> = std::rc::Rc::default();
        let seen = std::rc::Rc::clone(&tops);
        let _events = cx.update(|cx| {
            cx.subscribe(&state, move |_, event: &StateEvent, _| {
                if let StateEvent::TopRow(row) = event {
                    seen.borrow_mut().push(*row);
                }
            })
        });
        state.update(cx, |state, cx| {
            // The scans the launch started never land; these do, the
            // second folder's first.
            for folder in &mut state.folders {
                folder.scan_task = None;
            }
            for (key, dir) in [(1, &b), (0, &a)] {
                let tree = scan(dir.path(), ScanOptions::default(), &AtomicBool::new(false));
                let generation = state.folder(key).unwrap().generation;
                state.finish_scan(key, generation, tree.map(Arc::new), cx);
            }
            let tree = state.explorer().unwrap().tree();
            let names: Vec<&str> = tree.folders().map(|top| tree.name(top)).collect();
            let name = |dir: &tempfile::TempDir| {
                dir.path()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            };
            assert_eq!(names, [name(&a), name(&b)]);
            // The launch settles at the top, on the first folder's row,
            // though the second landed first and took focus then.
            assert_eq!(state.focus(), tree.folders().next());
        });
        cx.run_until_parked();
        assert_eq!(tops.borrow().last(), Some(&0));
    }

    #[test]
    fn the_os_message_loses_only_the_error_code() {
        assert_eq!(
            os_message("No such file or directory (os error 2)"),
            "No such file or directory"
        );
        assert_eq!(
            os_message("Permission denied (os error 13)"),
            "Permission denied"
        );
        // Anything else is shown as it is.
        assert_eq!(
            os_message("the folder holds too many entries"),
            "the folder holds too many entries"
        );
        assert_eq!(os_message("odd (os error x)"), "odd (os error x)");
        assert_eq!(os_message("odd (os error )"), "odd (os error )");
    }
}
