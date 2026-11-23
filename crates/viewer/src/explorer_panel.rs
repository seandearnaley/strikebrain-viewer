//! The explorer pane: the folder tree and the controls above it.
//!
//! Top to bottom, the spec's regions: toolbar (`toolbar.rs`), filter input
//! and kind control (`filter_bar.rs`), column header (`header.rs`), the
//! tree, and a selection bar floating over the tree (`selection_bar.rs`).
//! With no folder the pane shows only "Open Folder".

mod filter_bar;
mod header;
mod selection_bar;
mod toolbar;

use std::ops::Range;

use explorer::{Explorer, NodeId};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    UniformListScrollHandle, Window, div, prelude::*, uniform_list,
};

use crate::keys::{
    ClearFilter, Collapse, EXPLORER, EXPLORER_BODY, Expand, FocusFirst, FocusLast, FocusNext,
    FocusNextPage, FocusPrevious, FocusPreviousPage, OpenFolder, ShowFile, ToggleCheck,
};
use crate::row::TreeRow;
use crate::state::{AppState, DrawnRow, Motion, RowAction, ScanStatus, StateEvent};

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
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter"));
        let subscriptions = [
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&state, |this, _, event: &StateEvent, cx| match event {
                StateEvent::RowsReplaced => this.reveal_focus(cx),
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
            scroll: UniformListScrollHandle::new(),
            ring: false,
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

    /// Whether the filter input is on screen: a folder is open.
    pub(crate) fn has_filter(&self, cx: &App) -> bool {
        !self.state.read(cx).is_empty()
    }

    /// Focuses the filter input with its text selected, so typing replaces
    /// the query.
    pub(crate) fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_filter(cx) {
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
        let row = self
            .state
            .update(cx, |state, cx| state.move_focus(Motion::Up(1), cx));
        self.after_key(row, cx);
    }

    fn focus_next(&mut self, _: &FocusNext, _: &mut Window, cx: &mut Context<Self>) {
        let row = self
            .state
            .update(cx, |state, cx| state.move_focus(Motion::Down(1), cx));
        self.after_key(row, cx);
    }

    fn focus_first(&mut self, _: &FocusFirst, _: &mut Window, cx: &mut Context<Self>) {
        let row = self
            .state
            .update(cx, |state, cx| state.move_focus(Motion::First, cx));
        self.after_key(row, cx);
    }

    fn focus_last(&mut self, _: &FocusLast, _: &mut Window, cx: &mut Context<Self>) {
        let row = self
            .state
            .update(cx, |state, cx| state.move_focus(Motion::Last, cx));
        self.after_key(row, cx);
    }

    fn focus_previous_page(
        &mut self,
        _: &FocusPreviousPage,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = self.page(cx);
        let row = self
            .state
            .update(cx, |state, cx| state.move_focus(Motion::Up(page), cx));
        self.after_key(row, cx);
    }

    fn focus_next_page(&mut self, _: &FocusNextPage, _: &mut Window, cx: &mut Context<Self>) {
        let page = self.page(cx);
        let row = self
            .state
            .update(cx, |state, cx| state.move_focus(Motion::Down(page), cx));
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

    /// Empties the filter input and the query.
    fn clear_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state.update(cx, |state, cx| state.set_query("", cx));
    }

    /// How many whole rows fit in the tree's viewport, from the last layout.
    /// The list records its viewport and content heights; the content is
    /// one row height per row.
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a row count past 2^24 only blurs a fraction of a row; `fit` is a whole, positive number of rows"
    )]
    fn page(&self, cx: &App) -> usize {
        let rows = self
            .state
            .read(cx)
            .explorer()
            .map_or(0, |explorer| explorer.rows().len());
        let Some(size) = self.scroll.0.borrow().last_item_size else {
            return FALLBACK_PAGE;
        };
        if rows == 0 {
            return FALLBACK_PAGE;
        }
        let row_height = size.contents.height / rows as f32;
        let fit = (size.item.height / row_height).floor();
        if fit >= 1. { fit as usize } else { 1 }
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

    /// Any pointer press in the pane hides the focus ring.
    fn pointer_down(&mut self, cx: &mut Context<Self>) {
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
        let (current, focus) = (state.current(), state.focus());
        range
            .filter_map(|ix| Some((ix, explorer.row(ix)?)))
            .map(|(ix, row)| {
                let id = row.id;
                let drawn = state.drawn_row(ix, id);
                TreeRow::new(
                    &row,
                    cx.listener(move |this, action: &RowAction, window, cx| {
                        this.row_action(drawn, *action, window, cx);
                    }),
                )
                .current(current == Some(id))
                .focused(focus == Some(id), ring)
            })
            .collect()
    }

    /// The tree, with the selection bar floating over its bottom while
    /// anything is checked. The list gains bottom padding while the bar
    /// shows, so the last row can scroll clear of it; no row moves.
    fn render_tree(&self, explorer: &Explorer, selected: usize, cx: &Context<Self>) -> AnyElement {
        let label: SharedString = explorer.tree().name(NodeId::ROOT).to_owned().into();
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

    /// The operating system's message when the folder cannot be read.
    fn render_error(message: &SharedString, cx: &App) -> AnyElement {
        div()
            .id("scan-error")
            .test_support()
            .role(Role::Label)
            .aria_label(message.clone())
            .w_full()
            .px_2()
            .py_2()
            .truncate()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(message.clone())
            .into_any_element()
    }

    /// No folder: one "Open Folder" button, centred.
    fn render_empty() -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                Button::new("open-folder")
                    .primary()
                    .label("Open Folder")
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenFolder), cx)),
            )
            .into_any_element()
    }
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
        let (header, body) = match (state.status(), state.explorer()) {
            (ScanStatus::Failed(message), _) => (
                None,
                self.render_without_tree(Some(Self::render_error(message, cx))),
            ),
            (_, Some(explorer)) => (
                Some(self.render_header(explorer.sort(), cx)),
                self.render_tree(explorer, state.selected_count(), cx),
            ),
            (_, None) => (None, self.render_without_tree(None)),
        };

        panel
            .child(self.render_toolbar(cx))
            .child(self.render_filter_bar(cx))
            .children(header)
            .child(body)
    }
}
