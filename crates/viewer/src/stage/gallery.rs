//! The stage’s virtualized gallery and folder list. Both folder layouts use
//! the open directory’s filtered children. The checked gallery uses gallery order.
//!
//! One GPUI `uniform_list` of rows of tiles, inside a `container_query` that
//! hands it the pane's size: the columns, the tiles' size and the rows'
//! height follow from the width (`gallery/grid.rs`), so the list builds
//! only the rows on screen, whatever the number of files. Each frame asks
//! the engine for the files from the first row's first tile on.
//!
//! What a tile shows is read on the background executor
//! (`gallery/read.rs`), nearest the viewport first: the rows drawn, then
//! the row below and the row above. At most four reads run at once: a read
//! holds its slot until it ends. A read for a tile that leaves those rows,
//! or for any tile while the gallery is hidden, is told to stop, which it
//! does before reading a picture whole and before decoding it; a decode
//! already under way runs to its end, in its slot. Results go to a cache
//! of their own with a byte budget (`gallery/thumbs.rs`).
//!
//! Keyboard focus stays on the stage, which declares the `Gallery` key
//! context while the gallery shows and hands its keys here. The tile with
//! focus is the state's; the gallery keeps it on screen and draws its
//! ring after a key press, until a pointer press.

mod grid;
mod read;
mod thumbs;
mod tile;

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::folder::{self, Folder};
use crate::columns::{self, Columns};
use crate::state::{FolderLayout, StageView, tile_after};
use explorer::{Explorer, Kind, NodeId};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, IntoElement,
    ParentElement as _, Render, Result, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, SvgRenderer, Task,
    TestSupportExt as _, UniformListScrollHandle, Window, container_query, div, point, prelude::*,
    px, uniform_list,
};

use crate::state::{AppState, StateEvent, TileMotion};

use super::load::Stamp;
use grid::{GAP, Grid};
use read::Read;
use thumbs::{Reading, THUMBNAIL_BYTES, Thumbs};
use tile::{Focus, Tile};

/// Reads that run at once: enough to keep the cores busy, few enough that
/// the pictures decoding whole at the same time stay a few hundred
/// megabytes.
const LOADS: usize = 4;

/// A read under way: its task, and the flag that tells it to stop. It
/// keeps its slot until the read ends, stopped or not.
struct Load {
    _task: Task<Result<()>>,
    stop: Arc<AtomicBool>,
}

/// What the gallery draws from the state: it redraws when this changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Seen {
    shows: bool,
    folder_layout: Option<FolderLayout>,
    version: (u64, u64),
    current: Option<NodeId>,
    tile: Option<NodeId>,
}

impl Seen {
    fn of(state: &AppState) -> Self {
        Self {
            shows: state.shows_gallery(),
            folder_layout: (state.stage_view() == StageView::Folder).then(|| state.folder_layout()),
            version: state.gallery_version(),
            current: state.current(),
            tile: state.tile(),
        }
    }
}

/// What the gallery holds, for tests and probes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GalleryProbe {
    renders: usize,
    columns: usize,
    page: usize,
    drawn: usize,
    thumbnails: usize,
    bytes: usize,
    loading: usize,
    stopping: usize,
    largest: Option<(u32, u32)>,
    fit: (u32, u32),
    budget: usize,
}

impl GalleryProbe {
    /// How many times the gallery has rendered.
    #[must_use]
    pub fn renders(&self) -> usize {
        self.renders
    }

    /// Tiles to a row, as last laid out; 0 before the first layout.
    #[must_use]
    pub fn columns(&self) -> usize {
        self.columns
    }

    /// Whole rows of tiles on screen, as last laid out: a page.
    #[must_use]
    pub fn page(&self) -> usize {
        self.page
    }

    /// How many rows of tiles the last frame built.
    #[must_use]
    pub fn rows_drawn(&self) -> usize {
        self.drawn
    }

    /// How many tiles' content the cache holds.
    #[must_use]
    pub fn thumbnails(&self) -> usize {
        self.thumbnails
    }

    /// How many bytes the tiles' content holds; never more than
    /// [`GalleryProbe::budget`].
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The tiles' byte budget.
    #[must_use]
    pub fn budget(&self) -> usize {
        self.budget
    }

    /// How many reads are under way, those told to stop included: never
    /// more than four.
    #[must_use]
    pub fn loading(&self) -> usize {
        self.loading
    }

