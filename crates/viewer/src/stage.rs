//! The stage: the right pane, which shows one file at a time.
//!
//! For now it shows only the current file's name, centred, so `current` is
//! visible. Milestone 3 replaces this with the image, Markdown and code
//! viewers.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    Context, Entity, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
    prelude::*,
};

use crate::state::AppState;

/// The stage pane. Draws nothing while no file is current.
pub(crate) struct Stage {
    state: Entity<AppState>,
    _observe: Subscription,
}

impl std::fmt::Debug for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stage").finish_non_exhaustive()
    }
}

impl Stage {
    pub(crate) fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            _observe: observe,
        }
    }
}

impl Render for Stage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let name: Option<SharedString> = state
            .current()
            .and_then(|id| state.name(id))
            .map(|name| name.to_owned().into());
        let theme = cx.theme();

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.background)
            .text_color(theme.foreground)
            .when_some(name, |stage, name| {
                stage.child(
                    div()
                        .id("stage-file")
                        .test_support()
                        .role(Role::Label)
                        .aria_label(name.clone())
                        .max_w_full()
                        .px_4()
                        .truncate()
                        .child(name),
                )
            })
    }
}
