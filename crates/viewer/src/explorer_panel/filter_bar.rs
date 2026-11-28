//! The filter block: a text input with a search icon and a clear button,
//! 32 px, and the kind control below it, three equal segments.
//!
//! The input sits in the [`FILTER`] key context: Esc there clears a query,
//! or with none moves focus to the tree.

use explorer::KindFilter;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::h_flex;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _, Window, div,
};

use super::ExplorerPanel;
use crate::keys::{CancelFilter, FILTER};

/// The kinds in the order the segments show them, with their labels.
const KINDS: [(KindFilter, &str); 3] = [
    (KindFilter::All, "All"),
    (KindFilter::Images, "Images"),
    (KindFilter::Text, "Text"),
];

impl ExplorerPanel {
    pub(super) fn render_filter_bar(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let kind = self.state.read(cx).kind();
        let selected = KINDS
            .iter()
            .position(|(each, _)| *each == kind)
            .unwrap_or_default();

        let input = div()
            .flex_1()
            .min_w_0()
            .key_context(FILTER)
            .on_action(cx.listener(Self::cancel_filter))
            .child(
                Input::new(if self.content.enabled {
                    &self.content.input
                } else {
                    &self.filter
                })
                .id("filter")
                .prefix(
                    Icon::new(IconName::Search)
                        .size_4()
                        .text_color(theme.muted_foreground),
                )
                .cleanable(true),
            );

        // A segment takes no keyboard focus on a click: focus stays in the
        // tree or the filter, as with the title bar's buttons.
        let kinds = div()
            .id("kind")
            .capture_any_mouse_down(|_, window, _| window.prevent_default())
            .child(
                TabBar::new("kinds")
                    .segmented()
                    .small()
                    .w_full()
                    .selected_index(selected)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        if let Some((kind, _)) = KINDS.get(*ix) {
                            this.state.update(cx, |state, cx| state.set_kind(*kind, cx));
                        }
                    }))
                    .children(KINDS.map(|(_, label)| Tab::new().flex_1().label(label))),
            );

        // 8 px all round and between the input, the kinds and the header:
        // the input sits as far under the title bar's rule as from the
        // pane's sides.
        v_flex()
            .flex_shrink_0()
            .p_2()
            .gap_2()
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(input)
                    .child(self.scope_selector(cx)),
            )
            .children((!self.content.enabled).then_some(kinds))
    }

    fn scope_selector(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let contents = self.content.enabled;
        let panel = cx.weak_entity();
        Button::new("search-scope")
            .small()
            .ghost()
            .label(if contents { "Contents" } else { "File Names" })
            .accessibility_label("Search scope")
            .dropdown_menu(move |mut menu, _, _| {
                for (scope, label) in [(false, "File Names"), (true, "Contents")] {
                    let panel = panel.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label)
                            .checked(scope == contents)
                            .on_click(move |_, window, cx| {
                                let _alive = panel.update(cx, |this, cx| {
                                    this.content.enabled = scope;
                                    this.focus_filter(window, cx);
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            })
    }

    /// Esc in the filter input.
    fn cancel_filter(&mut self, _: &CancelFilter, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.enabled {
            self.dismiss_content(cx);
            return;
        }
        if self.filter.read(cx).value().is_empty() {
            self.focus_tree(window, cx);
        } else {
            self.clear_query(window, cx);
        }
    }
}
