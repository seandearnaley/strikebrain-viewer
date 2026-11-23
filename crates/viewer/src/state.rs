//! The application state: one entity that both panes observe.
//!
//! [`AppState`] owns the explorer engine, the file on the stage
//! (`current`), the keyboard focus row and the folder scan. Views read it
//! in `render` and change it only through its methods, each of which
//! notifies once.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use explorer::{Explorer, NodeId, ScanError, ScanOptions, Tree, read_sizes, scan};
use gpui_kit::{
    AppContext as _, Context, PathPromptOptions, Result, SharedString, Task, TaskExt as _,
};

/// A scan that takes longer than this shows the progress bar.
const SLOW_SCAN: Duration = Duration::from_millis(200);

/// What the app shows when it starts.
#[derive(Debug)]
pub enum Launch {
    /// No folder: the explorer offers "Open Folder".
    Empty,
    /// Scan this folder.
    Folder(PathBuf),
    /// Show a tree that is already built, such as a `Tree::from_paths`
    /// fixture. Nothing touches the disk.
    Tree(Arc<Tree>),
}

/// Where the folder scan stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanStatus {
    /// No scan is running.
    Idle,
    /// A scan is running; `slow` once it has taken longer than 200 ms.
    Scanning {
        /// Whether to show the progress bar.
        slow: bool,
    },
    /// The folder itself could not be read: the operating system's message.
    Failed(SharedString),
}

/// A keyboard movement over the rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// This many rows up, stopping at the first.
    Up(usize),
    /// This many rows down, stopping at the last.
    Down(usize),
    /// The first row.
    First,
    /// The last row.
    Last,
}

/// The application state. See the module docs.
pub struct AppState {
    /// The folder last asked for, as given.
    root: Option<PathBuf>,
    explorer: Option<Explorer>,
    status: ScanStatus,
    /// The file on the stage.
    current: Option<NodeId>,
    /// The keyboard focus row, by node so it survives row changes.
    focus: Option<NodeId>,
    /// Where `focus` was last seen in the rows; checked before use.
    focus_row: usize,
    presenting: bool,
    /// Bumped by every scan; a result from an older one is dropped.
    scan_generation: u64,
    /// Stops the running scan's worker threads.
    scan_cancel: Arc<AtomicBool>,
    /// The running scan. Replacing it drops the previous one.
    scan_task: Option<Task<Result<()>>>,
    /// Shows the progress bar once the scan is slow.
    slow_task: Option<Task<Result<()>>>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("root", &self.root)
            .field("explorer", &self.explorer)
            .field("status", &self.status)
            .field("current", &self.current)
            .field("focus", &self.focus)
            .field("presenting", &self.presenting)
            .field("scan_generation", &self.scan_generation)
            .finish_non_exhaustive()
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        self.scan_cancel.store(true, Ordering::Relaxed);
    }
}

impl AppState {
    /// Creates the state and starts whatever `launch` asks for.
    pub fn new(launch: Launch, cx: &mut Context<Self>) -> Self {
        let mut state = Self {
            root: None,
            explorer: None,
            status: ScanStatus::Idle,
            current: None,
            focus: None,
            focus_row: 0,
            presenting: false,
            scan_generation: 0,
            scan_cancel: Arc::new(AtomicBool::new(false)),
            scan_task: None,
            slow_task: None,
        };
        match launch {
            Launch::Empty => {}
            Launch::Folder(root) => state.open_folder(root, cx),
            Launch::Tree(tree) => state.show_tree(tree),
        }
        state
    }

    // Reading

    /// The explorer engine, once a tree is on screen.
    #[must_use]
    pub fn explorer(&self) -> Option<&Explorer> {
        self.explorer.as_ref()
    }

    /// Where the folder scan stands.
    #[must_use]
    pub fn status(&self) -> &ScanStatus {
        &self.status
    }

