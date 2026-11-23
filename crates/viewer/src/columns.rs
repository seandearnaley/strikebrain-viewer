//! The tree's columns: which show at a given width, and the cells that hold
//! them. Rows and the column header build their cells here, so a header
//! label and the values under it share one edge by construction.
//!
//! Left to right: Name (the tree itself), Type, Size. Type shows from a
//! 420 px wide tree, Size from 300 px. Every width is on the rem scale,
//! exact at the theme's 16 px base.

use gpui_kit::component::h_flex;
use gpui_kit::{Div, Pixels, Rems, Styled as _, div};

/// The tree width from which the Size column shows: 300 px.
const SIZE_FROM: Rems = Rems(18.75);
/// The tree width from which the Type column shows: 420 px.
const TYPE_FROM: Rems = Rems(26.25);
/// The Type column's width: 72 px, brainstrike's default.
const TYPE_WIDTH: Rems = Rems(4.5);

/// Which optional columns show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Columns {
    /// The Type column: the extension, muted.
    pub(crate) extension: bool,
    /// The Size column.
    pub(crate) size: bool,
}

impl Columns {
    /// The columns for a tree `width` wide, at the window's `rem` size.
    pub(crate) fn for_width(width: Pixels, rem: Pixels) -> Self {
        Self {
            extension: width >= TYPE_FROM.to_pixels(rem),
            size: width >= SIZE_FROM.to_pixels(rem),
        }
    }
}

/// One line of cells, full height, starting at the panel's 8 px inset.
pub(crate) fn line() -> Div {
    h_flex().size_full().pl_2()
}

/// The Type cell, which holds the extension: 72 px, its text 8 px in from
/// each side.
pub(crate) fn extension_cell() -> Div {
    div().flex_shrink_0().w(TYPE_WIDTH).px_2().truncate()
}

/// The Size cell: 80 px, its text right-aligned 8 px from the edge.
pub(crate) fn size_cell() -> Div {
    div().flex_shrink_0().w_20().pr_2().flex().justify_end()
}
