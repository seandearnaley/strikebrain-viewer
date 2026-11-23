//! The root view: the explorer and the stage side by side.

use gpui_kit::component::{h_resizable, resizable_panel};
use gpui_kit::{
    AppContext as _, Context, Entity, Focusable as _, IntoElement, ParentElement as _, Pixels,
    Render, SharedString, Styled as _, Subscription, TestSupportExt as _, Window, div, prelude::*,
    px,
};

use crate::explorer_panel::ExplorerPanel;
use crate::keys::OpenFolder;
use crate::stage::Stage;
use crate::state::{AppState, Launch};
use crate::theme;

/// Starting width of the explorer pane, in pixels.
pub const EXPLORER_WIDTH: f32 = 300.0;

/// Narrowest width the explorer pane can be dragged to, in pixels.
pub const EXPLORER_MIN_WIDTH: f32 = 220.0;

/// The root view: explorer on the left, stage on the right. Owns the
/// [`AppState`] both observe, keeps the window title on the folder's name,
/// and handles ⌘O.
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
        let explorer = cx.new(|cx| ExplorerPanel::new(app_state.clone(), cx));
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
}

impl Render for AppView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .on_action(cx.listener(Self::open_folder))
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
                                    .child(self.explorer.clone()),
                            ),
                    )
                    .child(
                        resizable_panel().child(
                            div()
                                .id("stage-pane")
                                .test_support()
                                .size_full()
                                .child(self.stage.clone()),
                        ),
                    ),
            )
    }
}
