use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::{Context, IntoElement, Render, TestSupportExt, Window, div, prelude::*, px};

/// Starting width of the explorer pane, in pixels.
pub const EXPLORER_WIDTH: f32 = 300.0;

/// The root view: explorer on the left, stage on the right.
pub struct AppView;

impl AppView {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Self
    }
}

impl Render for AppView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        h_resizable("panes")
            .child(
                resizable_panel()
                    .size(px(EXPLORER_WIDTH))
                    .child(div().id("explorer-pane").test_support().size_full()),
            )
            .child(
                div()
                    .id("stage-pane")
                    .test_support()
                    .size_full()
                    .into_any_element(),
            )
    }
}
