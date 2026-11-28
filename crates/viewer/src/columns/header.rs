//! The column header, 28 px, above the tree and outside its scroll, so it
//! stays put. Name always; Type and Size as the tree widens, in the same
//! cells the rows use. A click sorts by a column; a second click flips the
//! direction. An arrow marks the sorted column and its direction.
//!
//! For assistive technology the header is what a native table header is: a
//! group of sort buttons, each labelled with its column and, on the sorted
//! one, the direction.

use std::rc::Rc;

use explorer::{Direction, Sort, SortKey};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
use gpui_kit::{
    App, Div, Entity, FontWeight, IntoElement, ParentElement as _, RenderOnce, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, container_query,
    div, prelude::*,
};

use super::{self as columns, Columns};
use crate::state::AppState;

type SortHandler = Rc<dyn Fn(&SortKey, &mut Window, &mut App)>;

pub(crate) fn column_header(
    id: &'static str,
    sort: Sort,
    state: Entity<AppState>,
) -> impl IntoElement {
    ColumnHeader {
        id,
        sort,
        on_sort: Rc::new(move |key, _, cx| state.update(cx, |state, cx| state.sort_by(*key, cx))),
    }
}

/// The header row.
#[derive(IntoElement)]
struct ColumnHeader {
    id: &'static str,
    sort: Sort,
    on_sort: SortHandler,
}

impl RenderOnce for ColumnHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .test_support()
            .role(Role::Group)
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
        let extension = columns.extension.then(|| {
            self.cell(
                columns::extension_cell(),
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
        columns::line()
            .child(name)
            .children(extension)
            .children(size)
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
        let (arrow, accessible_label): (_, SharedString) = if active {
            let (id, icon, said) = match self.sort.direction {
                Direction::Ascending => ("sort-ascending", IconName::ArrowUp, "ascending"),
                Direction::Descending => ("sort-descending", IconName::ArrowDown, "descending"),
            };
            let arrow = div()
                .id(id)
                .test_support()
                .flex_shrink_0()
                .child(Icon::new(icon).size_3());
            (Some(arrow), format!("{label}, sorted {said}").into())
        } else {
            (None, label.into())
        };
        let (before, after) = if key == SortKey::Size {
            (arrow, None)
        } else {
            (None, arrow)
        };
        let on_sort = Rc::clone(&self.on_sort);

        cell.id(id)
            .test_support()
            .role(Role::Button)
            .aria_label(accessible_label)
            .h_full()
            .flex()
            .items_center()
            .gap_1()
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(if active { foreground } else { muted })
            .hover(|style| style.text_color(foreground))
            .on_click(move |_, window, cx| on_sort(&key, window, cx))
            .children(before)
            .child(div().min_w_0().truncate().child(label))
            .children(after)
    }
}
