//! The gallery: the stage's grid of the checked files, in gallery order.
//!
//! One GPUI `uniform_list` of rows of tiles, inside a `container_query` that
//! hands it the pane's size: the columns, the tiles' size and the rows'
//! height follow from the width (`gallery/grid.rs`), so the list builds
//! only the rows on screen, whatever the number of files. Each frame asks
//! the engine for the files from the first row's first tile on.
//!
//! What a tile shows is read on the background executor
//! (`gallery/read.rs`), at most four at once, nearest the viewport first:
//! the rows drawn, then the row below and the row above. Work for a tile
//! that leaves those rows is dropped, as is all of it while the gallery is
//! hidden. Results go to a cache of their own with a byte budget
//! (`gallery/thumbs.rs`).
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

use explorer::NodeId;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, IntoElement,
    ParentElement as _, Render, Result, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, SvgRenderer, Task,
    TestSupportExt as _, UniformListScrollHandle, Window, container_query, div, prelude::*,
    uniform_list,
};

use crate::state::{AppState, StateEvent, TileMotion};

use super::load::Stamp;
use grid::{GAP, Grid};
use read::Read;
use thumbs::{Reading, THUMBNAIL_BYTES, Thumbs};
use tile::{Focus, Tile};

/// Tiles read at once: enough to keep the cores busy, few enough that the
/// pictures decoding whole at the same time stay a few hundred megabytes.
const LOADS: usize = 4;

/// What the gallery draws from the state: it redraws when this changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Seen {
    shows: bool,
    version: (u64, u64),
    current: Option<NodeId>,
    tile: Option<NodeId>,
}

impl Seen {
    fn of(state: &AppState) -> Self {
        Self {
            shows: state.shows_gallery(),
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

    /// How many tiles are being read.
    #[must_use]
    pub fn loading(&self) -> usize {
        self.loading
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
    /// The tiles being read, by file. Dropping one drops its work.
    loading: HashMap<Arc<Path>, Task<Result<()>>>,
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
                    if !seen.shows {
                        // Hidden: its work stops.
                        this.loading.clear();
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
            largest: self.thumbs.largest(),
            fit: self.grid.map_or((0, 0), |grid| grid.thumbnail_box()),
            budget: self.thumbs.budget(),
        }
    }

    // Keys, from the stage

    /// Moves tile focus by `motion`, keeps the tile on screen and shows
    /// the ring.
    pub(crate) fn move_focus(&mut self, motion: TileMotion, cx: &mut Context<Self>) {
        let grid = self.grid;
        let columns = grid.map_or(1, |grid| grid.columns);
        let motion = match (motion, grid) {
            (TileMotion::PageUp(_), Some(grid)) => TileMotion::PageUp(grid.page()),
            (TileMotion::PageDown(_), Some(grid)) => TileMotion::PageDown(grid.page()),
            (motion, _) => motion,
        };
        self.state
            .update(cx, |state, cx| state.move_tile(motion, columns, cx));
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

    /// A click on a drawn tile: opens its file, with keyboard focus on the
    /// stage.
    fn open(&mut self, tile: crate::state::DrawnTile, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.update(cx, |state, cx| state.open_tile(tile, cx)) {
            window.focus(&self.focus, cx);
        }
    }

    // Layout and drawing

    /// The grid as this frame lays it out: kept for the keys, and the
    /// focused tile scrolled into view when asked, or when the columns
    /// changed under it.
    fn laid_out(&mut self, grid: Grid, cx: &mut Context<Self>) {
        let reflowed = self.grid.is_some_and(|was| was.columns != grid.columns);
        self.grid = Some(grid);
        if !(self.reveal || reflowed) {
            return;
        }
        self.reveal = false;
        let state = self.state.read(cx);
        let index = state
            .explorer()
            .zip(state.tile())
            .and_then(|(explorer, tile)| explorer.gallery_position(tile));
        if let Some(index) = index {
            self.scroll
                .scroll_to_item(grid.row_of(index), ScrollStrategy::Nearest);
        }
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
        self.queue_pump(window, cx);
        let state = self.state.read(cx);
        let Some(explorer) = state.explorer() else {
            return Vec::new();
        };
        let tree = explorer.tree();
        let (current, focus) = (state.current(), state.tile());
        let ring = self.ring && self.focus.is_focused(window);
        let mut files = explorer.gallery_from(range.start * grid.columns);
        range
            .map(|row| {
                let tiles: Vec<Tile> = (0..grid.columns)
                    .map_while(|column| Some((column, files.next()?)))
                    .map(|(column, id)| {
                        let path: Arc<Path> = tree.abs_path(id).into();
                        // A tree built in memory names files it has not got.
                        let look = if path.is_absolute() {
                            self.thumbs.get(&path)
                        } else {
                            Some(thumbs::Look::Icon)
                        };
                        let drawn = state.drawn_tile(id);
                        Tile::new(
                            id,
                            tree.name(id),
                            tree.extension(id),
                            grid.tile,
                            cx.listener(move |this, _, window, cx| this.open(drawn, window, cx)),
                        )
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

    /// Drops the work for tiles off screen and starts reading the tiles on
    /// screen and either side, nearest first, at most [`LOADS`] at once.
    fn pump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = self.wanted(cx);
        let keep: HashSet<&Path> = wanted.iter().map(|want| want.path.as_ref()).collect();
        self.loading.retain(|path, _| keep.contains(path.as_ref()));
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
        if !state.shows_gallery() {
            return Vec::new();
        }
        let tree = explorer.tree();
        let generation = state.gallery_version().0;
        let fit = grid.thumbnail_box();
        let drawn = self.drawn.start.min(grid.rows)..self.drawn.end.min(grid.rows);
        let below = drawn.end..(drawn.end + 1).min(grid.rows);
        let above = drawn.start.saturating_sub(1)..drawn.start;
        let mut wanted = Vec::new();
        for rows in [drawn, below, above] {
            let count = rows.len() * grid.columns;
            for id in explorer.gallery_from(rows.start * grid.columns).take(count) {
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
        let task = cx.spawn_in(window, async move |this, cx| {
            let read = cx
                .background_spawn(async move { read::read(&read_path, &name, held, fit, &svg) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.landed(path, read, fit, tree, window, cx);
            })
        });
        self.loading.insert(key, task);
    }

    /// A tile's read is in: keep it, release what it pushed out, redraw.
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
        let tiles = self
            .state
            .read(cx)
            .explorer()
            .map_or(0, explorer::Explorer::gallery_len);
        let scale = window.scale_factor();
        let rows = cx.processor(|this, (range, grid): (Range<usize>, Grid), window, cx| {
            this.render_rows(range, grid, window, cx)
        });
        let laid_out = cx.processor(|this, grid: Grid, _, cx| this.laid_out(grid, cx));
        let scroll = self.scroll.clone();

        div()
            .id("gallery")
            .test_support()
            .role(Role::Grid)
            .aria_label("Gallery")
            .relative()
            .size_full()
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| this.pointer_down(cx)))
            .child(container_query(move |size, window, cx| {
                let grid = Grid::new(size, tiles, scale);
                laid_out(grid, window, cx);
                uniform_list("tiles", grid.rows, move |range, window, cx| {
                    rows((range, grid), window, cx)
                })
                .track_scroll(&scroll)
                .size_full()
                .pb(GAP)
            }))
            .vertical_scrollbar(&self.scroll)
    }
}
