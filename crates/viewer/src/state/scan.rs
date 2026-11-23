//! Opening and rescanning the folder, off the UI thread.
//!
//! Opening a folder clears the view and shows the tree as soon as its names
//! are read; sizes follow. Refresh and Show ignored rescan the same folder
//! while the old tree stays on screen, read the sizes too, then swap the
//! trees, carrying checks, open folders and the stage across by path.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use explorer::{Explorer, ScanError, ScanOptions, Tree, read_sizes, scan};
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
    pub fn open_folder(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.root = Some(root.clone());
        self.explorer = None;
        self.tree_replaced();
        self.current = None;
        self.focus = None;
        self.selected = 0;
        self.start_scan(root, ScanKind::Open, cx);
    }

    /// Rescans the folder with the current rules, keeping the tree on screen
    /// until the new one is ready. Does nothing for a tree with no folder.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        if self.explorer.is_some() {
            self.start_scan(root, ScanKind::Refresh, cx);
        } else {
            self.open_folder(root, cx);
        }
    }

    /// Shows or hides the files the ignore rules leave out, by rescanning.
    pub fn toggle_show_ignored(&mut self, cx: &mut Context<Self>) {
        self.show_ignored = !self.show_ignored;
        self.refresh(cx);
        cx.notify();
    }

    fn start_scan(&mut self, root: PathBuf, kind: ScanKind, cx: &mut Context<Self>) {
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
        match scanned {
            Ok(tree) => {
                self.show_tree(Arc::clone(&tree));
                cx.notify();
                Some(tree)
            }
            Err(ScanError::Cancelled) => None,
            Err(err) => {
                self.fail(&err, cx);
                None
            }
        }
    }

    fn finish_sizes(&mut self, generation: u64, cx: &mut Context<Self>) {
        if generation == self.scan_generation
            && let Some(explorer) = &mut self.explorer
        {
            explorer.sizes_loaded();
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
            Err(err) => return self.fail(&err, cx),
        };
        let Some(explorer) = self.explorer.as_mut() else {
            self.show_tree(tree);
            return cx.notify();
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
        self.rows_replaced(cx);
    }

    /// The folder itself could not be read: show the system's message.
    fn fail(&mut self, err: &ScanError, cx: &mut Context<Self>) {
        self.explorer = None;
        self.tree_replaced();
        self.current = None;
        self.focus = None;
        self.selected = 0;
        self.status = ScanStatus::Failed(err.to_string().into());
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