    /// How many of the reads under way have been told to stop.
    #[must_use]
    pub fn stopping(&self) -> usize {
        self.stopping
    }

    /// The largest thumbnail held, in device pixels.
    #[must_use]
    pub fn largest(&self) -> Option<(u32, u32)> {
        self.largest
    }

    /// The box a thumbnail is decoded to fit, in device pixels.
    #[must_use]
    pub fn fit(&self) -> (u32, u32) {
        self.fit
    }
}

/// The gallery. See the module docs.
pub(crate) struct Gallery {
    state: Entity<AppState>,
    /// The stage's focus handle, which holds keyboard focus for the
    /// gallery.
    focus: FocusHandle,
    folder: Option<Folder>,
    scroll: UniformListScrollHandle,
    /// The grid as last laid out.
    grid: Option<Grid>,
    /// Scroll the focused tile into view at the next layout.
    reveal: bool,
    /// Whether the focus ring shows: after a key press, until a pointer
    /// press.
    ring: bool,
    /// The rows of tiles the last frame built.
    drawn: Range<usize>,
    thumbs: Thumbs,
    /// The reads under way, by file.
    loading: HashMap<Arc<Path>, Load>,
    /// A look at what to read is waiting for the frame to end.
    pump_queued: bool,
    svg: SvgRenderer,
    seen: Seen,
    renders: usize,
    _subscriptions: [Subscription; 2],
}

impl std::fmt::Debug for Gallery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gallery")
            .field("grid", &self.grid)
            .field("thumbs", &self.thumbs)
            .finish_non_exhaustive()
    }
}

impl Gallery {
    pub(crate) fn new(state: Entity<AppState>, focus: FocusHandle, cx: &mut Context<Self>) -> Self {
        let subscriptions = [
            // The state changes on every key in the tree; the gallery
            // redraws only when what it shows does.
            cx.observe(&state, |this, state, cx| {
                let seen = Seen::of(state.read(cx));
                if seen != this.seen {
                    // Focus that moved, by whatever means, is kept on screen.
                    if seen.tile != this.seen.tile {
                        this.reveal = true;
                    }
                    if !seen.shows && seen.folder_layout != Some(FolderLayout::Gallery) {
                        // Thumbnails are hidden: their reads stop.
                        for load in this.loading.values() {
                            load.stop.store(true, Ordering::Relaxed);
                        }
                    }
                    this.seen = seen;
                    cx.notify();
                }
            }),
            cx.subscribe(&state, |this, _, event: &StateEvent, cx| {
                if *event == StateEvent::RevealTile {
                    this.reveal = true;
                    cx.notify();
                }
            }),
        ];
        cx.on_release(|this, cx| {
            for image in this.thumbs.drain() {
                cx.drop_image(image, None);
            }
        })
        .detach();
        let seen = Seen::of(state.read(cx));
        Self {
            state,
            focus,
            folder: None,
            scroll: UniformListScrollHandle::new(),
            grid: None,
            reveal: true,
            ring: false,
            drawn: 0..0,
            thumbs: Thumbs::with_budget(THUMBNAIL_BYTES),
            loading: HashMap::new(),
            pump_queued: false,
            svg: cx.svg_renderer(),
            seen,
            renders: 0,
            _subscriptions: subscriptions,
        }
    }

    /// What the gallery holds, for tests and probes.
    pub(crate) fn probe(&self) -> GalleryProbe {
        GalleryProbe {
            renders: self.renders,
            columns: self.grid.map_or(0, |grid| grid.columns),
            page: self.grid.map_or(0, |grid| grid.page()),
            drawn: self.drawn.len(),
            thumbnails: self.thumbs.len(),
            bytes: self.thumbs.bytes(),
            loading: self.loading.len(),
            stopping: self
                .loading
                .values()
                .filter(|load| load.stop.load(Ordering::Relaxed))
                .count(),
            largest: self.thumbs.largest(),
            fit: self.grid.map_or((0, 0), |grid| grid.thumbnail_box()),
            budget: self.thumbs.budget(),
        }
    }

