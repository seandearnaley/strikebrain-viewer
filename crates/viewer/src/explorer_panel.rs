//! The explorer pane: the folder tree and the controls above it.
//!
//! Top to bottom, the spec's regions: filter input and kind control
//! (`filter_bar.rs`), column header (`header.rs`), the tree, and a
//! selection bar floating over the tree (`selection_bar.rs`). The pane
//! starts at the filter: its commands are in the window's title bar
//! (`title_bar.rs`). With no folder the pane shows only "Add Folder".

mod filter_bar;
mod header;
mod selection_bar;

use std::ops::Range;
use std::rc::Rc;

use explorer::{Explorer, NodeId};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    UniformListScrollHandle, Window, div, point, prelude::*, uniform_list,
};

use crate::keys::{
    AddFolder, ClearFilter, Collapse, EXPLORER, EXPLORER_BODY, Expand, FocusFirst, FocusLast,
    FocusNext, FocusNextPage, FocusPrevious, FocusPreviousPage, RemoveFolder, ShowFile,
    ToggleCheck,
};
use crate::row::TreeRow;
use crate::state::{AppState, DrawnRow, Motion, RowAction, StateEvent};

use selection_bar::SelectionBar;

/// Rows a page key moves when the tree has not been laid out yet.
const FALLBACK_PAGE: usize = 10;

/// The part of the explorer pane that holds keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExplorerFocus {
    /// The filter input.
    Filter,
    /// The tree, or the pane's body when there is no tree.
    Tree,
}

/// The explorer pane. Owns keyboard focus for the tree, the filter input's
/// state and the tree's scroll position; everything else lives in
/// [`AppState`].
pub(crate) struct ExplorerPanel {
    state: Entity<AppState>,
    focus_handle: FocusHandle,
    filter: Entity<InputState>,
    scroll: UniformListScrollHandle,
    /// Whether the focus ring shows: set by a key press that moves or lands
    /// focus in the tree, cleared by any pointer press in the pane.
    ring: bool,
    /// The row under the pointer, which shows its remove button if it is
    /// a top-level folder's.
    hovered: Option<NodeId>,
    _subscriptions: [Subscription; 3],
}

impl std::fmt::Debug for ExplorerPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExplorerPanel")
            .field("ring", &self.ring)
            .finish_non_exhaustive()
    }
}

