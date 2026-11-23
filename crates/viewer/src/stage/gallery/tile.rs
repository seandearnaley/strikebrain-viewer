//! One tile: the file's picture, the top of its text, or its kind's icon,
//! in a 4:3 frame, its name under it.
//!
//! A picture is fitted inside the frame, scaled down and never up. Text is
//! the monospace font at 9 px on 12 px lines, clipped to the frame at the
//! last whole line, with no highlighting. A tile shows nothing until its content is read. The tile
//! of the file on the stage has the accent background; the focused tile
//! has the focus ring, outside the frame, after a key press.

use std::rc::Rc;

use explorer::NodeId;
use gpui_kit::component::{ActiveTheme as _, Icon, v_flex};
use gpui_kit::{
    AnyElement, App, ClickEvent, IntoElement, ObjectFit, ParentElement as _, Pixels, RenderOnce,
    Role, SharedString, Size, StatefulInteractiveElement, Styled as _, StyledImage as _,
    TestSupportExt as _, Window, div, img, prelude::*, px,
};

use super::grid::{NAME_GAP, NAME_LINE};
use super::thumbs::Look;
use crate::row::file_icon;

/// The text of a text tile: 9 px on 12 px lines.
const TEXT_SIZE: Pixels = px(9.);
const TEXT_LINE: Pixels = px(12.);

/// How far inside the frame its content sits: 8 px.
const INSET: Pixels = px(8.);

/// How far outside the frame the focus ring sits, and its width.
const RING_OFFSET: Pixels = px(3.);

type Handler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// How the tile shows keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    Elsewhere,
    /// The focused tile, before any key press: no ring.
    Here,
    /// The focused tile with its ring.
    Ringed,
}

/// One tile, built fresh each frame.
#[derive(IntoElement)]
pub(crate) struct Tile {
    id: NodeId,
    name: SharedString,
    /// The extension, for the kind's icon.
    ext: SharedString,
    look: Option<Look>,
    size: Size<Pixels>,
    current: bool,
    focus: Focus,
    /// Where it sits in the grid: row, then column.
    at: (usize, usize),
    on_click: Handler,
}

impl Tile {
    pub(crate) fn new(
        id: NodeId,
        name: &str,
        ext: &str,
        size: Size<Pixels>,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id,
            name: name.to_owned().into(),
            ext: ext.to_owned().into(),
            look: None,
            size,
            current: false,
            focus: Focus::Elsewhere,
            at: (0, 0),
            on_click: Rc::new(on_click),
        }
    }

    /// What the tile shows: `None` until it is read.
    pub(crate) fn look(mut self, look: Option<Look>) -> Self {
        self.look = look;
        self
    }

    /// The tile of the file on the stage.
    pub(crate) fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }

    pub(crate) fn focus(mut self, focus: Focus) -> Self {
        self.focus = focus;
        self
    }

    /// Its row and column in the grid, for assistive technology.
    pub(crate) fn at(mut self, row: usize, column: usize) -> Self {
        self.at = (row, column);
        self
    }
}

impl Tile {
    /// What the frame holds: the picture, the text or the icon, or nothing
    /// before the tile is read.
    fn content(&self, cx: &App) -> Option<AnyElement> {
        let theme = cx.theme();
        Some(match self.look.clone()? {
            Look::Picture(image) => div()
                .id("tile-image")
                .test_support()
                .role(Role::Image)
                .aria_label(self.name.clone())
                .absolute()
                .inset(INSET)
                .flex()
                .child(img(image).size_full().object_fit(ObjectFit::ScaleDown))
                .into_any_element(),
            Look::Text(head) => div()
                .id("tile-text")
                .test_support()
                .aria_value(head.clone())
                .absolute()
                .top(INSET)
                .left(INSET)
                .right(INSET)
                // Whole lines only: none cut through at the bottom.
                .h(TEXT_LINE
                    * ((self.size.height - INSET * 2.) / TEXT_LINE)
                        .floor()
                        .max(0.))
                .overflow_hidden()
                .font_family(theme.mono_font_family.clone())
                .text_size(TEXT_SIZE)
                .line_height(TEXT_LINE)
                .whitespace_nowrap()
                .text_color(theme.foreground)
                .child(head)
                .into_any_element(),
            Look::Icon => div()
                .id("tile-icon")
                .test_support()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(file_icon(&self.ext))
                        .size_8()
                        .text_color(theme.muted_foreground),
                )
                .into_any_element(),
        })
    }

    /// The 4:3 frame and, after a key press, the focus ring around it.
    fn frame(&self, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (muted, accent) = (theme.muted, theme.accent);
        let hover = theme
            .accent
            .opacity(if theme.is_dark() { 0.6 } else { 0.7 });
        let current = self.current;
        div()
            .relative()
            .w_full()
            .h(self.size.height)
            .child(
                div()
                    .id("tile-frame")
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .rounded(theme.radius)
                    .map(|frame| {
                        if current {
                            frame.bg(accent)
                        } else {
                            frame.bg(muted).hover(|style| style.bg(hover))
                        }
                    })
                    .children(self.content(cx)),
            )
            .when(self.focus == Focus::Ringed, |frame| {
                frame.child(
                    div()
                        .id("tile-ring")
                        .test_support()
                        .absolute()
                        .top(-RING_OFFSET)
                        .bottom(-RING_OFFSET)
                        .left(-RING_OFFSET)
                        .right(-RING_OFFSET)
                        .rounded(theme.radius + RING_OFFSET)
                        .border_2()
                        .border_color(theme.ring),
                )
            })
    }
}

impl RenderOnce for Tile {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let name_color = if self.current {
            theme.foreground
        } else {
            theme.muted_foreground
        };
        let frame = self.frame(cx);
        let on_click = Rc::clone(&self.on_click);

        v_flex()
            .id(("tile", self.id.index()))
            .test_support()
            .role(Role::GridCell)
            .aria_label(self.name.clone())
            .aria_selected(self.current)
            .aria_row_index(self.at.0)
            .aria_column_index(self.at.1)
            .when(
                self.focus != Focus::Elsewhere,
                StatefulInteractiveElement::aria_active_descendant,
            )
            .flex_shrink_0()
            .w(self.size.width)
            .gap(NAME_GAP)
            .on_click(move |event, window, cx| on_click(event, window, cx))
            .child(frame)
            .child(
                div()
                    .h(NAME_LINE)
                    .w_full()
                    .text_xs()
                    .line_height(NAME_LINE)
                    .truncate()
                    .text_color(name_color)
                    .child(self.name),
            )
    }
}
