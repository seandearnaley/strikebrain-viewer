//! The root view: the explorer and the stage side by side, and keyboard
//! focus moving between the panes: filter, tree, stage.
//!
//! Both panes are embedded as cached views: a frame reuses a pane's last
//! drawing unless that pane notified, so a key in the tree never rebuilds
//! the stage.

use gpui_kit::component::{h_resizable, resizable_panel};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _, Pixels,
    Render, SharedString, StyleRefinement, Styled as _, Subscription, TestSupportExt as _, Window,
    div, prelude::*, px,
};

use crate::explorer_panel::{ExplorerFocus, ExplorerPanel};
use crate::keys::{FocusExplorer, FocusFilter, NextPane, OpenFolder, PreviousPane};
use crate::stage::Stage;
use crate::state::{AppState, Launch};
use crate::theme;

/// Starting width of the explorer pane, in pixels.
pub const EXPLORER_WIDTH: f32 = 300.0;

/// Narrowest width the explorer pane can be dragged to, in pixels.
pub const EXPLORER_MIN_WIDTH: f32 = 220.0;

/// A pane that takes keyboard focus, in Tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pane {
    Filter,
    Tree,
    Stage,
}

/// The root view: explorer on the left, stage on the right. Owns the
/// [`AppState`] both observe, keeps the window title on the folder's name,
/// and handles ⌘O, ⌘F and moving between panes.
pub struct AppView {
    state: Entity<AppState>,
    explorer: Entity<ExplorerPanel>,
    stage: Entity<Stage>,
    /// The title last given to the window.
    title: Option<SharedString>,
    _subscriptions: [Subscription; 2],
}

impl std::fmt::Debug for AppView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppView")
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

impl AppView {
    /// Builds the window's content and starts what `launch` asks for. The
    /// tree takes keyboard focus.
    pub fn new(launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let app_state = cx.new(|cx| AppState::new(launch, cx));
        let explorer = cx.new(|cx| ExplorerPanel::new(app_state.clone(), window, cx));
        let stage = cx.new(|cx| Stage::new(app_state.clone(), cx));
        let subscriptions = [
            cx.observe_in(&app_state, window, |this, _, window, cx| {
                this.sync_title(window, cx);
            }),
            cx.observe_window_appearance(window, |_, window, cx| theme::follow_system(window, cx)),
        ];
        window.focus(&explorer.focus_handle(cx), cx);

        let mut view = Self {
            state: app_state,
            explorer,
            stage,
            title: None,
            _subscriptions: subscriptions,
        };
        view.sync_title(window, cx);
        view
    }

    /// The shared application state.
    #[must_use]
    pub fn state(&self) -> &Entity<AppState> {
        &self.state
    }

    /// How many times the stage pane has rendered: a probe that tests use
    /// to see that work in the explorer leaves the stage alone.
    #[must_use]
    pub fn stage_renders(&self, cx: &App) -> usize {
        self.stage.read(cx).renders()
    }

    /// The title this view gave the window: the open folder's name.
    #[must_use]
    pub fn title(&self) -> Option<&SharedString> {
        self.title.as_ref()
    }

    fn sync_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.state.read(cx).folder_name();
        if name.is_some() && name != self.title {
            if let Some(name) = &name {
                window.set_window_title(name);
            }
            self.title = name;
        }
    }

    fn open_folder(&mut self, _: &OpenFolder, _: &mut Window, cx: &mut Context<Self>) {
        self.state
            .update(cx, |_, cx| AppState::prompt_for_folder(cx));
    }

    // Panes

    fn next_pane(&mut self, _: &NextPane, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_pane(true, window, cx);
    }

    fn previous_pane(&mut self, _: &PreviousPane, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_pane(false, window, cx);
    }

    fn focus_filter(&mut self, _: &FocusFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_pane(Pane::Filter, window, cx);
    }

    fn focus_explorer(&mut self, _: &FocusExplorer, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_pane(Pane::Tree, window, cx);
    }

    /// The pane holding keyboard focus, if any.
    fn focused_pane(&self, window: &Window, cx: &App) -> Option<Pane> {
        match self.explorer.read(cx).focused(window, cx) {
            Some(ExplorerFocus::Filter) => Some(Pane::Filter),
            Some(ExplorerFocus::Tree) => Some(Pane::Tree),
            None if self.stage.focus_handle(cx).contains_focused(window, cx) => Some(Pane::Stage),
            None => None,
        }
    }

    /// Tab and shift-Tab: filter, tree, stage, round again. The filter is
    /// skipped while no folder is open.
    fn cycle_pane(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let order: &[Pane] = if self.explorer.read(cx).has_filter(cx) {
            &[Pane::Filter, Pane::Tree, Pane::Stage]
        } else {
            &[Pane::Tree, Pane::Stage]
        };
        let here = self
            .focused_pane(window, cx)
            .and_then(|pane| order.iter().position(|&each| each == pane));
        let next = match (here, forward) {
            (Some(ix), true) => (ix + 1) % order.len(),
            (Some(ix), false) => (ix + order.len() - 1) % order.len(),
            (None, true) => 0,
            (None, false) => order.len() - 1,
        };
        self.focus_pane(order[next], window, cx);
    }

    fn focus_pane(&mut self, pane: Pane, window: &mut Window, cx: &mut Context<Self>) {
        match pane {
            Pane::Filter => self
                .explorer
                .update(cx, |explorer, cx| explorer.focus_filter(window, cx)),
            Pane::Tree => self
                .explorer
                .update(cx, |explorer, cx| explorer.focus_tree(window, cx)),
            Pane::Stage => window.focus(&self.stage.focus_handle(cx), cx),
        }
    }
}

/// The layout of a cached pane: it fills its side of the split. A cached
/// view is laid out from this style instead of being measured.
fn pane() -> StyleRefinement {
    StyleRefinement::default().size_full()
}

impl Render for AppView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .on_action(cx.listener(Self::open_folder))
            .on_action(cx.listener(Self::next_pane))
            .on_action(cx.listener(Self::previous_pane))
            .on_action(cx.listener(Self::focus_filter))
            .on_action(cx.listener(Self::focus_explorer))
            .child(
                h_resizable("panes")
                    .child(
                        resizable_panel()
                            .size(px(EXPLORER_WIDTH))
                            .size_range(px(EXPLORER_MIN_WIDTH)..Pixels::MAX)
                            .child(
                                div()
                                    .id("explorer-pane")
                                    .test_support()
                                    .size_full()
                                    .child(self.explorer.clone().cached(pane())),
                            ),
                    )
                    .child(
                        resizable_panel().child(
                            div()
                                .id("stage-pane")
                                .test_support()
                                .size_full()
                                .child(self.stage.clone().cached(pane())),
                        ),
                    ),
            )
    }
}
