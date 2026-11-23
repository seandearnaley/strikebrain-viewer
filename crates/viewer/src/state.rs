//! The application state: one entity that both panes observe.
//!
//! [`AppState`] owns the explorer engine, the file on the stage
//! (`current`), the keyboard focus row, the filter asked for and the folder
//! scan. Views read it in `render` and change it only through its methods,
//! each of which notifies once.
//!
//! Scanning lives in `state/scan.rs`, filtering in `state/filter.rs`, and
//! restoring a saved view in `state/restore.rs`.

mod filter;
mod restore;
mod scan;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use explorer::{Explorer, KindFilter, NodeId, SortKey, Tree};
use gpui_kit::{Context, EventEmitter, Result, SharedString, Task};

use crate::persist::FolderView;

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

/// What an [`AppState`] brings back from a saved state.
#[derive(Clone, Debug, Default)]
pub struct StateOptions {
    /// A saved view of the folder `launch` opens, applied when its first
    /// scan lands. Its filter and Show ignored apply from the start.
    pub view: Option<FolderView>,
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
    /// restored.
    pub fn new(launch: Launch, cx: &mut Context<Self>) -> Self {
        Self::with_options(launch, StateOptions::default(), cx)
    }

    /// Creates the state, restoring what `options` holds, and starts
    /// whatever `launch` asks for.
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

    // Rows

    /// The row at `ix`, showing `id`, as the frame being drawn shows it.
    pub(crate) fn drawn_row(&self, ix: usize, id: NodeId) -> DrawnRow {
        DrawnRow {
            tree: self.tree_generation,
            ix,
            id,
        }
    }

    /// A press on a drawn row: focuses the row, then does `action` on it.
    /// Does nothing and returns false when the row is no longer on screen
    /// as it was drawn: the tree was replaced, or the rows changed under it.
    pub(crate) fn row_action(
        &mut self,
        row: DrawnRow,
        action: RowAction,
        cx: &mut Context<Self>,
    ) -> bool {
        let still_drawn = row.tree == self.tree_generation
            && self
                .explorer
                .as_ref()
                .is_some_and(|explorer| explorer.rows().get(row.ix) == Some(&row.id));
        if !still_drawn {
            return false;
        }
        self.set_focus(row.id, row.ix);
        match action {
            RowAction::Show => self.show(row.id, cx),
            RowAction::ToggleCheck => self.toggle_check(row.id, cx),
            RowAction::ToggleOpen => self.toggle_folder(row.id, cx),
        }
        cx.notify();
        true
    }

    /// Moves keyboard focus; returns the new focus row.
    pub(crate) fn move_focus(&mut self, motion: Motion, cx: &mut Context<Self>) -> Option<usize> {
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

    /// Opens or closes a folder: its chevron. Focus is already on it, and
    /// the rows above it do not move.
    fn toggle_folder(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.toggle_folder(id);
            cx.notify();
        }
    }

