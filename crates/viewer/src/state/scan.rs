//! Opening and rescanning the folder, off the UI thread.
//!
//! Opening a folder clears the view and shows the tree as soon as its names
//! are read; sizes follow. Refresh and Show ignored rescan the same folder
//! while the old tree stays on screen, read the sizes too, then swap the
//! trees, carrying checks, open folders and the stage across by path.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use explorer::{Explorer, ScanError, ScanOptions, SortKey, Tree, read_sizes, scan};
use gpui_kit::{AppContext as _, Context, PathPromptOptions, Result, TaskExt as _};

use super::{AppState, ScanStatus};

/// A scan that takes longer than this shows the progress bar.
const SLOW_SCAN: Duration = Duration::from_millis(200);

/// Whether a scan replaces the view or refreshes the tree on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanKind {
    Open,
    Refresh,
}

impl AppState {
    /// Whether scans list the files the ignore rules leave out.
    #[must_use]
    pub fn shows_ignored(&self) -> bool {
        self.show_ignored
    }

    /// Shows the system folder picker and opens the folder chosen.
    pub fn prompt_for_folder(cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            // A cancelled picker or a dropped channel leaves the current
            // folder open; a picker that failed to open is logged.
            let Ok(paths) = chosen.await else {
                return Ok(());
            };
            let Some(root) = paths?.and_then(|paths| paths.into_iter().next()) else {
                return Ok(());
            };
            this.update(cx, |this, cx| this.open_folder(root, cx))
        })
        .detach_and_log_err(cx);
    }

    /// Scans `root` on the background executor and shows its tree. Sizes are
    /// read afterwards, also in the background. Supersedes any running scan.
    /// Until the new tree is on screen, the view on screen before is what
    /// is saved.
    pub fn open_folder(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.keep_view();
        self.root = Some(root.clone());
        self.explorer = None;
        self.tree_replaced();
        self.current = None;
        self.focus = None;
        self.selected = 0;
        self.restore = None;
        self.restore_top = None;
        self.follow(root.clone(), cx);
        self.start_scan(root, ScanKind::Open, cx);
    }

    /// Rescans the folder with the current rules, keeping the tree on screen
    /// until the new one is ready. Does nothing for a tree with no folder.
    /// With no tree on screen (the first scan runs, or the folder could not
    /// be read) it scans the folder again, and the view waiting for it, or
    /// the view it had, applies when that scan lands. A folder that is not
    /// followed on disk because its watcher could not start is followed
    /// again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if self.explorer.is_none() {
            return self.reopen(root, cx);
        }
        if self.watch_error.is_some() {
            self.follow(root.clone(), cx);
        }
        self.start_scan(root, ScanKind::Refresh, cx);
    }

    /// Scans `root` again while no tree is on screen, keeping the view that
    /// waits for it. After a failure, the view the folder had waits: named
    /// here, on the UI thread, as this happens once per Refresh of a
    /// folder that could not be read.
    fn reopen(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        if self.restore.is_none()
            && let Some((kept_root, kept)) = &self.kept
            && same_folder(kept_root, &root)
        {
            self.restore = Some(kept.clone().named());
        }
        self.follow(root.clone(), cx);
        self.start_scan(root, ScanKind::Open, cx);
    }

    /// Before the tree on screen goes: keeps its view, or the saved view
    /// still waiting for it, as the one to save. Its top row is the one the
    /// last save saw. With neither, whatever was kept before stays.
    fn keep_view(&mut self) {
        let top = self.saved_top.take();
        if let (Some(root), Some(view)) = (self.saved_root(), self.saved_view(top)) {
            self.kept = Some((root, view));
        }
    }

    /// Shows or hides the files the ignore rules leave out, by rescanning.
    /// Does nothing for a tree with no folder.
    pub fn toggle_show_ignored(&mut self, cx: &mut Context<Self>) {
        if self.root.is_none() {
            return;
        }
        self.show_ignored = !self.show_ignored;
        self.refresh(cx);
        cx.notify();
    }

    fn start_scan(&mut self, root: PathBuf, kind: ScanKind, cx: &mut Context<Self>) {
        // This scan sees every change made before it starts.
        self.rescan_pending = false;
        self.scan_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.scan_cancel = Arc::clone(&cancel);
        self.scan_generation += 1;
        let generation = self.scan_generation;
        let options = ScanOptions {
            show_ignored: self.show_ignored,
        };
        self.status = ScanStatus::Scanning { slow: false };

        // Each task ends early with an error only when the state is gone.
        let timer = cx.background_executor().timer(SLOW_SCAN);
        self.slow_task = Some(cx.spawn(async move |this, cx| {
            timer.await;
            this.update(cx, |this, cx| this.mark_slow(generation, cx))
        }));
        self.scan_task = Some(cx.spawn(async move |this, cx| {
            let worker_cancel = Arc::clone(&cancel);
            let scanned = cx
                .background_spawn(async move { scan(&root, options, &worker_cancel) })
                .await
                .map(Arc::new);
            if kind == ScanKind::Refresh {
                // Sizes first, so the Size column never blanks. A cancelled
                // read means a newer scan has replaced this one.
                let scanned = match scanned {
                    Ok(tree) => {
                        let (sized, sizes_cancel) = (Arc::clone(&tree), Arc::clone(&cancel));
                        cx.background_spawn(async move { read_sizes(&sized, &sizes_cancel) })
                            .await
                            .map(|()| tree)
                    }
                    Err(err) => Err(err),
                };
                return this.update(cx, |this, cx| this.finish_refresh(generation, scanned, cx));
            }
            let shown = this.update(cx, |this, cx| this.finish_open(generation, scanned, cx))?;
            let Some(tree) = shown else {
                return Ok(());
            };
            // A cancelled read leaves the Size column blank; a newer scan
            // has replaced this tree by then.
            let sized = cx
                .background_spawn(async move { read_sizes(&tree, &cancel) })
                .await;
            if sized.is_ok() {
                this.update(cx, |this, cx| this.finish_sizes(generation, cx))?;
            }
            Ok(())
        }));
        cx.notify();
    }

    fn mark_slow(&mut self, generation: u64, cx: &mut Context<Self>) {
        if generation == self.scan_generation
            && let ScanStatus::Scanning { slow } = &mut self.status
        {
            *slow = true;
            cx.notify();
        }
    }

    /// Shows a finished scan if it is still the latest. Returns the tree so
    /// the caller can read its sizes.
    fn finish_open(
        &mut self,
        generation: u64,
        scanned: Result<Arc<Tree>, ScanError>,
        cx: &mut Context<Self>,
    ) -> Option<Arc<Tree>> {
        if generation != self.scan_generation {
            return None;
        }
        self.slow_task = None;
        let shown = match scanned {
            Ok(tree) => {
                self.kept = None;
                self.show_tree(Arc::clone(&tree));
                self.restore_view(cx);
                cx.notify();
                Some(tree)
            }
            Err(ScanError::Cancelled) => return None,
            Err(err) => {
                self.fail(&err, cx);
                None
            }
        };
        self.scan_settled(cx);
        shown
    }

    /// The sizes of the tree on screen are in. Under a Size sort they
    /// reorder the rows, which keeps the focus row on screen as any command
    /// that rebuilds the rows does.
    fn finish_sizes(&mut self, generation: u64, cx: &mut Context<Self>) {
        if generation != self.scan_generation {
            return;
        }
        let Some(explorer) = &mut self.explorer else {
            return;
        };
        explorer.sizes_loaded();
        if let Some(top) = self.restore_top.take() {
            return self.show_saved_top(Some(&top), cx);
        }
        if explorer.sort().key == SortKey::Size {
            self.rows_replaced(cx);
        } else {
            cx.notify();
        }
    }

    /// Swaps a rescanned tree in, if it is still the latest: checks, open
    /// folders, the filter and the stage carry across by path. A file on the
    /// stage that is gone moves to the nearest checked file.
    fn finish_refresh(
        &mut self,
        generation: u64,
        scanned: Result<Arc<Tree>, ScanError>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.scan_generation {
            return;
        }
        self.slow_task = None;
        let tree = match scanned {
            Ok(tree) => tree,
            Err(ScanError::Cancelled) => return,
            Err(err) => {
                self.fail(&err, cx);
                return self.scan_settled(cx);
            }
        };
        let Some(explorer) = self.explorer.as_mut() else {
            self.kept = None;
            self.show_tree(tree);
            self.restore_view(cx);
            cx.notify();
            return self.scan_settled(cx);
        };
        let map = match self.current {
            Some(current) => {
                let (map, carried) = explorer.replace_tree_carrying(tree, current);
                self.current = carried;
                map
            }
            None => explorer.replace_tree(tree),
        };
        self.tree_replaced();
        self.focus = self.focus.and_then(|focus| map.get(focus));
        self.status = ScanStatus::Idle;
        self.selection_changed();
        // A saved top row still waiting for sizes: this tree has them. The
        // open scan's own read of them was cancelled by this one.
        match self.restore_top.take() {
            Some(top) => self.show_saved_top(Some(&top), cx),
            None => self.rows_replaced(cx),
        }
        self.scan_settled(cx);
    }

    /// A scan landed or failed and no newer one runs: if the disk changed
    /// meanwhile, rescan once.
    fn scan_settled(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.rescan_pending) {
            self.refresh(cx);
        }
    }

    /// The folder itself could not be read: show the system's message. The
    /// view it had stays the one to save.
    fn fail(&mut self, err: &ScanError, cx: &mut Context<Self>) {
        if self.explorer.is_some() {
            self.keep_view();
        }
        self.explorer = None;
        self.tree_replaced();
        self.current = None;
        self.focus = None;
        self.selected = 0;
        self.status = ScanStatus::Failed(os_message(&err.to_string()).to_owned().into());
        cx.notify();
    }

    /// Puts a new tree on screen with the filter asked for.
    pub(super) fn show_tree(&mut self, tree: Arc<Tree>) {
        let mut explorer = Explorer::new(tree);
        if self.filter_asked() {
            // Synchronous: a scan just finished, and this runs once.
            explorer.set_filter(&self.query, self.kind);
        }
        self.focus = explorer.rows().first().copied();
        self.focus_row = 0;
        self.explorer = Some(explorer);
        self.tree_replaced();
        self.current = None;
        self.selected = 0;
        self.status = ScanStatus::Idle;
    }
}

/// Whether two paths name the same folder: the same path, or the same
/// folder once links and `..` are resolved.
fn same_folder(a: &Path, b: &Path) -> bool {
    a == b || std::fs::canonicalize(a).is_ok_and(|a| std::fs::canonicalize(b).is_ok_and(|b| a == b))
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
    use super::os_message;

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
