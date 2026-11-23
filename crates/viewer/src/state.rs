//! The application state: one entity that both panes observe.
//!
//! [`AppState`] owns the folders asked for, the explorer engine over the
//! ones on screen, the file on the stage (`current`), the keyboard focus
//! row and the filter asked for. Views read it in `render` and change it
//! only through its methods, each of which notifies once.
//!
//! The folders and their order live in `state/folders.rs`, scanning them
//! in `state/scan.rs`, following them on disk in `state/watch.rs`, moving
//! over the rows in `state/rows.rs`, checks and the stage in
//! `state/checks.rs`, the gallery and which of the stage's views shows in
//! `state/gallery.rs`, filtering in `state/filter.rs`, and restoring a
//! saved view in `state/restore.rs`.

mod checks;
mod filter;
mod folders;
mod gallery;
mod restore;
mod rows;
mod scan;
mod watch;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use explorer::{Explorer, IdMap, KindFilter, NodeId, Sort, Tree};
use gpui_kit::{Context, EventEmitter, Result, Task};

use crate::persist::SavedView;
use folders::Folder;
pub use gallery::StageView;
pub(crate) use gallery::{DrawnTile, TileMotion};
use restore::Pending;
use watch::Watching;

/// What the app shows when it starts.
#[derive(Debug)]
pub enum Launch {
    /// No folder: the explorer offers "Add Folder".
    Empty,
    /// Scan these folders, each a top-level folder, in this order. A file
    /// among them opens its folder and shows on the stage.
    Folders(Vec<PathBuf>),
    /// Show a tree that is already built, such as a `Tree::from_paths`
    /// fixture, as it is: its top-level entries are the top rows. Nothing
    /// touches the disk; refreshing does nothing.
    Tree(Arc<Tree>),
}

/// What an [`AppState`] brings back from a saved state, and whether it
/// follows its folders on disk.
#[derive(Clone, Debug, Default)]
pub struct StateOptions {
    /// A saved view of the folders `launch` opens. Its filter and Show
    /// ignored apply from the start, each folder's open folders and checks
    /// when that folder's first scan lands, and the sort, the file on the
    /// stage and the top row as the folders land.
    pub view: Option<SavedView>,
    /// Rescan a folder when something under it changes on disk.
    pub watch: bool,
    /// Changes to these files never cause a rescan: the app's own state
    /// file and its temporary name, should an open folder hold them.
    pub quiet: Vec<PathBuf>,
}

/// Where the folders' scans stand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanStatus {
    /// No scan is running.
    Idle,
    /// A scan is running; `slow` once one has taken longer than 200 ms.
    Scanning {
        /// Whether to show the progress bar.
        slow: bool,
    },
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
    /// A press on a top-level folder's remove button.
    Remove,
}

/// What [`AppState`] tells its views besides "something changed".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StateEvent {
    /// The rows were rebuilt by a command, not by a key on one row: a filter
    /// landed, folders opened or closed together, the sort or the tree
    /// changed. The tree keeps the focus row on screen.
    RowsReplaced,
    /// The stage moved to another file by its own keys, or a tile opened
    /// one. The tree scrolls that file's row into view if it is a row, and
    /// opens no folder.
    CurrentMoved,
    /// The gallery's focus moved to a tile the gallery should scroll into
    /// view: back from the file view or a presentation, or a check changed.
    RevealTile,
    /// A saved view was restored, or a launch settled: the tree scrolls
    /// this row to its top.
    TopRow(usize),
    /// A folder was rescanned, taken in or removed while the app runs. The
    /// row that was at the top, if it is still a row, moved from the first
    /// index to the second: the tree keeps it where it was, sub-row offset
    /// and all. A focus row that was on screen stays on screen.
    RowsMoved {
        /// The top row's index before and after.
        top: Option<(usize, usize)>,
        /// How many rows there were before.
        rows_before: usize,
        /// The focus row's index before.
        focus_before: Option<usize>,
    },
}

