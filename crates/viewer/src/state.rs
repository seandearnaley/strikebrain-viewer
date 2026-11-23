//! The application state: one entity that both panes observe.
//!
//! [`AppState`] owns the explorer engine, the file on the stage
//! (`current`), the keyboard focus row, the filter asked for and the folder
//! scan. Views read it in `render` and change it only through its methods,
//! each of which notifies once.
//!
//! Moving over the rows lives in `state/rows.rs`, checks and the stage in
//! `state/checks.rs`, scanning in `state/scan.rs`, filtering in
//! `state/filter.rs`, following the folder on disk in `state/watch.rs`,
//! and restoring a saved view in `state/restore.rs`.

mod checks;
mod filter;
mod restore;
mod rows;
mod scan;
mod watch;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use explorer::{Explorer, KindFilter, NodeId, Tree};
use gpui_kit::{Context, EventEmitter, Result, SharedString, Task};

use crate::persist::{FolderView, SavedFolder, SnapshotView};
use watch::Watching;

/// What the app shows when it starts.
#[derive(Debug)]
pub enum Launch {
    /// No folder: the explorer offers "Open Folder".
    Empty,
    /// Scan this folder.
    Folder(PathBuf),
    /// Show a tree that is already built, such as a `Tree::from_paths`
    /// fixture. Nothing touches the disk; refreshing does nothing.
    Tree(Arc<Tree>),
}

/// What an [`AppState`] brings back from a saved state, and whether it
/// follows its folder on disk.
#[derive(Clone, Debug, Default)]
pub struct StateOptions {
    /// A saved view of the folder `launch` opens, applied when its first
    /// scan lands. Its filter and Show ignored apply from the start.
    pub view: Option<FolderView>,
    /// A saved folder `launch` does not open: saved again as it was until
    /// another folder opens.
    pub kept: Option<SavedFolder>,
    /// Rescan when something under the folder changes on disk.
    pub watch: bool,
    /// Changes to these files never cause a rescan: the app's own state
    /// file and its temporary name, should the open folder hold them.
    pub quiet: Vec<PathBuf>,
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
pub(crate) enum Motion {
    /// This many rows up, stopping at the first.
    Up(usize),
    /// This many rows down, stopping at the last.
    Down(usize),
    /// The first row.
    First,
    /// The last row.
    Last,
}

/// A row as one frame drew it: which tree, where, and which node. Input
/// reaches a frame's listeners before the next frame is drawn, so by then
/// the rows or the whole tree may have changed; [`AppState`] acts on a drawn
/// row only while it is still that row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DrawnRow {
    tree: u64,
    ix: usize,
    id: NodeId,
}

/// What a pointer press or an assistive-technology action asks of a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowAction {
    /// A press on the row: shows a file on the stage; on a folder, nothing
    /// more than focus.
    Show,
    /// A press on the checkbox.
    ToggleCheck,
    /// A press on the chevron, or Expand or Collapse from assistive
    /// technology.
    ToggleOpen,
}

/// What [`AppState`] tells its views besides "something changed".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StateEvent {
    /// The rows were rebuilt by a command, not by a key on one row: a filter
    /// landed, folders opened or closed together, the sort or the tree
    /// changed. The tree keeps the focus row on screen.
    RowsReplaced,
    /// The stage moved to another file by its own keys. The tree scrolls
    /// that file's row into view if it is a row, and opens no folder.
    CurrentMoved,
    /// A saved view was restored: the tree scrolls this row to its top.
    TopRow(usize),
}