    /// A change of folder, sort, filter, or layout. Only a different source
    /// starts a fresh scroll position. A refreshed tree can renumber every id.
    pub(super) fn sync_folder(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let mut same = false;
        self.folder = state
            .explorer()
            .zip(state.current())
            .filter(|_| state.stage_view() == StageView::Folder)
            .map(|(e, id)| {
                let mut folder = Folder::new(e, id, state.staged().0);
                same = self
                    .folder
                    .as_ref()
                    .is_some_and(|was| folder.carry_focus(was, e));
                folder
            });
        if !same {
            self.scroll = UniformListScrollHandle::new();
            self.grid = None;
            self.drawn = 0..0;
            self.ring = false;
        }
        if !same
            || state.folder_layout() == FolderLayout::List
            || self
                .folder
                .as_ref()
                .zip(state.explorer())
                .is_some_and(|(f, e)| f.entries(e).is_empty())
        {
            for load in self.loading.values() {
                load.stop.store(true, Ordering::Relaxed);
            }
        }
        self.reveal = true;
        cx.notify();
    }

    fn is_showing(&self, state: &AppState) -> bool {
        self.folder
            .as_ref()
            .map_or_else(|| state.shows_gallery(), |folder| folder.is_current(state))
    }

    // Keys, from the stage

    /// Moves tile focus by `motion`, keeps the tile on screen and shows
    /// the ring.
    pub(crate) fn move_focus(&mut self, motion: TileMotion, cx: &mut Context<Self>) {
        if !self.is_showing(self.state.read(cx)) {
            return;
        }
        let grid = self.grid;
        let columns = grid.map_or(1, |grid| grid.columns);
        let motion = match (motion, grid) {
            (TileMotion::PageUp(_), Some(grid)) => TileMotion::PageUp(grid.page()),
            (TileMotion::PageDown(_), Some(grid)) => TileMotion::PageDown(grid.page()),
            (motion, _) => motion,
        };
        if let Some(folder) = &mut self.folder {
            if let Some(e) = self.state.read(cx).explorer() {
                let len = folder.entries(e).len();
                let to = match folder.focused() {
                    Some(from) => tile_after(motion, from, len, columns),
                    None if motion == TileMotion::Last => len.saturating_sub(1),
                    None => 0,
                };
                folder.focus(to, e);
            }
        } else {
            self.state
                .update(cx, |state, cx| state.move_tile(motion, columns, cx));
        }
        self.key_pressed(cx);
    }

    /// A key reached the gallery: the ring shows, on a tile kept on screen.
    pub(crate) fn key_pressed(&mut self, cx: &mut Context<Self>) {
        self.reveal = true;
        self.ring = true;
        cx.notify();
    }

    /// A pointer press anywhere in the gallery hides the ring.
    fn pointer_down(&mut self, cx: &mut Context<Self>) {
        if self.ring {
            self.ring = false;
            cx.notify();
        }
    }

    pub(super) fn open_focused(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if !self.is_showing(state) {
            return;
        }
        if let Some(folder) = &self.folder {
            let id = state
                .explorer()
                .and_then(|e| folder.entries(e).get(folder.focused().unwrap_or(0)))
                .copied();
            if let Some(id) = id {
                let generation = state.staged().0;
                self.state
                    .update(cx, |state, cx| state.navigate(generation, id, cx));
            }
        } else {
            self.state.update(cx, AppState::open_focus_tile);
        }
    }

    fn open(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        let opened = self.state.update(cx, |state, cx| match entry {
            Entry::Checked(tile) => state.shows_gallery() && state.open_tile(tile, cx),
            Entry::Child {
                parent,
                id,
                generation,
            } if state.staged() == (generation, Some(parent))
                && state.stage_view() == StageView::Folder =>
            {
                state.navigate(generation, id, cx);
                true
            }
            Entry::Child { .. } => false,
        });
        if opened {
            window.focus(&self.focus, cx);
        }
    }

    // Layout and drawing

    /// The grid as this frame lays it out: kept for the keys, the scroll
    /// held on what was on screen when the tiles' size or the columns
    /// changed, and the focused tile scrolled into view when asked.
    fn laid_out(&mut self, grid: Grid, cx: &mut Context<Self>) {
        let focused = {
            let state = self.state.read(cx);
            match &self.folder {
                Some(folder) => folder.focused(),
                None => state
                    .explorer()
                    .zip(state.tile())
                    .and_then(|(e, tile)| e.gallery_position(tile)),
            }
        };
        if let Some(was) = self.grid
            && (was.row_height != grid.row_height || was.columns != grid.columns)
        {
            self.reanchor(was, grid, focused);
        }
        self.grid = Some(grid);
        if self.reveal {
            self.reveal = false;
            if let Some(index) = focused {
                self.scroll
                    .scroll_to_item(grid.row_of(index), ScrollStrategy::Nearest);
            }
        }
    }