/// What the tree showed before a change of folders: see
/// [`StateEvent::RowsMoved`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Before {
    top: Option<(usize, NodeId)>,
    rows: usize,
    focus: Option<usize>,
}

/// The application state. See the module docs.
pub struct AppState {
    /// Every folder asked for, in the order added: being scanned, on
    /// screen, or unreadable.
    folders: Vec<Folder>,
    /// The key the next folder takes.
    next_key: u64,
    /// The tree of the folders on screen; `None` before the first lands.
    explorer: Option<Explorer>,
    /// Bumped whenever the tree on screen is replaced or removed, so a row
    /// drawn for an older tree is recognised. A count, not the tree's
    /// address, which a new tree can reuse.
    tree_generation: u64,
    /// The file on the stage.
    current: Option<NodeId>,
    /// Which of the stage's views shows: the gallery or the file.
    view: StageView,
    /// The tile with keyboard focus in the gallery, by node.
    tile: Option<NodeId>,
    /// Where `tile` was last seen in the gallery: where its focus goes if
    /// its file goes.
    tile_at: usize,
    /// Counts changes to the gallery other than a new tree: the checks and
    /// the order.
    gallery_changes: u64,
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
    /// The sort, kept here too so it outlasts a tree with no folder.
    sort: Sort,
    /// The running filter job. Replacing it drops the previous one.
    filter_task: Option<Task<Result<()>>>,
    /// Whether scans list the files the ignore rules would leave out.
    show_ignored: bool,
    /// How many scans have started, across folders; each scan's number.
    scans: u64,
    /// Files asked for, each waiting for a tree that lists it to show, and
    /// whether to check it: it was asked for with others.
    asked_files: Vec<(PathBuf, bool)>,
    /// The parts of a saved view that wait for the folders to land.
    restore: Option<Pending>,
    /// The saved top row, waiting for sizes before a Size sort settles.
    restore_top: Option<PathBuf>,
    /// How to follow the folders on disk; `None` does not.
    watching: Option<Watching>,
    /// Which row is at the top of the tree's scroll, given how many rows
    /// there are: a probe the tree view hands over, so a change of folders
    /// can keep that row at the top, and a save can name it.
    top_row: Option<TopRowProbe>,
}

/// See [`AppState::probe_top_row`].
pub(crate) type TopRowProbe = Rc<dyn Fn(usize) -> Option<usize>>;

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("folders", &self.folders)
            .field("explorer", &self.explorer)
            .field("current", &self.current)
            .field("view", &self.view)
            .field("tile", &self.tile)
            .field("focus", &self.focus)
            .field("selected", &self.selected)
            .field("presenting", &self.presenting)
            .field("query", &self.query)
            .field("kind", &self.kind)
            .field("show_ignored", &self.show_ignored)
            .field("scans", &self.scans)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<StateEvent> for AppState {}

impl AppState {
    /// Creates the state and starts whatever `launch` asks for. Nothing is
    /// restored and no folder is followed on disk.
    pub fn new(launch: Launch, cx: &mut Context<Self>) -> Self {
        Self::with_options(launch, StateOptions::default(), cx)
    }

