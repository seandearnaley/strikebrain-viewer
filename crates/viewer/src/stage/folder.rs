//! The open folder's immediate children in display order. Both layouts borrow
//! the engine's order directly unless a filter needs a subset of it.

use std::path::PathBuf;

use explorer::{Direction, Explorer, Kind, NodeId, Sort, SortKey, format_size};
use gpui_kit::assets::IconName as Icons;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Selectable as _, Sizable as _, h_flex};
use gpui_kit::{
    Anchor, App, ClickEvent, Entity, IntoElement, ParentElement as _, Role, Styled as _,
    TestSupportExt as _, Window, container_query, div, prelude::*, px,
};

use crate::columns::{self, Columns};
use crate::state::{AppState, FolderLayout};
use crate::theme::TABULAR_FIGURES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Options {
    pub(super) sort: Sort,
    filter: u64,
    sized: bool,
}

impl Options {
    pub(super) fn new(e: &Explorer, id: NodeId) -> Self {
        Self {
            sort: e.sort(),
            filter: e.visibility().stamp(),
            sized: e.tree().size(id).is_some(),
        }
    }
}

pub(super) struct Folder {
    pub(super) id: NodeId,
    generation: u64,
    path: PathBuf,
    filtered: Option<Vec<NodeId>>,
    focus: Option<(usize, String)>,
}

impl Folder {
    pub(super) fn new(e: &Explorer, id: NodeId, generation: u64) -> Self {
        let filtered = e.is_filter_active().then(|| {
            e.children(id)
                .iter()
                .copied()
                .filter(|&child| e.visibility().is_visible(child))
                .collect()
        });
        Self {
            id,
            generation,
            path: e.tree().abs_path(id),
            filtered,
            focus: None,
        }
    }

    pub(super) fn is_current(&self, state: &AppState) -> bool {
        state.stage_view() == crate::StageView::Folder
            && state.staged() == (self.generation, Some(self.id))
    }

    pub(super) fn entries<'a>(&'a self, e: &'a Explorer) -> &'a [NodeId] {
        self.filtered
            .as_deref()
            .unwrap_or_else(|| e.children(self.id))
    }

    pub(super) fn empty_label(&self, e: &Explorer) -> &'static str {
        if e.tree().kind(self.id) == Kind::Unreadable {
            "Folder unavailable"
        } else if e.tree().is_partial(self.id) {
            "Reading folder…"
        } else if self.filtered.is_some() && !e.children(self.id).is_empty() {
            "No matching items"
        } else {
            "Empty folder"
        }
    }

    /// Recover the focused child by name, including when a rescan renumbers it.
    pub(super) fn carry_focus(&mut self, previous: &Self, e: &Explorer) -> bool {
        if self.path != previous.path {
            return false;
        }
        if let Some((_, name)) = &previous.focus {
            self.focus = self
                .entries(e)
                .iter()
                .position(|&id| e.tree().name(id) == name)
                .map(|ix| (ix, name.clone()));
        }
        true
    }

    pub(super) fn focused(&self) -> Option<usize> {
        self.focus.as_ref().map(|(ix, _)| *ix)
    }

    pub(super) fn focus(&mut self, ix: usize, e: &Explorer) {
        self.focus = self
            .entries(e)
            .get(ix)
            .map(|&id| (ix, e.tree().name(id).to_owned()));
    }
}

pub(super) fn toolbar(
    options: Options,
    layout: FolderLayout,
    count: usize,
    total: usize,
    state: &Entity<AppState>,
) -> impl IntoElement {
    let state = state.clone();
    div()
        .h_9()
        .w_full()
        .flex_shrink_0()
        .child(container_query(move |size, _, _| {
            toolbar_contents(options, layout, count, total, state, size.width >= px(360.))
        }))
}

