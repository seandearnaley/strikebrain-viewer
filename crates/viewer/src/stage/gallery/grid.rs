//! The gallery's geometry: how many tiles fit across, how large each is,
//! and how tall a row of them stands.
//!
//! Columns fill the width: tiles at least 180 px wide, 4:3, 16 px apart and
//! 16 px in from the pane's edges. Under each tile its name, 8 px below on
//! a 16 px line. Each row of the list is the 16 px gap above it, the tile
//! and the name; the list's bottom padding is the last gap. Tiles take
//! whole pixels; what is left of the width stays at the right edge.

use gpui_kit::{Pixels, Size, px, size};

/// The space around the grid and between its tiles: 16 px.
pub(crate) const GAP: Pixels = px(16.);

/// The narrowest a tile gets before a column goes: 180 px.
const MIN_TILE: Pixels = px(180.);

/// Between a tile and its name: 8 px.
pub(crate) const NAME_GAP: Pixels = px(8.);

/// The name's line: 16 px.
pub(crate) const NAME_LINE: Pixels = px(16.);

/// A thumbnail is decoded for a box this many device pixels larger than
/// the tile, rounded up: a pane resized a little keeps its thumbnails.
const BOX_STEP: u32 = 64;

/// The grid laid out in a pane of one size. See the module docs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Grid {
    /// Tiles in a row.
    pub(crate) columns: usize,
    /// One tile, 4:3.
    pub(crate) tile: Size<Pixels>,
    /// The gap above a row, the tile and its name.
    pub(crate) row_height: Pixels,
    /// Rows of tiles, the last perhaps short.
    pub(crate) rows: usize,
    /// The pane's height.
    pub(crate) viewport: Pixels,
    /// Device pixels to a pixel, for the thumbnails.
    pub(crate) scale: f32,
}

impl Grid {
    /// The grid for `tiles` tiles in a pane of `pane` size on a display of
    /// `scale` device pixels to a pixel.
    pub(crate) fn new(pane: Size<Pixels>, tiles: usize, scale: f32) -> Self {
        let inner = (pane.width - GAP * 2.).max(px(1.));
        // Every column but the first brings a gap with it.
        let fit = ((inner + GAP) / (MIN_TILE + GAP)).floor();
        let columns = whole(fit).max(1);
        let gaps = GAP * count(columns - 1);
        let width = ((inner - gaps) / count(columns)).floor().max(px(1.));
        let tile = size(width, (width * 0.75).round());
        Self {
            columns,
            tile,
            row_height: GAP + tile.height + NAME_GAP + NAME_LINE,
            rows: tiles.div_ceil(columns),
            viewport: pane.height,
            scale,
        }
    }

    /// The row holding the tile at `index`.
    pub(crate) fn row_of(&self, index: usize) -> usize {
        index / self.columns
    }

    /// How many whole rows fit in the pane: a page for the page keys, at
    /// least one.
    pub(crate) fn page(&self) -> usize {
        whole((self.viewport / self.row_height).floor()).max(1)
    }

    /// The box a thumbnail is decoded to fit, in device pixels: the tile at
    /// the display's scale, rounded up to a step.
    pub(crate) fn thumbnail_box(&self) -> (u32, u32) {
        let device = |side: Pixels| {
            let pixels = whole((side * self.scale).ceil());
            u32::try_from(pixels)
                .unwrap_or(u32::MAX)
                .div_ceil(BOX_STEP)
                .saturating_mul(BOX_STEP)
        };
        (device(self.tile.width), device(self.tile.height))
    }
}

/// A count as a multiplier of pixels.
#[expect(
    clippy::cast_precision_loss,
    reason = "columns and gaps number in the tens"
)]
fn count(n: usize) -> f32 {
    n as f32
}

/// A whole, non-negative number of pixels or rows as a count.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is whole and clamped to zero below; panes are far below usize::MAX"
)]
fn whole(value: impl Into<f32>) -> usize {
    value.into().max(0.) as usize
}

#[cfg(test)]
mod tests {
    use gpui_kit::{px, size};

    use super::{GAP, Grid, NAME_GAP, NAME_LINE};

    #[test]
    fn columns_fill_the_width_with_tiles_at_least_180_px_wide() {
        // 700 - 32 = 668 across: three tiles of 212, two gaps of 16.
        let grid = Grid::new(size(px(700.), px(600.)), 10, 2.);
        assert_eq!(grid.columns, 3);
        assert_eq!(grid.tile, size(px(212.), px(159.)));
        assert_eq!(grid.rows, 4);
        assert_eq!(grid.row_height, GAP + px(159.) + NAME_GAP + NAME_LINE);
        // Exactly room for a fourth: 4 x 180 + 3 x 16 + 32.
        let grid = Grid::new(size(px(800.), px(600.)), 10, 1.);
        assert_eq!((grid.columns, grid.tile.width), (4, px(180.)));
        let grid = Grid::new(size(px(799.), px(600.)), 10, 1.);
        assert_eq!(grid.columns, 3);
    }

    #[test]
    fn a_pane_narrower_than_a_tile_still_has_one_column() {
        let grid = Grid::new(size(px(150.), px(600.)), 3, 1.);
        assert_eq!(grid.columns, 1);
        assert_eq!(grid.tile.width, px(118.));
        assert_eq!(grid.rows, 3);
        let none = Grid::new(size(px(0.), px(0.)), 0, 1.);
        assert_eq!((none.columns, none.rows), (1, 0));
        assert_eq!(none.page(), 1);
    }

    #[test]
    fn a_page_is_the_whole_rows_that_fit() {
        // Rows of 16 + 159 + 24 = 199 px: three fit in 600 px.
        let grid = Grid::new(size(px(700.), px(600.)), 100, 1.);
        assert_eq!(grid.page(), 3);
        assert_eq!(grid.row_of(0), 0);
        assert_eq!(grid.row_of(5), 1);
    }

    #[test]
    fn a_thumbnail_box_is_the_tile_at_the_display_scale_rounded_up() {
        let grid = Grid::new(size(px(700.), px(600.)), 1, 2.);
        // 424 x 318 device pixels, up to steps of 64.
        assert_eq!(grid.thumbnail_box(), (448, 320));
        let grid = Grid::new(size(px(700.), px(600.)), 1, 1.);
        assert_eq!(grid.thumbnail_box(), (256, 192));
    }
}