    /// After a resize changed the tiles' size or the columns: the list
    /// keeps its scroll in pixels, which would now show other rows. The
    /// focused tile, if it was on screen, goes back to where it stood;
    /// otherwise the first tile on screen keeps its place, the same share
    /// of its row above the top.
    fn reanchor(&self, was: Grid, now: Grid, focused: Option<usize>) {
        let handle = &self.scroll.0.borrow().base_handle;
        let offset = handle.offset();
        let scrolled = (-offset.y).max(px(0.));
        let row_top = |grid: Grid, index: usize| grid.row_height * count(grid.row_of(index));
        let on_screen = |index: usize| {
            let top = row_top(was, index) - scrolled;
            top + was.row_height > px(0.) && top < was.viewport
        };
        let scrolled = if let Some(index) = focused.filter(|&index| on_screen(index)) {
            row_top(now, index) - (row_top(was, index) - scrolled)
        } else {
            let first = whole_rows(scrolled / was.row_height);
            let into = (scrolled - was.row_height * count(first)) / was.row_height;
            row_top(now, first * was.columns) + now.row_height * into
        };
        handle.set_offset(point(offset.x, -scrolled.max(px(0.))));
    }

    /// The rows of tiles in `range`.
    fn render_rows(
        &mut self,
        range: Range<usize>,
        grid: Grid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        // The list measures its first row, then builds the rows on screen:
        // the last range asked for is what the frame shows.
        self.drawn = range.clone();
        if !self.is_showing(self.state.read(cx)) {
            return Vec::new();
        }
        if self.folder.is_none() || self.state.read(cx).folder_layout() == FolderLayout::Gallery {
            self.queue_pump(window, cx);
        }
        let state = self.state.read(cx);
        let Some(explorer) = state.explorer() else {
            return Vec::new();
        };
        let tree = explorer.tree();
        let current = state.current();
        let folder = self.folder.as_ref();
        let items = Items::new(explorer, folder);
        let focus = match folder {
            Some(f) => f
                .focused()
                .and_then(|ix| f.entries(explorer).get(ix))
                .copied(),
            None => state.tile(),
        };
        let ring = self.ring && self.focus.is_focused(window);
        let mut files = items.from(range.start * grid.columns);
        let drawn = |id| match folder {
            Some(f) => Entry::Child {
                parent: f.id,
                id,
                generation: state.staged().0,
            },
            None => Entry::Checked(state.drawn_tile(id)),
        };
        if folder.is_some() && state.folder_layout() == FolderLayout::List {
            let columns = Columns::for_width(grid.tile.width, window.rem_size());
            return files
                .take(range.len())
                .map(|id| {
                    let entry = drawn(id);
                    folder::details_row(
                        explorer,
                        id,
                        columns,
                        ring && Some(id) == focus,
                        cx.listener(move |this, _, window, cx| this.open(entry, window, cx)),
                        cx,
                    )
                    .into_any_element()
                })
                .collect();
        }
        range
            .map(|row| {
                let tiles: Vec<Tile> = (0..grid.columns)
                    .map_while(|column| Some((column, files.next()?)))
                    .map(|(column, id)| {
                        let path: Arc<Path> = tree.abs_path(id).into();
                        // A tree built in memory names files it has not got.
                        let kind = folder.map(|_| tree.kind(id));
                        let look = if path.is_absolute() && kind.is_none_or(|k| k == Kind::File) {
                            self.thumbs.get(&path)
                        } else {
                            Some(thumbs::Look::Icon)
                        };
                        let drawn = drawn(id);
                        Tile::new(
                            id,
                            tree.name(id),
                            tree.extension(id),
                            grid.tile,
                            cx.listener(move |this, _, window, cx| this.open(drawn, window, cx)),
                        )
                        .folder_entry(kind)
                        .look(look)
                        .current(current == Some(id))
                        .focus(match (focus == Some(id), ring) {
                            (false, _) => Focus::Elsewhere,
                            (true, false) => Focus::Here,
                            (true, true) => Focus::Ringed,
                        })
                        .at(row, column)
                    })
                    .collect();
                div()
                    .id(("tile-row", row))
                    .role(Role::Row)
                    .h(grid.row_height)
                    .w_full()
                    .pt(GAP)
                    .px(GAP)
                    .flex()
                    .gap(GAP)
                    .children(tiles)
                    .into_any_element()
            })
            .collect()
    }

