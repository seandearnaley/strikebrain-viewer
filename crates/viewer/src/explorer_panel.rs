//! The explorer pane: the folder tree and its states.
//!
//! Top to bottom, the spec's regions are: toolbar, filter input, kind
//! control, column header, tree, and a selection bar floating over the
//! tree. This milestone draws the tree and the scan's progress bar, which
//! sits under the toolbar; the other regions slot in at the marked places
//! in [`ExplorerPanel::render`].

use std::ops::Range;

use explorer::{Explorer, NodeId};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, IntoElement, MouseButton,
    ParentElement as _, Render, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    UniformListScrollHandle, Window, div, prelude::*, uniform_list,
};

use crate::keys::{
    Collapse, EXPLORER, Expand, FocusFirst, FocusLast, FocusNext, FocusNextPage, FocusPrevious,
    FocusPreviousPage, OpenFolder, ShowFile, ToggleCheck,
};
use crate::row::{RowEvent, TreeRow};
use crate::state::{AppState, Motion, ScanStatus};

/// Rows a page key moves when the tree has not been laid out yet.
const FALLBACK_PAGE: usize = 10;

/// The explorer pane. Owns keyboard focus for the tree and its scroll
/// position; everything else lives in [`AppState`].
pub(crate) struct ExplorerPanel {
    state: Entity<AppState>,
    focus_handle: FocusHandle,
    scroll: UniformListScrollHandle,
    /// Whether the focus ring shows: set by a key press, cleared by a
    /// pointer press.
    ring: bool,
    _observe: Subscription,
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
    pub(crate) fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            focus_handle: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            ring: false,
            _observe: observe,
        }
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

    fn after_key(&mut self, row: Option<usize>, cx: &mut Context<Self>) {
        if let Some(row) = row {
            self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
        }
        self.ring = true;
        cx.notify();
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

    fn row_event(
        &mut self,
        id: NodeId,
        event: RowEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.state.update(cx, |state, cx| {
            state.focus_node(id, cx);
            match event {
                RowEvent::Click => state.show(id, cx),
                RowEvent::ToggleCheck => state.toggle_check(id, cx),
                RowEvent::ToggleOpen => state.toggle_folder(id, cx),
            }
        });
    }

    fn pointer_down(&mut self, cx: &mut Context<Self>) {
        if self.ring {
            self.ring = false;
            cx.notify();
        }
    }

    // Rendering

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<TreeRow> {
        let state = self.state.read(cx);
        let Some(explorer) = state.explorer() else {
            return Vec::new();
        };
        let (current, focus, ring) = (state.current(), state.focus(), self.ring);
        range
            .filter_map(|ix| explorer.row(ix))
            .map(|row| {
                let id = row.id;
                TreeRow::new(
                    &row,
                    cx.listener(move |this, event: &RowEvent, window, cx| {
                        this.row_event(id, *event, window, cx);
                    }),
                )
                .current(current == Some(id))
                .focused(focus == Some(id), ring)
            })
            .collect()
    }

    fn render_tree(&self, explorer: &Explorer, cx: &Context<Self>) -> AnyElement {
        let label: SharedString = explorer.tree().name(NodeId::ROOT).to_owned().into();
        div()
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.pointer_down(cx)),
            )
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                uniform_list(
                    "rows",
                    explorer.rows().len(),
                    cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
                )
                .track_scroll(&self.scroll)
                .size_full(),
            )
            .vertical_scrollbar(&self.scroll)
            .into_any_element()
    }

    /// The body when there is no tree. It holds keyboard focus in the tree's
    /// place, without the tree's key context, so ⌘O reaches the root view.
    fn render_without_tree(&self, content: Option<AnyElement>) -> AnyElement {
        div()
            .id("explorer-body")
            .track_focus(&self.focus_handle)
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
            .px_3()
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
        let state = self.state.read(cx);
        let slow = matches!(state.status(), ScanStatus::Scanning { slow: true });
        let body = match (state.status(), state.explorer()) {
            (ScanStatus::Failed(message), _) => {
                self.render_without_tree(Some(Self::render_error(message, cx)))
            }
            (_, Some(explorer)) => self.render_tree(explorer, cx),
            (ScanStatus::Idle, None) => self.render_without_tree(Some(Self::render_empty())),
            (ScanStatus::Scanning { .. }, None) => self.render_without_tree(None),
        };

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            // Toolbar (36 px) goes here.
            .when(slow, |panel| {
                panel.child(Progress::new("scan-progress").loading(true).xsmall())
            })
            // Filter input, kind control and column header go here, in that order.
            .child(body)
        // The selection bar floats over the tree, so it joins as an overlay.
    }
}
