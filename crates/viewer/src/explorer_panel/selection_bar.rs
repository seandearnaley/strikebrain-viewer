//! The selection bar: "N selected" and a clear button, floating over the
//! bottom of the tree while anything is checked.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex};
use gpui_kit::{
    App, ClickEvent, InteractiveElement as _, IntoElement, ParentElement as _, Rems, RenderOnce,
    Role, SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window,
    div,
};

type ClearHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// The floating bar. It sits 8 px above the tree's bottom edge.
#[derive(IntoElement)]
pub(super) struct SelectionBar {
    count: usize,
    on_clear: ClearHandler,
}

impl SelectionBar {
    pub(super) fn new(
        count: usize,
        on_clear: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            count,
            on_clear: Rc::new(on_clear),
        }
    }

    /// The bottom padding the list takes while the bar shows: the bar's
    /// 32 px, its 8 px margin, and 8 px of air above it.
    pub(super) fn clearance() -> Rems {
        Rems(3.)
    }
}

impl RenderOnce for SelectionBar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let text: SharedString = format!("{} selected", self.count).into();
        let on_clear = self.on_clear;

        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_2()
            .flex()
            .justify_center()
            .child(
                h_flex()
                    .id("selection-bar")
                    .test_support()
                    .role(Role::Toolbar)
                    .aria_label(text.clone())
                    .occlude()
                    .h_8()
                    .pl_3()
                    .pr_1()
                    .gap_2()
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .text_color(theme.popover_foreground)
                    .shadow_md()
                    .child(div().text_sm().child(text))
                    .child(
                        Button::new("clear-selection")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip("Clear")
                            .accessibility_label("Clear")
                            .on_click(move |event, window, cx| on_clear(event, window, cx)),
                    ),
            )
    }
}