    // Reading tiles

    /// Looks at what to read once the frame being drawn is done.
    fn queue_pump(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !self.pump_queued {
            self.pump_queued = true;
            cx.defer_in(window, |this, window, cx| {
                this.pump_queued = false;
                this.pump(window, cx);
            });
        }
    }

    /// Tells the reads for tiles off screen to stop, and starts reading
    /// the tiles on screen and either side, nearest first, while fewer than
    /// [`LOADS`] reads are under way.
    fn pump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.wanted(cx);
        let keep: HashSet<&Path> = wanted.iter().map(|want| want.path.as_ref()).collect();
        for (path, load) in &self.loading {
            // One wanted again goes on, unless it has stopped already.
            load.stop
                .store(!keep.contains(path.as_ref()), Ordering::Relaxed);
        }
        for want in wanted {
            if self.loading.len() >= LOADS {
                break;
            }
            if !self.loading.contains_key(&want.path) {
                self.start(want, window, cx);
            }
        }
    }

    /// The tiles to read, nearest the viewport first: those drawn, then
    /// the row below, then the row above.
    fn wanted(&self, cx: &App) -> Vec<Want> {
        let state = self.state.read(cx);
        let (Some(grid), Some(explorer)) = (self.grid, state.explorer()) else {
            return Vec::new();
        };
        if !self.is_showing(state)
            || self.folder.is_some() && state.folder_layout() == FolderLayout::List
        {
            return Vec::new();
        }
        let items = Items::new(explorer, self.folder.as_ref());
        let tree = explorer.tree();
        let generation = state.gallery_version().0;
        let fit = grid.thumbnail_box();
        let drawn = self.drawn.start.min(grid.rows)..self.drawn.end.min(grid.rows);
        let below = drawn.end..(drawn.end + 1).min(grid.rows);
        let above = drawn.start.saturating_sub(1)..drawn.start;
        let mut wanted = Vec::new();
        for rows in [drawn, below, above] {
            let count = rows.len() * grid.columns;
            for id in items.from(rows.start * grid.columns).take(count) {
                if self.folder.is_some() && tree.kind(id) != Kind::File {
                    continue;
                }
                let path: Arc<Path> = tree.abs_path(id).into();
                if !path.is_absolute() {
                    continue;
                }
                if let Some(Reading { held }) = self.thumbs.wants(&path, generation, fit) {
                    wanted.push(Want {
                        path,
                        name: tree.name(id).to_owned().into(),
                        held,
                        fit,
                        tree: generation,
                    });
                }
            }
        }
        wanted
    }

    /// Reads one tile on the background executor.
    fn start(&mut self, want: Want, window: &mut Window, cx: &mut Context<Self>) {
        let svg = self.svg.clone();
        let Want {
            path,
            name,
            held,
            fit,
            tree,
        } = want;
        let (key, read_path) = (Arc::clone(&path), Arc::clone(&path));
        let stop = Arc::new(AtomicBool::new(false));
        let read_stop = Arc::clone(&stop);
        let task = cx.spawn_in(window, async move |this, cx| {
            let read = cx
                .background_spawn(async move {
                    read::read(&read_path, &name, held, fit, &svg, &read_stop)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.landed(path, read, fit, tree, window, cx);
            })
        });
        self.loading.insert(key, Load { _task: task, stop });
    }

    /// A tile's read has ended: its slot frees; keep what it read, release
    /// what that pushed out, and redraw, which starts the next reads.
    fn landed(
        &mut self,
        path: Arc<Path>,
        read: Read,
        fit: (u32, u32),
        tree: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.loading.remove(&path);
        for image in self.thumbs.insert(path, read, fit, tree) {
            cx.drop_image(image, Some(window));
        }
        cx.notify();
    }
}

#[derive(Clone, Copy)]
enum Entry {
    Checked(crate::state::DrawnTile),
    Child {
        parent: NodeId,
        id: NodeId,
        generation: u64,
    },
}

