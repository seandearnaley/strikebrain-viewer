//! The toolbar, 36 px: Expand all and Collapse all on the left; Add Folder,
//! Show ignored, Refresh and Select all (Clear while anything is checked)
//! on the right. Icon buttons, 24 px, each with a tooltip and an accessible
//! name.
//! The scan's progress bar runs along its bottom edge. Refresh shows in the
//! warning colour while the folder's changes are not followed.

use gpui_kit::assets::IconName as AllIcons;
use gpui_kit::component::button::Button;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::toolbar::Toolbar;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
};
use gpui_kit::{Context, IntoElement, ParentElement as _, Styled as _, div, prelude::*};

use super::ExplorerPanel;
use crate::keys::AddFolder;
use crate::state::{AppState, ScanStatus};

/// An icon button whose tooltip is also its accessible name. The toolbar
/// makes it a compact ghost button at its own size.
fn command(id: &'static str, icon: impl Into<Icon>, label: &'static str) -> Button {
    Button::new(id)
        .icon(icon)
        .tooltip(label)
        .accessibility_label(label)
}

impl ExplorerPanel {
    pub(super) fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let state = self.state.read(cx);
        let has_tree = state.explorer().is_some();
        let checked = state.selected_count() > 0;
        let shows_ignored = state.shows_ignored();
        let slow = matches!(state.status(), ScanStatus::Scanning { slow: true });

        let selection = if checked {
            command("clear", AllIcons::ListX, "Clear").on_click(cx.listener(|this, _, _, cx| {
                this.state.update(cx, AppState::clear_selection);
            }))
        } else {
            command("select-all", AllIcons::ListChecks, "Select all")
                .disabled(!has_tree)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.state.update(cx, AppState::select_all);
                }))
        };

        // Refresh turns the warning colour while a folder is not followed
        // on disk, and its tooltip says why.
        let refresh = match state.watch_error() {
            None => command("refresh", IconName::RefreshCw, "Refresh"),
            Some(message) => Button::new("refresh")
                .icon(Icon::new(IconName::RefreshCw).text_color(cx.theme().warning))
                .tooltip(format!(
                    "Refresh. Changes on disk are not followed: {message}"
                ))
                .accessibility_label("Refresh"),
        };

        let toolbar = Toolbar::new("toolbar")
            .h_9()
            .px_2()
            .py_0()
            .child(
                command("expand-all", IconName::ChevronsUpDown, "Expand all")
                    .disabled(!has_tree)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, AppState::expand_all);
                    })),
            )
            .child(
                command("collapse-all", AllIcons::ChevronsDownUp, "Collapse all")
                    .disabled(!has_tree)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, AppState::collapse_all);
                    })),
            )
            .content(div().flex_1())
            .child(
                command("add-folder", AllIcons::FolderPlus, "Add Folder")
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(AddFolder), cx)),
            )
            .child(
                command("show-ignored", IconName::Eye, "Show ignored")
                    .selected(shows_ignored)
                    .toggled(shows_ignored)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, AppState::toggle_show_ignored);
                    })),
            )
            .child(refresh.on_click(cx.listener(|this, _, _, cx| {
                this.state.update(cx, AppState::refresh);
            })))
            .child(selection);

        div()
            .relative()
            .flex_shrink_0()
            .child(toolbar)
            .when(slow, |bar| {
                bar.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .child(Progress::new("scan-progress").loading(true).xsmall()),
                )
            })
    }
}
