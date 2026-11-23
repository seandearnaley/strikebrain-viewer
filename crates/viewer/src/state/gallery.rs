//! The gallery: which of the stage's two views shows, the tile with
//! keyboard focus, and the rules that move them.
//!
//! The stage shows either the gallery, a grid of the checked files, or the
//! file view, one file filling the pane. Any change to the checked files
//! shows the gallery when two or more are checked, the one file when one
//! is, and a blank stage when none is. A click or Enter on a file row, or
//! on a tile, opens that file in the file view; Esc there, or the grid
//! button, goes back to the gallery, focused on that file's tile. A
//! presentation leaves to the view it started from.
//!
//! The tile with keyboard focus is kept by node, as the tree's focus row
//! is, so it survives the gallery changing around it; while the gallery
//! shows it is always a checked file.

use explorer::NodeId;
use gpui_kit::Context;

use super::{AppState, StateEvent};

/// Which of the stage's two views shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StageView {
    /// One file fills the pane: `current`, or nothing.
    #[default]
    File,
    /// The grid of the checked files.
    Gallery,
}

/// A keyboard movement over the tiles, in gallery order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TileMotion {
    /// One tile back, onto the row above from a row's first tile.
    Previous,
    /// One tile on, onto the row below from a row's last tile.
    Next,
    /// One row up.
    Up,
    /// One row down; to the last tile when the row below is too short.
    Down,
    /// This many rows up, stopping at the first tile.
    PageUp(usize),
    /// This many rows down, stopping at the last tile.
    PageDown(usize),
    First,
    Last,
}

/// A tile as one frame drew it: which tree and which file. A click
/// reaches a frame's listeners before the next frame is drawn, so the tree
/// may have been replaced or the file unchecked by then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DrawnTile {
    tree: u64,
    id: NodeId,
}

/// Where `motion` takes tile focus from `from`, in a gallery of `len`
/// tiles laid out `columns` to a row.
pub(crate) fn tile_after(motion: TileMotion, from: usize, len: usize, columns: usize) -> usize {
    let last = len.saturating_sub(1);
    let columns = columns.max(1);
    match motion {
        TileMotion::Previous => from.saturating_sub(1),
        TileMotion::Next => (from + 1).min(last),
        TileMotion::Up => from.checked_sub(columns).unwrap_or(from),
        TileMotion::Down => match from + columns {
            below if below <= last => below,
            // A short last row: its last tile, if it is a row further down.
            _ if last / columns > from / columns => last,
            _ => from,
        },
        TileMotion::PageUp(rows) => from.saturating_sub(rows.max(1) * columns),
        TileMotion::PageDown(rows) => from.saturating_add(rows.max(1) * columns).min(last),
        TileMotion::First => 0,
        TileMotion::Last => last,
    }
}

impl AppState {
    /// Which view the stage is asked for. While presenting the stage shows
    /// the file whatever this says, and returns to it after.
    #[must_use]
    pub fn stage_view(&self) -> StageView {
        self.view
    }

    /// Whether the stage shows the gallery now: its view, and no
    /// presentation in front of it.
    #[must_use]
    pub fn shows_gallery(&self) -> bool {
        self.view == StageView::Gallery && !self.presenting
    }

    /// The tile with keyboard focus while the gallery shows.
    #[must_use]
    pub fn tile(&self) -> Option<NodeId> {
        self.tile
    }

    /// Counts changes to what the gallery holds and in which order: the
    /// checks, the sort and the tree. The gallery redraws when it moves.
    pub(crate) fn gallery_version(&self) -> (u64, u64) {
        (self.tree_generation, self.gallery_changes)
    }

    /// The tile of `id` as the frame being drawn shows it.
    pub(crate) fn drawn_tile(&self, id: NodeId) -> DrawnTile {
        DrawnTile {
            tree: self.tree_generation,
            id,
        }
    }

    /// A click on a drawn tile: opens its file in the file view. Does
    /// nothing and returns false when the tile is gone since it was drawn.
    pub(crate) fn open_tile(&mut self, tile: DrawnTile, cx: &mut Context<Self>) -> bool {
        let still_drawn = tile.tree == self.tree_generation
            && self
                .explorer
                .as_ref()
                .is_some_and(|explorer| explorer.is_selected(tile.id));
        if still_drawn {
            self.open(tile.id, cx);
        }
        still_drawn
    }

    /// Enter in the gallery: opens the focused tile's file.
    pub fn open_focus_tile(&mut self, cx: &mut Context<Self>) {
        if self.shows_gallery()
            && let Some(tile) = self.tile
        {
            self.open(tile, cx);
        }
    }

    /// Opens the checked file `id` in the file view.
    fn open(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.current = Some(id);
        self.tile = Some(id);
        self.view = StageView::File;
        cx.emit(StateEvent::CurrentMoved);
        cx.notify();
    }

    /// Esc in the file view, and its grid button: back to the gallery when
    /// two or more files are checked, its focus on the tile of the file
    /// that was showing (or the nearest checked file) and scrolled to it.
    /// Returns false, changing nothing, with fewer than two.
    pub fn show_gallery(&mut self, cx: &mut Context<Self>) -> bool {
        if self.gallery_len() < 2 || self.presenting {
            return false;
        }
        self.view = StageView::Gallery;
        self.tile = self.nearest_tile(self.current);
        cx.emit(StateEvent::RevealTile);
        cx.notify();
        true
    }