/// Random access into either source, without materializing the checked set
/// or walking offscreen siblings to find the first row of the viewport.
enum Items<'a> {
    Checked(&'a Explorer),
    Children(&'a [NodeId]),
}

impl<'a> Items<'a> {
    fn new(e: &'a Explorer, folder: Option<&'a Folder>) -> Self {
        folder.map_or(Self::Checked(e), |f| Self::Children(f.entries(e)))
    }
    fn len(&self) -> usize {
        match self {
            Self::Checked(e) => e.gallery_len(),
            Self::Children(ids) => ids.len(),
        }
    }
    fn from(&self, start: usize) -> Box<dyn Iterator<Item = NodeId> + '_> {
        match self {
            Self::Checked(e) => Box::new(e.gallery_from(start)),
            Self::Children(ids) => Box::new(ids.get(start..).unwrap_or_default().iter().copied()),
        }
    }
}

/// A count of rows or tiles as a multiplier of pixels.
#[expect(
    clippy::cast_precision_loss,
    reason = "a row count past 2^24 only blurs a fraction of a row"
)]
fn count(n: usize) -> f32 {
    n as f32
}

/// The whole rows in a number of rows.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is floored and clamped to zero; rows number far below usize::MAX"
)]
fn whole_rows(rows: f32) -> usize {
    rows.floor().max(0.) as usize
}

/// A tile to read.
struct Want {
    path: Arc<Path>,
    name: SharedString,
    /// Unless it is as this.
    held: Option<Stamp>,
    fit: (u32, u32),
    tree: u64,
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let state = self.state.read(cx);
        let folder = self
            .folder
            .as_ref()
            .filter(|f| f.is_current(state))
            .zip(state.explorer());
        let tiles = state
            .explorer()
            .map_or(0, |e| Items::new(e, folder.map(|(f, _)| f)).len());
        let options = folder.map(|(f, e)| folder::Options::new(e, f.id));
        let toolbar = folder.zip(options).map(|((f, e), opts)| {
            folder::toolbar(
                opts,
                state.folder_layout(),
                tiles,
                e.children(f.id).len(),
                &self.state,
            )
        });
        let list = folder.is_some() && state.folder_layout() == FolderLayout::List;
        let header = options
            .filter(|_| list)
            .map(|o| columns::column_header("folder-header", o.sort, self.state.clone()));
        let empty = folder.filter(|_| tiles == 0).map(|(f, e)| f.empty_label(e));
        let surface = if list {
            "folder-list"
        } else if folder.is_some() {
            "folder-gallery"
        } else {
            "gallery"
        };
        let row_height = window.rem_size() * 2.;
        let scale = window.scale_factor();
        let rows = cx.processor(|this, (range, grid): (Range<usize>, Grid), window, cx| {
            this.render_rows(range, grid, window, cx)
        });
        let laid_out = cx.processor(|this, grid: Grid, _, cx| this.laid_out(grid, cx));
        let scroll = self.scroll.clone();

        let body = div()
            .id(surface)
            .test_support()
            .role(if list { Role::List } else { Role::Grid })
            .aria_label(if list { "Folder contents" } else { "Gallery" })
            // As last laid out: the list builds only the rows on screen.
            .when_some(self.grid.filter(|_| !list), |grid, laid_out| {
                grid.aria_row_count(laid_out.rows)
                    .aria_column_count(laid_out.columns)
            })
            .relative()
            .size_full()
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| this.pointer_down(cx)))
            .child(container_query(move |size, window, cx| {
                let grid = if list {
                    Grid::list(size, tiles, row_height)
                } else {
                    Grid::new(size, tiles, scale)
                };
                laid_out(grid, window, cx);
                uniform_list("tiles", grid.rows, move |range, window, cx| {
                    rows((range, grid), window, cx)
                })
                .track_scroll(&scroll)
                .size_full()
                .pb(GAP)
            }))
            .vertical_scrollbar(&self.scroll);
        v_flex()
            .size_full()
            .children(toolbar)
            .children(header)
            .child(div().flex_1().min_h_0().relative().child(body).when_some(
                empty,
                |view, label| {
                    view.child(
                        div()
                            .id("folder-empty")
                            .test_support()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(label),
                    )
                },
            ))
    }
}