fn toolbar_contents(
    options: Options,
    layout: FolderLayout,
    count: usize,
    total: usize,
    state: Entity<AppState>,
    labels: bool,
) -> impl IntoElement {
    let switch = ButtonGroup::new("folder-views").small().compact().children(
        [
            (
                "folder-gallery-view",
                "Gallery view",
                Icons::LayoutGrid,
                FolderLayout::Gallery,
            ),
            (
                "folder-list-view",
                "List view",
                Icons::List,
                FolderLayout::List,
            ),
        ]
        .map(|(id, label, icon, mode)| {
            Button::new(id)
                .icon(icon)
                .when(labels, |button| {
                    button.label(if mode == FolderLayout::Gallery {
                        "Gallery"
                    } else {
                        "List"
                    })
                })
                .accessibility_label(label)
                .tooltip(label)
                .selected(layout == mode)
                .on_click(move |_, window, cx| match mode {
                    FolderLayout::Gallery => {
                        window.dispatch_action(Box::new(crate::keys::FolderGalleryView), cx);
                    }
                    FolderLayout::List => {
                        window.dispatch_action(Box::new(crate::keys::FolderListView), cx);
                    }
                })
        }),
    );
    let items = if count != total {
        format!("{count} of {total} items")
    } else if count == 1 {
        "1 item".into()
    } else {
        format!("{count} items")
    };
    h_flex()
        .w_full()
        .flex_shrink_0()
        .h_9()
        .px_2()
        .gap_1()
        .child(div().flex_1().min_w_0().truncate().text_xs().child(items))
        .child(
            Button::new("folder-filter")
                .ghost()
                .small()
                .icon(Icons::ListFilter)
                .tooltip("Filter files")
                .accessibility_label("Filter files")
                .selected(options.filter != 0)
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::keys::FocusFilter), cx);
                }),
        )
        .child(sort_button(options.sort, state))
        .child(switch)
}

fn sort_button(sort: Sort, state: Entity<AppState>) -> impl IntoElement {
    Button::new("folder-sort")
        .small()
        .ghost()
        .icon(Icons::ArrowUpDown)
        .tooltip("Sort by")
        .accessibility_label("Sort by")
        .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
            let item = |label, next: Sort, checked| {
                let state = state.clone();
                PopupMenuItem::new(label)
                    .checked(checked)
                    .on_click(move |_, _, cx| state.update(cx, |s, cx| s.set_sort(next, cx)))
            };
            for (key, label) in [
                (SortKey::Name, "Name"),
                (SortKey::Type, "Type"),
                (SortKey::Size, "Size"),
            ] {
                menu = menu.item(item(label, Sort { key, ..sort }, sort.key == key));
            }
            menu = menu.separator();
            for (direction, label) in [
                (Direction::Ascending, "Ascending"),
                (Direction::Descending, "Descending"),
            ] {
                menu = menu.item(item(
                    label,
                    Sort { direction, ..sort },
                    sort.direction == direction,
                ));
            }
            menu
        })
}

pub(super) fn details_row<F: Fn(&ClickEvent, &mut Window, &mut App) + 'static>(
    e: &Explorer,
    id: NodeId,
    columns: Columns,
    focused: bool,
    open: F,
    cx: &App,
) -> impl IntoElement + use<F> {
    let tree = e.tree();
    let name = tree.name(id).to_owned();
    let ext = tree.extension(id);
    let (icon, kind) = match tree.kind(id) {
        Kind::Dir | Kind::Unreadable => (
            Icon::new(Icons::Folder).text_color(cx.theme().info),
            "Folder",
        ),
        Kind::Symlink => (Icon::new(Icons::FileSymlink), "Link"),
        Kind::File => (
            Icon::new(crate::row::file_icon(ext)),
            if ext.is_empty() { "File" } else { ext },
        ),
    };
    let muted = cx.theme().muted_foreground;
    let hover = cx.theme().muted;
    columns::line()
        .id(("folder-entry", id.index()))
        .test_support()
        .role(Role::ListItem)
        .aria_selected(focused)
        .aria_label(name.clone())
        .h_8()
        .text_sm()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .when(focused, |row| row.bg(cx.theme().accent))
        .child(
            h_flex()
                .min_w_0()
                .flex_1()
                .gap_2()
                .child(icon.size_4().flex_shrink_0())
                .child(div().truncate().child(name)),
        )
        .when(columns.extension, |row| {
            row.child(
                columns::extension_cell()
                    .text_xs()
                    .text_color(muted)
                    .child(kind.to_owned()),
            )
        })
        .when(columns.size, |row| {
            row.child(
                columns::size_cell()
                    .text_xs()
                    .text_color(muted)
                    .font_features(TABULAR_FIGURES.clone())
                    .child(
                        tree.display_size(id)
                            .map_or_else(|| "—".into(), format_size),
                    ),
            )
        })
        .on_click(open)
}