impl Focusable for ExplorerPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ExplorerPanel {
    pub(crate) fn new(
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // A restored filter shows in the input from the start.
        let query = state.read(cx).query().to_owned();
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Filter")
                .default_value(query)
        });
        let scroll = UniformListScrollHandle::new();
        let probe = scroll.clone();
        state.update(cx, |state, _| {
            state.probe_top_row(Rc::new(move |rows| top_index(&probe, rows)));
        });
        let subscriptions = [
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&state, |this, _, event: &StateEvent, cx| match event {
                StateEvent::RowsReplaced => this.reveal_focus(cx),
                StateEvent::CurrentMoved => this.reveal_current(cx),
                StateEvent::TopRow(row) => {
                    // Strict: at the top even when it is already on screen.
                    this.scroll.scroll_to_item_strict(*row, ScrollStrategy::Top);
                }
                StateEvent::RowsMoved {
                    top,
                    rows_before,
                    focus_before,
                } => this.rows_moved(*top, *rows_before, *focus_before, cx),
                // The gallery's own, and the window's.
                StateEvent::RevealTile | StateEvent::FolderAdded => {}
            }),
            cx.subscribe(&filter, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value();
                    this.state
                        .update(cx, |state, cx| state.set_query(&text, cx));
                }
            }),
        ];
        Self {
            state,
            focus_handle: cx.focus_handle(),
            filter,
            scroll,
            ring: false,
            hovered: None,
            _subscriptions: subscriptions,
        }
    }

    // Focus, for the panes' Tab order and ⌘F

    /// Which part of the pane holds keyboard focus, if any.
    pub(crate) fn focused(&self, window: &Window, cx: &App) -> Option<ExplorerFocus> {
        if self.filter.focus_handle(cx).contains_focused(window, cx) {
            Some(ExplorerFocus::Filter)
        } else if self.focus_handle.contains_focused(window, cx) {
            Some(ExplorerFocus::Tree)
        } else {
            None
        }
    }

    /// Whether the filter input is on screen: anything but the "Add
    /// Folder" state.
    pub(crate) fn filter_on_screen(&self, cx: &App) -> bool {
        !self.state.read(cx).is_empty()
    }

    /// Focuses the filter input with its text selected, so typing replaces
    /// the query.
    pub(crate) fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.filter_on_screen(cx) {
            return;
        }
        self.filter.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
    }

    /// Focuses the tree from the keyboard: the focus ring shows.
    pub(crate) fn focus_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        let row = self.state.read(cx).focus_index();
        self.after_key(row, cx);
    }

    // Keys: each moves or changes the focus row, shows the ring and keeps
    // the focus row on screen.

    fn focus_previous(&mut self, _: &FocusPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(Motion::Up(1), cx);
    }

    fn focus_next(&mut self, _: &FocusNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(Motion::Down(1), cx);
    }

    fn focus_first(&mut self, _: &FocusFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(Motion::First, cx);
    }

    fn focus_last(&mut self, _: &FocusLast, _: &mut Window, cx: &mut Context<Self>) {
        self.move_focus(Motion::Last, cx);
    }

    fn focus_previous_page(
        &mut self,
        _: &FocusPreviousPage,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = self.page(cx);
        self.move_focus(Motion::Up(page), cx);
    }

    fn focus_next_page(&mut self, _: &FocusNextPage, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.page(cx);
        self.move_focus(Motion::Down(page), cx);
    }

    /// The one step behind the six focus keys.
    fn move_focus(&mut self, motion: Motion, cx: &mut Context<Self>) {
        let row = self
            .state
            .update(cx, |state, cx| state.move_focus(motion, cx));
        self.after_key(row, cx);
    }

    fn expand(&mut self, _: &Expand, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.state.update(cx, AppState::expand);
        self.after_key(row, cx);
    }

    fn collapse(&mut self, _: &Collapse, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.state.update(cx, AppState::collapse);
        self.after_key(row, cx);
    }

    fn toggle_check(&mut self, _: &ToggleCheck, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, AppState::toggle_focus_check);
        let row = self.state.read(cx).focus_index();
        self.after_key(row, cx);
    }

    fn show_file(&mut self, _: &ShowFile, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, AppState::show_focus);
        let row = self.state.read(cx).focus_index();
        self.after_key(row, cx);
    }

    /// ⌘⌫ in the tree: removes the focused top-level folder.
    fn remove_folder(&mut self, _: &RemoveFolder, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, AppState::remove_focus_folder);
        let row = self.state.read(cx).focus_index();
        self.after_key(row, cx);
    }

    /// Esc in the tree: clears the filter, the query and the kind.
    fn clear_filter(&mut self, _: &ClearFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, AppState::clear_filter);
        self.filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        let row = self.state.read(cx).focus_index();
        self.after_key(row, cx);
    }

    fn after_key(&mut self, row: Option<usize>, cx: &mut Context<Self>) {
        if let Some(row) = row {
            self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
        }
        self.ring = true;
        cx.notify();
    }

    /// Keeps the focus row on screen after the rows were rebuilt.
    fn reveal_focus(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.state.read(cx).focus_index() {
            self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
        }
    }

    /// After a change of folders: the row that was at the top stays there,
    /// moved by as many rows as it moved, sub-row offset and all; a focus
    /// row that was on screen stays on screen, untouched if it kept its
    /// place below the top row. With no top row to keep, the focus row is
    /// kept on screen as after any rebuild.
    fn rows_moved(
        &mut self,
        top: Option<(usize, usize)>,
        rows_before: usize,
        focus_before: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some((from, to)) = top else {
            return self.reveal_focus(cx);
        };
        if from != to
            && let Some(height) = row_height(&self.scroll, rows_before)
        {
            let handle = &self.scroll.0.borrow().base_handle;
            let offset = handle.offset();
            handle.set_offset(point(offset.x, offset.y - height * rows_between(from, to)));
        }
        // A focus row on screen that kept its place below the top row is
        // where it was; one that moved is brought back into view.
        let page = page_rows(&self.scroll, rows_before).unwrap_or(FALLBACK_PAGE);
        let focus_now = self.state.read(cx).focus_index();
        let was_shown = focus_before.filter(|focus| (from..from + page).contains(focus));
        if let Some(before) = was_shown
            && focus_now.is_none_or(|now| now.wrapping_sub(to) != before - from)
        {
            self.reveal_focus(cx);
        }
    }

    /// Keeps the stage's file on screen while the stage's keys move it: the
    /// row scrolls into view if it is a row. A file inside a closed folder
    /// or hidden by the filter stays out of sight; nothing opens.
    fn reveal_current(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let row = state
            .explorer()
            .zip(state.current())
            .and_then(|(explorer, current)| explorer.row_of(current));
        if let Some(row) = row {
            self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
        }
    }

    /// Empties the filter input and the query.
    fn clear_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state.update(cx, |state, cx| state.set_query("", cx));
    }

    /// How many whole rows fit in the tree's viewport, from the last layout.
    fn page(&self, cx: &App) -> usize {
        let rows = self
            .state
            .read(cx)
            .explorer()
            .map_or(0, |explorer| explorer.rows().len());
        page_rows(&self.scroll, rows).unwrap_or(FALLBACK_PAGE)
    }

    // Pointer

    /// A press on a row the last frame drew. It does nothing when the row
    /// is gone since: input arrives before the next frame.
    fn row_action(
        &mut self,
        row: DrawnRow,
        action: RowAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .state
            .update(cx, |state, cx| state.row_action(row, action, cx))
        {
            window.focus(&self.focus_handle, cx);
        }
    }

    /// The pointer entered or left the row of `id`.
    fn hover(&mut self, id: NodeId, hovered: bool, cx: &mut Context<Self>) {
        let now = match (hovered, self.hovered) {
            (true, _) => Some(id),
            (false, Some(was)) if was == id => None,
            (false, was) => was,
        };
        if now != self.hovered {
            self.hovered = now;
            cx.notify();
        }
    }

    /// Any pointer press in the pane, or on the window's title bar, hides
    /// the focus ring.
    pub(crate) fn pointer_down(&mut self, cx: &mut Context<Self>) {
        if self.ring {
            self.ring = false;
            cx.notify();
        }
    }

    // Rendering

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<TreeRow> {
        let state = self.state.read(cx);
        let Some(explorer) = state.explorer() else {
            return Vec::new();
        };
        let ring = self.ring && self.focus_handle.is_focused(window);
        let (current, focus, hovered) = (state.current(), state.focus(), self.hovered);
        range
            .filter_map(|ix| Some((ix, explorer.row(ix)?)))
            .map(|(ix, row)| {
                let id = row.id;
                let drawn = state.drawn_row(ix, id);
                let folder = state.is_folder_row(id);
                // Focused as the ring shows it: after a key press.
                let shown = hovered == Some(id) || (ring && focus == Some(id));
                TreeRow::new(
                    &row,
                    cx.listener(move |this, action: &RowAction, window, cx| {
                        this.row_action(drawn, *action, window, cx);
                    }),
                )
                .current(current == Some(id))
                .focused(focus == Some(id), ring)
                .when(folder, |row| {
                    row.tooltip(state.folder_tooltip(id))
                        .removable(shown)
                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                            this.hover(id, *hovered, cx);
                        }))
                })
            })
            .collect()
    }

    /// The tree, with the selection bar floating over its bottom while
    /// anything is checked. The list gains bottom padding while the bar
    /// shows, so the last row can scroll clear of it; no row moves.
    fn render_tree(
        &self,
        explorer: &Explorer,
        label: SharedString,
        selected: usize,
        cx: &Context<Self>,
    ) -> AnyElement {
        let tree = div()
            .id("tree")
            .test_support()
            .role(Role::Tree)
            .aria_label(label)
            .track_focus(&self.focus_handle)
            .key_context(EXPLORER)
            .on_action(cx.listener(Self::focus_previous))
            .on_action(cx.listener(Self::focus_next))
            .on_action(cx.listener(Self::focus_first))
            .on_action(cx.listener(Self::focus_last))
            .on_action(cx.listener(Self::focus_previous_page))
            .on_action(cx.listener(Self::focus_next_page))
            .on_action(cx.listener(Self::expand))
            .on_action(cx.listener(Self::collapse))
            .on_action(cx.listener(Self::toggle_check))
            .on_action(cx.listener(Self::show_file))
            .on_action(cx.listener(Self::clear_filter))
            .on_action(cx.listener(Self::remove_folder))
            .relative()
            .size_full()
            .child(
                uniform_list(
                    "rows",
                    explorer.rows().len(),
                    cx.processor(|this, range, window, cx| this.render_rows(range, window, cx)),
                )
                .track_scroll(&self.scroll)
                .size_full()
                .when(selected > 0, |list| list.pb(SelectionBar::clearance())),
            )
            .vertical_scrollbar(&self.scroll);

        div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(tree)
            .when(selected > 0, |area| {
                area.child(SelectionBar::new(
                    selected,
                    cx.listener(|this, _, _, cx| this.state.update(cx, AppState::clear_selection)),
                ))
            })
            .into_any_element()
    }

    /// The body when there is no tree. It holds keyboard focus in the tree's
    /// place, in its own key context: Tab, shift-Tab and ⌘F reach the root
    /// view, and the tree's keys have nothing to act on.
    fn render_without_tree(&self, content: Option<AnyElement>) -> AnyElement {
        div()
            .id("explorer-body")
            .test_support()
            .track_focus(&self.focus_handle)
            .key_context(EXPLORER_BODY)
            .flex_1()
            .min_h_0()
            .w_full()
            .children(content)
            .into_any_element()
    }

    /// No folder: one "Add Folder" button, centred.
    fn render_empty() -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                Button::new("add-folder")
                    .primary()
                    .label("Add Folder")
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(AddFolder), cx)),
            )
            .into_any_element()
    }
}