/// The application state. See the module docs.
pub struct AppState {
    /// The folder last asked for, as given.
    root: Option<PathBuf>,
    explorer: Option<Explorer>,
    /// Bumped whenever the tree on screen is replaced or removed, so a row
    /// drawn for an older tree is recognised. A count, not the tree's
    /// address, which a new tree can reuse.
    tree_generation: u64,
    status: ScanStatus,
    /// The file on the stage.
    current: Option<NodeId>,
    /// The keyboard focus row, by node so it survives row changes. Always a
    /// row on screen, or `None` when there are no rows.
    focus: Option<NodeId>,
    /// Where `focus` was last seen in the rows; checked before use.
    focus_row: usize,
    /// How many files are checked, kept so a frame need not count them.
    selected: usize,
    presenting: bool,
    /// The filter text asked for, as typed. The tree shows it once its
    /// background job lands.
    query: String,
    /// The kind asked for.
    kind: KindFilter,
    /// The running filter job. Replacing it drops the previous one.
    filter_task: Option<Task<Result<()>>>,
    /// Whether scans list the files the ignore rules would leave out.
    show_ignored: bool,
    /// Bumped by every scan; a result from an older one is dropped.
    scan_generation: u64,
    /// Stops the running scan's worker threads.
    scan_cancel: Arc<AtomicBool>,
    /// The running scan. Replacing it drops the previous one.
    scan_task: Option<Task<Result<()>>>,
    /// Shows the progress bar once the scan is slow.
    slow_task: Option<Task<Result<()>>>,
    /// A saved view waiting for the first scan of its folder.
    restore: Option<FolderView>,
    /// The saved top row, waiting for sizes before a Size sort settles.
    restore_top: Option<String>,
    /// What to save while no tree is on screen and no saved view waits for
    /// a scan: the saved folder this launch did not open, or the last tree
    /// when its folder could no longer be read. Dropped once a folder
    /// opens, so a folder that is away for a while loses nothing.
    kept: Option<(PathBuf, SnapshotView)>,
    /// The row at the top of the tree's scroll when the state was last
    /// saved, by path: the top row of a view kept once its tree is gone.
    saved_top: Option<String>,
    /// How to follow the folder on disk; `None` does not.
    watching: Option<Watching>,
    /// Follows the folder: its watcher and the task batching its events.
    /// Replacing it stops the watcher.
    watch_task: Option<Task<Result<()>>>,
    /// Why the folder is not followed, when its watcher failed to start.
    watch_error: Option<SharedString>,
    /// Something changed on disk while a scan was running: rescan once it
    /// lands.
    rescan_pending: bool,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("root", &self.root)
            .field("explorer", &self.explorer)
            .field("status", &self.status)
            .field("current", &self.current)
            .field("focus", &self.focus)
            .field("selected", &self.selected)
            .field("presenting", &self.presenting)
            .field("query", &self.query)
            .field("kind", &self.kind)
            .field("show_ignored", &self.show_ignored)
            .field("scan_generation", &self.scan_generation)
            .finish_non_exhaustive()
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        self.scan_cancel.store(true, Ordering::Relaxed);
    }
}

impl EventEmitter<StateEvent> for AppState {}

impl AppState {
    /// Creates the state and starts whatever `launch` asks for. Nothing is
    /// restored and the folder is not followed on disk.
    pub fn new(launch: Launch, cx: &mut Context<Self>) -> Self {
        Self::with_options(launch, StateOptions::default(), cx)
    }

    /// Creates the state, restoring and following as `options` say, and
    /// starts whatever `launch` asks for.
    pub fn with_options(launch: Launch, options: StateOptions, cx: &mut Context<Self>) -> Self {
        let mut state = Self {
            root: None,
            explorer: None,
            tree_generation: 0,
            status: ScanStatus::Idle,
            current: None,
            focus: None,
            focus_row: 0,
            selected: 0,
            presenting: false,
            query: String::new(),
            kind: KindFilter::All,
            filter_task: None,
            show_ignored: false,
            scan_generation: 0,
            scan_cancel: Arc::new(AtomicBool::new(false)),
            scan_task: None,
            slow_task: None,
            restore: None,
            restore_top: None,
            kept: options
                .kept
                .map(|kept| (kept.root, SnapshotView::Waiting(kept.view))),
            saved_top: None,
            watching: options.watch.then_some(Watching {
                quiet: options.quiet,
            }),
            watch_task: None,
            watch_error: None,
            rescan_pending: false,
        };
        let view = options.view;
        match launch {
            Launch::Empty => {}
            Launch::Folder(root) => {
                // The filter and the rules apply to the first scan already.
                if let Some(view) = &view {
                    state.take_filter(view);
                }
                state.open_folder(root, cx);
                state.restore = view;
            }
            Launch::Tree(tree) => {
                if let Some(view) = &view {
                    state.take_filter(view);
                }
                state.show_tree(tree);
                state.restore = view;
                state.restore_view(cx);
            }
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

    /// The file on the stage with the tree it belongs to: while this stays
    /// the same, so does what the stage shows.
    pub(crate) fn staged(&self) -> (u64, Option<NodeId>) {
        (self.tree_generation, self.current)
    }

    /// Where the file on the stage sits in the gallery, counting from 1,
    /// and how many files the gallery holds: "2 / 3". `None` when the file
    /// is not checked. Cheap enough for every change to the state.
    #[must_use]
    pub fn position(&self) -> Option<(usize, usize)> {
        let explorer = self.explorer.as_ref()?;
        let at = explorer.gallery_position(self.current?)?;
        Some((at + 1, explorer.gallery_len()))
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

    /// How many files are checked, hidden ones included. Constant time.
    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.selected
    }

    /// How many scans have started, the first included: a probe for tests
    /// of live refresh.
    #[must_use]
    pub fn scans(&self) -> u64 {
        self.scan_generation
    }

    /// Whether the presentation is showing: the stage alone, fullscreen.
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

    // Shared steps

    /// Call whenever the tree on screen is replaced or removed.
    fn tree_replaced(&mut self) {
        self.tree_generation += 1;
    }

    fn set_focus(&mut self, id: NodeId, row: usize) {
        self.focus = Some(id);
        self.focus_row = row;
    }
}
