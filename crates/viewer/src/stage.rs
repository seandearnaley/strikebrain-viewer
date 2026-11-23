//! The stage: the right pane, which shows one file at a time.
//!
//! For now it shows only the current file's name, centred, so `current` is
//! visible. Milestone 3 replaces this with the image, Markdown and code
//! viewers. It takes keyboard focus in the [`STAGE`] key context.
//!
//! The root view embeds it cached, so it redraws only when it notifies:
//! when the file it shows changes, not on every change to the state.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    Window, div, prelude::*,
};

use explorer::NodeId;

use crate::keys::STAGE;
use crate::state::AppState;

/// The stage pane. Draws nothing while no file is current.
pub(crate) struct Stage {
    state: Entity<AppState>,
    focus_handle: FocusHandle,
    /// What the stage last showed; it redraws only when this changes.
    shown: (u64, Option<NodeId>),
    /// How many times `render` ran: a probe for the view cache.
    renders: usize,
    _observe: Subscription,
}

impl std::fmt::Debug for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stage").finish_non_exhaustive()
    }
}

impl Focusable for Stage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Stage {
    pub(crate) fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        // The state changes on every key in the tree; the stage redraws only
        // when the file it shows does.
        let observe = cx.observe(&state, |this, state, cx| {
            let staged = state.read(cx).staged();
            if staged != this.shown {
                this.shown = staged;
                cx.notify();
            }
        });
        let shown = state.read(cx).staged();
        Self {
            state,
            focus_handle: cx.focus_handle(),
            shown,
            renders: 0,
            _observe: observe,
        }
    }

    /// How many times the stage has rendered.
    pub(crate) fn renders(&self) -> usize {
        self.renders
    }
}

impl Render for Stage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let state = self.state.read(cx);
        let name: Option<SharedString> = state
            .current()
            .and_then(|id| state.name(id))
            .map(|name| name.to_owned().into());
        let theme = cx.theme();

        div()
            .id("stage")
            .test_support()
            .track_focus(&self.focus_handle)
            .key_context(STAGE)
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