/// The row at the top of a list of `rows` rows scrolled by `scroll`, from
/// its last layout; `None` before it was laid out.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the quotient is a whole, non-negative row count, clamped below"
)]
fn top_index(scroll: &UniformListScrollHandle, rows: usize) -> Option<usize> {
    let height = row_height(scroll, rows)?;
    let scrolled = -scroll.0.borrow().base_handle.offset().y;
    let top = (scrolled / height).floor().max(0.) as usize;
    Some(top.min(rows.saturating_sub(1)))
}

/// How many whole rows of a list of `rows` rows fit in its viewport, from
/// its last layout: the list records its viewport and content heights, and
/// the content is one row height per row.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "`fit` is a whole, positive number of rows"
)]
fn page_rows(scroll: &UniformListScrollHandle, rows: usize) -> Option<usize> {
    let viewport = scroll.0.borrow().last_item_size?.item.height;
    let fit = (viewport / row_height(scroll, rows)?).floor();
    Some(if fit >= 1. { fit as usize } else { 1 })
}

/// `to - from` rows, as a multiplier of a row's height.
#[expect(
    clippy::cast_precision_loss,
    reason = "a row count past 2^24 only blurs a fraction of a row"
)]
fn rows_between(from: usize, to: usize) -> f32 {
    to as f32 - from as f32
}

/// One row's height in a list of `rows` rows, from its last layout.
#[expect(
    clippy::cast_precision_loss,
    reason = "a row count past 2^24 only blurs a fraction of a row"
)]
fn row_height(scroll: &UniformListScrollHandle, rows: usize) -> Option<gpui_kit::Pixels> {
    let size = scroll.0.borrow().last_item_size?;
    (rows > 0).then(|| size.contents.height / rows as f32)
}

impl Render for ExplorerPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = v_flex()
            .size_full()
            .bg(cx.theme().background)
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| this.pointer_down(cx)));

        let state = self.state.read(cx);
        if state.is_empty() {
            return panel.child(self.render_without_tree(Some(Self::render_empty())));
        }
        // The header labels rows, so it shows only with a tree.
        let (header, body) = match state.explorer() {
            Some(explorer) => (
                Some(Self::render_header(explorer.sort(), cx)),
                self.render_tree(explorer, state.title(), state.selected_count(), cx),
            ),
            None => (None, self.render_without_tree(None)),
        };

        panel
            .child(self.render_filter_bar(cx))
            .children(header)
            .child(body)
    }
}