    /// Creates the state, restoring and following as `options` say, and
    /// starts whatever `launch` asks for.
    pub fn with_options(launch: Launch, options: StateOptions, cx: &mut Context<Self>) -> Self {
        let mut state = Self {
            folders: Vec::new(),
            next_key: 0,
            explorer: None,
            tree_generation: 0,
            current: None,
            view: StageView::File,
            tile: None,
            tile_at: 0,
            gallery_changes: 0,
            focus: None,
            focus_row: 0,
            selected: 0,
            presenting: false,
            query: String::new(),
            kind: KindFilter::All,
            sort: Sort::default(),
            filter_task: None,
            show_ignored: false,
            scans: 0,
            asked_files: Vec::new(),
            restore: None,
            restore_top: None,
            watching: options.watch.then_some(Watching {
                quiet: options.quiet,
            }),
            top_row: None,
        };
        // The filter and the rules apply to the first scans already.
        let waiting = options.view.map(|view| state.take_view(view));
        match launch {
            Launch::Empty => {}
            Launch::Folders(folders) => {
                // The launch settles once every folder is on screen: at
                // its saved top row, or at the top.
                state.restore.get_or_insert_with(Pending::default);
                state.open_paths(folders, waiting.unwrap_or_default(), cx);
            }
            Launch::Tree(tree) => {
                state.show_tree(tree);
                for folder in waiting.unwrap_or_default() {
                    state.restore_folder(&folder);
                }
                state.landed(cx);
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

    /// Where the folders' scans stand: scanning while any is, slow when
    /// any is slow.
    #[must_use]
    pub fn status(&self) -> ScanStatus {
        let mut status = ScanStatus::Idle;
        for folder in &self.folders {
            match (folder.status(), &status) {
                (ScanStatus::Scanning { slow: true }, _) => return folder.status(),
                (scanning @ ScanStatus::Scanning { .. }, ScanStatus::Idle) => status = scanning,
                _ => {}
            }
        }
        status
    }

    /// Whether no folder is open or being opened: the "Add Folder" state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.folders.is_empty() && self.explorer.is_none()
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

    /// How many scans have started, across folders, the first included: a
    /// probe for tests of live refresh.
    #[must_use]
    pub fn scans(&self) -> u64 {
        self.scans
    }

    /// Whether the presentation is showing: the stage alone, fullscreen.
    #[must_use]
    pub fn is_presenting(&self) -> bool {
        self.presenting
    }

    /// The name of a node, for the stage.
    #[must_use]
    pub fn name(&self, id: NodeId) -> Option<&str> {
        Some(self.explorer.as_ref()?.tree().name(id))
    }

    /// Takes the tree view's probe of the row at the top of its scroll.
    pub(crate) fn probe_top_row(&mut self, probe: TopRowProbe) {
        self.top_row = Some(probe);
    }

    // Shared steps

    /// The row at the top of the tree's scroll and its node, if the tree
    /// view has laid it out.
    fn top_row(&self) -> Option<(usize, NodeId)> {
        let rows = self.explorer.as_ref()?.rows();
        let at = (self.top_row.as_ref()?)(rows.len())?;
        Some((at, *rows.get(at)?))
    }

    /// The node of the row at the top of the tree's scroll.
    fn top_node(&self) -> Option<NodeId> {
        self.top_row().map(|(_, id)| id)
    }

    /// What the tree shows now, before a change of folders.
    fn before(&self) -> Before {
        Before {
            top: self.top_row(),
            rows: self
                .explorer
                .as_ref()
                .map_or(0, |explorer| explorer.rows().len()),
            focus: self.focus_index(),
        }
    }

    /// After a change of folders, given what the tree showed `before` and
    /// the old-to-new `map`: while a launch or a saved view settles,
    /// [`AppState::landed`]; otherwise the views hear which way the rows
    /// moved, so the tree keeps what was on screen.
    fn landed_keeping(&mut self, before: Before, map: &IdMap, cx: &mut Context<Self>) {
        if self.restore.is_some() || self.restore_top.is_some() {
            return self.landed(cx);
        }
        self.settle_focus();
        let top = before.top.and_then(|(from, id)| {
            let to = self.explorer.as_ref()?.row_of(map.get(id)?)?;
            Some((from, to))
        });
        cx.emit(StateEvent::RowsMoved {
            top,
            rows_before: before.rows,
            focus_before: before.focus,
        });
        cx.notify();
    }

    /// Call whenever the tree on screen is replaced or removed.
    fn tree_replaced(&mut self) {
        self.tree_generation += 1;
    }

    fn set_focus(&mut self, id: NodeId, row: usize) {
        self.focus = Some(id);
        self.focus_row = row;
    }
}