    /// Opens every folder: the toolbar's Expand all.
    pub fn expand_all(&mut self, cx: &mut Context<Self>) {
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.expand_all();
            self.rows_replaced(cx);
        }
    }

    /// Closes every folder: the toolbar's Collapse all. Focus inside a
    /// folder moves to its top-level folder.
    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.collapse_all();
            self.rows_replaced(cx);
        }
    }

    /// A click on a column header: sorts by `key`, or flips the direction
    /// when the tree is already sorted by it.
    pub fn sort_by(&mut self, key: SortKey, cx: &mut Context<Self>) {
        if let Some(explorer) = self.explorer.as_mut() {
            let sort = explorer.sort().clicked(key);
            explorer.set_sort(sort);
            self.rows_replaced(cx);
        }
    }

    // Checks and the stage

    /// Toggles a checkbox and applies the rules for `current`: checking a
    /// single file shows it; unchecking the file on the stage moves the stage
    /// to the next checked file, else the previous one, else nothing; an
    /// empty stage takes the first checked file.
    fn toggle_check(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        let current_was_checked = self.current.is_some_and(|c| explorer.is_selected(c));
        explorer.toggle_check(id);

        if explorer.tree().is_file(id) && explorer.is_selected(id) {
            self.current = Some(id);
        } else {
            self.leave_unchecked_current(current_was_checked);
        }
        self.selection_changed();
        cx.notify();
    }

    /// Checks every file the filter shows: the toolbar's Select all.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        explorer.select_all();
        self.selection_changed();
        cx.notify();
    }

    /// Unchecks every file, hidden ones included: Clear.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        let current_was_checked = self.current.is_some_and(|c| explorer.is_selected(c));
        explorer.clear_selection();
        self.leave_unchecked_current(current_was_checked);
        self.selection_changed();
        cx.notify();
    }

    /// Shows `id` on the stage if it is a file, checked or not. Does
    /// nothing on a folder.
    fn show(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(explorer) = &self.explorer else {
            return;
        };
        if explorer.tree().is_file(id) {
            self.current = Some(id);
            cx.notify();
        }
    }

    /// → on the stage: the next gallery file. From a file that is not
    /// checked, the first gallery file after it in display order. Stays
    /// put at the end of the gallery, and with nothing on the stage.
    pub fn show_next(&mut self, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    /// ← on the stage: the previous gallery file. From a file that is not
    /// checked, the last gallery file before it in display order.
    pub fn show_previous(&mut self, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let (Some(explorer), Some(current)) = (&self.explorer, self.current) else {
            return;
        };
        let to = if forward {
            explorer.gallery_next(current)
        } else {
            explorer.gallery_previous(current)
        };
        if let Some(to) = to {
            self.current = Some(to);
            cx.emit(StateEvent::CurrentMoved);
            cx.notify();
        }
    }

    /// Space or Enter on the stage: starts the presentation of the file on
    /// the stage. Does nothing with no file there.
    pub fn start_presenting(&mut self, cx: &mut Context<Self>) {
        if self.current.is_some() && !self.presenting {
            self.presenting = true;
            cx.notify();
        }
    }

    /// Esc in the presentation: back to the explorer and the stage, on the
    /// same file.
    pub fn stop_presenting(&mut self, cx: &mut Context<Self>) {
        if self.presenting {
            self.presenting = false;
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

    // Shared steps

    /// Call whenever the tree on screen is replaced or removed.
    fn tree_replaced(&mut self) {
        self.tree_generation += 1;
    }

    fn set_focus(&mut self, id: NodeId, row: usize) {
        self.focus = Some(id);
        self.focus_row = row;
    }

    /// After checks were removed: a `current` that was checked and no
    /// longer is moves to the next checked file, else the previous one,
    /// else nothing.
    fn leave_unchecked_current(&mut self, current_was_checked: bool) {
        let Some(explorer) = &self.explorer else {
            return;
        };
        if let Some(current) = self.current
            && current_was_checked
            && !explorer.is_selected(current)
        {
            self.current = explorer
                .gallery_next(current)
                .or_else(|| explorer.gallery_previous(current));
        }
    }

    /// After any change to the checks: recount, and give an empty stage the
    /// first checked file.
    fn selection_changed(&mut self) {
        let Some(explorer) = &self.explorer else {
            self.selected = 0;
            return;
        };
        self.selected = explorer.selection().count();
        if self.current.is_none() {
            self.current = explorer.gallery().next();
        }
    }

    /// After a command rebuilt the rows: keep focus on a row and tell the
    /// views.
    fn rows_replaced(&mut self, cx: &mut Context<Self>) {
        self.settle_focus();
        cx.emit(StateEvent::RowsReplaced);
        cx.notify();
    }

    /// Keeps `focus` on a row on screen after the rows changed. A focus row
    /// that is gone moves to its deepest folder still on screen, else to
    /// the first row. One pass over the rows, stopping at the focus row.
    fn settle_focus(&mut self) {
        let Some(explorer) = &self.explorer else {
            return;
        };
        let rows = explorer.rows();
        let Some(focus) = self.focus else {
            self.focus = rows.first().copied();
            self.focus_row = 0;
            return;
        };
        if rows.get(self.focus_row) == Some(&focus) {
            return;
        }
        let tree = explorer.tree();
        let mut nearest = None;
        for (row, &id) in rows.iter().enumerate() {
            if id == focus {
                self.focus_row = row;
                return;
            }
            if tree.subtree(id).contains(&focus.index()) {
                nearest = Some((row, id));
            }
        }
        if let Some((row, id)) = nearest {
            self.set_focus(id, row);
        } else {
            self.focus = rows.first().copied();
            self.focus_row = 0;
        }
    }
}
