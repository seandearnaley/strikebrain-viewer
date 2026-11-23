//! The column header, 28 px, above the tree and outside its scroll, so it
//! stays put. Name always; Type and Size as the tree widens, in the same
//! cells the rows use. A click sorts by a column; a second click flips the
//! direction. An arrow marks the sorted column and its direction.

use std::rc::Rc;

use explorer::{Direction, Sort, SortKey};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
use gpui_kit::{
    App, Context, Div, FontWeight, IntoElement, ParentElement as _, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, container_query,
    div, prelude::*,
};

use super::ExplorerPanel;
use crate::columns::{self, Columns};

type SortHandler = Rc<dyn Fn(SortKey, &mut Window, &mut App)>;

impl ExplorerPanel {
    pub(super) fn render_header(&self, sort: Sort, _: &Context<Self>) -> impl IntoElement + use<> {
        let state = self.state.clone();
        ColumnHeader {
            sort,
            on_sort: Rc::new(move |key, _, cx| {
                state.update(cx, |state, cx| state.sort_by(key, cx));
            }),
        }
    }
}

/// The header row.
#[derive(IntoElement)]
struct ColumnHeader {
    sort: Sort,
    on_sort: SortHandler,
}

impl RenderOnce for ColumnHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .id("header")
            .test_support()
            .role(Role::Row)
            .flex_shrink_0()
            .h_7()
            .w_full()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(container_query(move |size, window, cx| {
                self.cells(Columns::for_width(size.width, window.rem_size()), cx)
            }))
    }
}

impl ColumnHeader {
    fn cells(self, columns: Columns, cx: &App) -> impl IntoElement + use<> {
        let name = self.cell(
            div().flex_1().min_w_0(),
            "header-name",
            "Name",
            SortKey::Name,
            cx,
        );
        let kind = columns.kind.then(|| {
            self.cell(
                columns::kind_cell(),
                "header-type",
                "Type",
                SortKey::Type,
                cx,
            )
        });
        let size = columns.size.then(|| {
            self.cell(
                columns::size_cell(),
                "header-size",
                "Size",
                SortKey::Size,
                cx,
            )
        });
        columns::line().child(name).children(kind).children(size)
    }

    /// One clickable header cell on the column's own cell. The arrow sits
    /// on the side away from the column's values, so the label keeps its
    /// edge with them: after a left-aligned label, before the right-aligned
    /// Size.
    fn cell(
        &self,
        cell: Div,
        id: &'static str,
        label: &'static str,
        key: SortKey,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (foreground, muted) = (theme.foreground, theme.muted_foreground);
        let active = self.sort.key == key;
        let arrow = active.then(|| {
            let (id, icon) = match self.sort.direction {
                Direction::Ascending => ("sort-ascending", IconName::ArrowUp),
                Direction::Descending => ("sort-descending", IconName::ArrowDown),
            };
            div()
                .id(id)
                .test_support()
                .flex_shrink_0()
                .child(Icon::new(icon).size_3())
        });
        let (before, after) = if key == SortKey::Size {
            (arrow, None)
        } else {
            (None, arrow)
        };
        let on_sort = Rc::clone(&self.on_sort);
        let label: SharedString = label.into();

        cell.id(id)
            .test_support()
            .role(Role::ColumnHeader)
            .aria_label(label.clone())
            .h_full()
            .flex()
            .items_center()
            .gap_1()
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(if active { foreground } else { muted })
            .hover(|style| style.text_color(foreground))
            .on_click(move |_, window, cx| on_sort(key, window, cx))
            .children(before)
            .child(div().min_w_0().truncate().child(label))
            .children(after)
    }
}