    /// Whether no folder is open or being opened: the "Open Folder" state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.explorer.is_none() && self.status == ScanStatus::Idle
    }

    /// The file on the stage.
    #[must_use]
    pub fn current(&self) -> Option<NodeId> {
        self.current
    }

    /// The keyboard focus row's node.
    #[must_use]
    pub fn focus(&self) -> Option<NodeId> {
        self.focus
    }

    /// The keyboard focus row's index in the rows. Linear in the rows only
    /// when the rows changed since focus last moved.
    #[must_use]
    pub fn focus_index(&self) -> Option<usize> {
        let (explorer, focus) = (self.explorer.as_ref()?, self.focus?);
        if explorer.rows().get(self.focus_row) == Some(&focus) {
            Some(self.focus_row)
        } else {
            explorer.row_of(focus)
        }
    }

    /// Whether the presentation is showing. Always false until milestone 4.
    #[must_use]
    pub fn is_presenting(&self) -> bool {
        self.presenting
    }

    /// The name the window shows: the open folder's.
    #[must_use]
    pub fn folder_name(&self) -> Option<SharedString> {
        if let Some(explorer) = &self.explorer {
            return Some(explorer.tree().name(NodeId::ROOT).to_owned().into());
        }
        let root = self.root.as_ref()?;
        let name = root.file_name().unwrap_or(root.as_os_str());
        Some(name.to_string_lossy().into_owned().into())
    }

    /// The name of a node, for the stage.
    #[must_use]
    pub fn name(&self, id: NodeId) -> Option<&str> {
        Some(self.explorer.as_ref()?.tree().name(id))
    }

    // Opening

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
        self.scan_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.scan_cancel = Arc::clone(&cancel);
        self.scan_generation += 1;
        let generation = self.scan_generation;

        self.root = Some(root.clone());
        self.explorer = None;
        self.current = None;
        self.focus = None;
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
                .background_spawn(
                    async move { scan(&root, ScanOptions::default(), &worker_cancel) },
                )
                .await;
            let shown = this.update(cx, |this, cx| this.finish_scan(generation, scanned, cx))?;
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
    fn finish_scan(
        &mut self,
        generation: u64,
        scanned: Result<Tree, ScanError>,
        cx: &mut Context<Self>,
    ) -> Option<Arc<Tree>> {
        if generation != self.scan_generation {
            return None;
        }
        self.slow_task = None;
        match scanned {
            Ok(tree) => {
                let tree = Arc::new(tree);
                self.show_tree(Arc::clone(&tree));
                cx.notify();
                Some(tree)
            }
            Err(ScanError::Cancelled) => None,
            Err(err) => {
                self.status = ScanStatus::Failed(err.to_string().into());
                cx.notify();
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

    fn show_tree(&mut self, tree: Arc<Tree>) {
        let explorer = Explorer::new(tree);
        self.focus = explorer.rows().first().copied();
        self.focus_row = 0;
        self.explorer = Some(explorer);
        self.current = None;
        self.status = ScanStatus::Idle;
    }

    // Rows

    /// Moves keyboard focus to `id`, if it is a row.
    pub fn focus_node(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(row) = self.explorer.as_ref().and_then(|e| e.row_of(id)) else {
            return;
        };
        self.set_focus(id, row);
        cx.notify();
    }

    /// Moves keyboard focus; returns the new focus row.
    pub fn move_focus(&mut self, motion: Motion, cx: &mut Context<Self>) -> Option<usize> {
        let explorer = self.explorer.as_ref()?;
        let last = explorer.rows().len().checked_sub(1)?;
        let from = self.focus_index().unwrap_or(0);
        let to = match motion {
            Motion::Up(n) => from.saturating_sub(n),
            Motion::Down(n) => from.saturating_add(n).min(last),
            Motion::First => 0,
            Motion::Last => last,
        };
        let id = explorer.rows()[to];
        self.set_focus(id, to);
        cx.notify();
        Some(to)
    }

    /// → on the focus row: opens a closed folder, or focuses the first
    /// child of an open one. Returns the new focus row.
    pub fn expand(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        let row = self.focus_index()?;
        let explorer = self.explorer.as_mut()?;
        let id = explorer.rows()[row];
        if !explorer.can_open(id) {
            return Some(row);
        }
        if !explorer.is_open(id) {
            explorer.set_open(id, true);
            cx.notify();
            return Some(row);
        }
        let child = explorer.rows().get(row + 1).copied();
        match child {
            Some(child) if explorer.tree().parent(child) == Some(id) => {
                self.set_focus(child, row + 1);
                cx.notify();
                Some(row + 1)
            }
            _ => Some(row),
        }
    }

    /// ← on the focus row: closes an open folder, or focuses the parent
    /// folder. Returns the new focus row.
    pub fn collapse(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        let row = self.focus_index()?;
        let explorer = self.explorer.as_mut()?;
        let id = explorer.rows()[row];
        if explorer.can_open(id) && explorer.is_open(id) {
            explorer.set_open(id, false);
            cx.notify();
            return Some(row);
        }
        let parent = explorer.tree().parent(id).filter(|&p| p != NodeId::ROOT);
        let Some((parent, parent_row)) =
            parent.and_then(|parent| Some((parent, explorer.row_of(parent)?)))
        else {
            return Some(row);
        };
        self.set_focus(parent, parent_row);
        cx.notify();
        Some(parent_row)
    }

    /// Opens or closes a folder: its chevron. Focus inside a folder that
    /// closes moves to the folder.
    pub fn toggle_folder(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        explorer.toggle_folder(id);
        if !explorer.is_open(id)
            && let Some(focus) = self.focus
            && focus != id
            && explorer.tree().subtree(id).contains(&focus.index())
        {
            self.focus = Some(id);
        }
        cx.notify();
    }

    // Checks and the stage

    /// Toggles a checkbox and applies the rules for `current`: checking a
    /// single file shows it; unchecking the file on the stage moves the stage
    /// to the next checked file, else the previous one, else nothing; an
    /// empty stage takes the first checked file.
    pub fn toggle_check(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        let current_was_checked = self.current.is_some_and(|c| explorer.is_selected(c));
        explorer.toggle_check(id);

        if explorer.tree().is_file(id) && explorer.is_selected(id) {
            self.current = Some(id);
        } else if let Some(current) = self.current
            && current_was_checked
            && !explorer.is_selected(current)
        {
            self.current = explorer
                .gallery_next(current)
                .or_else(|| explorer.gallery_previous(current));
        }
        if self.current.is_none() {
            self.current = explorer.gallery().next();
        }
        cx.notify();
    }

    /// Shows `id` on the stage if it is a file, checked or not. Does
    /// nothing on a folder.
    pub fn show(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(explorer) = &self.explorer else {
            return;
        };
        if explorer.tree().is_file(id) {
            self.current = Some(id);
            cx.notify();
        }
    }

    /// Space on the focus row.
    pub fn toggle_focus_check(&mut self, cx: &mut Context<Self>) {
        if let Some(focus) = self.focus {
            self.toggle_check(focus, cx);
        }
    }

    /// Enter on the focus row.
    pub fn show_focus(&mut self, cx: &mut Context<Self>) {
        if let Some(focus) = self.focus {
            self.show(focus, cx);
        }
    }

    fn set_focus(&mut self, id: NodeId, row: usize) {
        self.focus = Some(id);
        self.focus_row = row;
    }
}