    /// Moves tile focus by `motion` in a grid `columns` tiles wide; returns
    /// the focused tile's place in the gallery.
    pub(crate) fn move_tile(
        &mut self,
        motion: TileMotion,
        columns: usize,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        if !self.shows_gallery() {
            return None;
        }
        let explorer = self.explorer.as_ref()?;
        let len = explorer.gallery_len();
        let from = self
            .tile
            .and_then(|tile| explorer.gallery_position(tile))
            .unwrap_or(0);
        let to = tile_after(motion, from, len, columns);
        self.tile = explorer.gallery_from(to).next();
        cx.notify();
        self.tile.map(|_| to)
    }

    /// How many files are checked: the gallery's size.
    fn gallery_len(&self) -> usize {
        self.explorer
            .as_ref()
            .map_or(0, explorer::Explorer::gallery_len)
    }

    /// The checked file nearest `id`: itself when checked, else the next
    /// checked file after it, else the one before, else the first.
    fn nearest_tile(&self, id: Option<NodeId>) -> Option<NodeId> {
        let explorer = self.explorer.as_ref()?;
        id.and_then(|id| {
            if explorer.is_selected(id) {
                Some(id)
            } else {
                explorer
                    .gallery_next(id)
                    .or_else(|| explorer.gallery_previous(id))
            }
        })
        .or_else(|| explorer.gallery().next())
    }

    /// After a change to the checks the person made: the gallery with two
    /// or more checked files, its focus on the file on the stage; the one
    /// file with one; a blank stage with none.
    pub(super) fn show_for_checks(&mut self, cx: &mut Context<Self>) {
        match self.gallery_len() {
            0 => {
                self.current = None;
                self.view = StageView::File;
            }
            1 => {
                self.current = self.explorer.as_ref().and_then(|e| e.gallery().next());
                self.view = StageView::File;
            }
            _ => {
                self.view = StageView::Gallery;
                self.tile = self.nearest_tile(self.current.or(self.tile));
                cx.emit(StateEvent::RevealTile);
            }
        }
    }

    /// After the gallery changed by anything else (a rescan, a folder
    /// removed, a view restored): a gallery left with fewer than two files
    /// gives way to the file view, and tile focus stays on a checked file.
    pub(super) fn settle_view(&mut self) {
        self.gallery_changes += 1;
        if self.view == StageView::Gallery && self.gallery_len() < 2 {
            self.view = StageView::File;
        }
        let tile_checked = self.tile.is_some_and(|tile| {
            self.explorer
                .as_ref()
                .is_some_and(|explorer| explorer.is_selected(tile))
        });
        if !tile_checked {
            self.tile = self.nearest_tile(self.current);
        }
    }

    /// Leaving a presentation that started from the gallery: its focus on
    /// the file presented last, scrolled to it.
    pub(super) fn back_from_presenting(&mut self, cx: &mut Context<Self>) {
        if self.view == StageView::Gallery {
            self.tile = self.nearest_tile(self.current);
            cx.emit(StateEvent::RevealTile);
        }
    }

    /// Restores the saved view once the folders are on screen: the gallery
    /// only while two or more files are checked.
    pub(super) fn restore_view(&mut self, view: StageView, cx: &mut Context<Self>) {
        if view == StageView::Gallery {
            self.show_gallery(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TileMotion, tile_after};

    #[test]
    fn left_and_right_run_through_the_rows_and_stop_at_the_ends() {
        // Seven tiles, three to a row: 0 1 2 / 3 4 5 / 6.
        let step = |motion, from| tile_after(motion, from, 7, 3);
        assert_eq!(step(TileMotion::Next, 2), 3, "a row's end leads on");
        assert_eq!(step(TileMotion::Previous, 3), 2, "a row's start leads back");
        assert_eq!(step(TileMotion::Next, 6), 6);
        assert_eq!(step(TileMotion::Previous, 0), 0);
    }

    #[test]
    fn up_and_down_keep_the_column_and_find_the_short_last_row() {
        let step = |motion, from| tile_after(motion, from, 7, 3);
        assert_eq!(step(TileMotion::Down, 1), 4);
        assert_eq!(step(TileMotion::Down, 3), 6);
        // Nothing under 4 or 5 in the short last row: its last tile.
        assert_eq!(step(TileMotion::Down, 4), 6);
        assert_eq!(step(TileMotion::Down, 5), 6);
        // The last row: nowhere further down.
        assert_eq!(step(TileMotion::Down, 6), 6);
        assert_eq!(step(TileMotion::Up, 6), 3);
        assert_eq!(step(TileMotion::Up, 2), 2);
        // Six tiles fill two rows: nothing below the last row.
        assert_eq!(tile_after(TileMotion::Down, 4, 6, 3), 4);
    }

    #[test]
    fn pages_move_by_whole_rows_and_stop_at_the_ends() {
        let step = |motion, from| tile_after(motion, from, 100, 4);
        assert_eq!(step(TileMotion::PageDown(3), 1), 13);
        assert_eq!(step(TileMotion::PageDown(3), 97), 99);
        assert_eq!(step(TileMotion::PageUp(3), 13), 1);
        assert_eq!(step(TileMotion::PageUp(3), 5), 0);
        assert_eq!(step(TileMotion::First, 50), 0);
        assert_eq!(step(TileMotion::Last, 50), 99);
        // No row fits: a page is still one row.
        assert_eq!(step(TileMotion::PageDown(0), 1), 5);
    }

    #[test]
    fn one_column_moves_by_one_either_way() {
        assert_eq!(tile_after(TileMotion::Down, 2, 5, 1), 3);
        assert_eq!(tile_after(TileMotion::Up, 2, 5, 0), 1);
    }
}
