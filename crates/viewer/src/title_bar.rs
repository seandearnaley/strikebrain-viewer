//! The window's title bar: the Kit's `TitleBar`, one row across the top of
//! the window above both panes, in place of the system's title bar and the
//! explorer's toolbar.
//!
//! Its part over the explorer is exactly as wide as the explorer, so the
//! rule beside it continues the split's divider: the traffic lights'
//! room and the sidebar button, then Add Folder and More at its right end,
//! and the scan's progress along its bottom edge. While the explorer is
//! hidden the part holds only the lights' room and the sidebar button, and
//! no rule is drawn. Its part over the stage holds Back and Forward, the
//! stage's title and Present. Its buttons dispatch the window's actions,
//! which the root view handles, so a button, a menu item and a key run the
//! same code.
//!
//! The Kit pads its bar 80 px on macOS (12 px elsewhere) for the traffic
//! lights; this bar zeroes that padding and keeps the room inside its
//! explorer part, which can then start at the window's edge. In fullscreen
//! no lights show and the Kit pads its inner row `rems(0.75)` itself.
//!
//! The split draws its divider as the first pixel of the stage's side, so
//! the rule is the stage part's left border rather than the explorer
//! part's right one: there it continues the divider exactly.

use gpui_kit::assets::IconName as Icons;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, TitleBar, h_flex,
};
use gpui_kit::{
    Action, Anchor, App, IntoElement, ParentElement as _, Pixels, Role, SharedString, Styled as _,
    TestSupportExt as _, Window, div, prelude::*, px, rems,
};

use crate::keys::{
    AddFolder, CollapseAll, DeselectAll, ExpandAll, GoBack, GoForward, Refresh, SelectAllFiles,
    StartPresenting, ToggleShowIgnored, ToggleSidebar,
};
use crate::state::{AppState, ScanStatus};

/// The traffic lights' room at the bar's left, as the Kit's bar leaves it:
/// none in fullscreen.
fn lead(fullscreen: bool) -> Pixels {
    if fullscreen {
        px(0.)
    } else if cfg!(target_os = "macos") {
        px(80.)
    } else {
        px(12.)
    }
}

/// A 24 px ghost icon button whose tooltip is its accessible name, and
/// which dispatches `action`.
fn command(
    id: &'static str,
    icon: impl Into<Icon>,
    label: &'static str,
    action: impl Action,
) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .icon(icon)
        .tooltip(label)
        .accessibility_label(label)
        .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
}

/// The bar for `state`. Its part over the explorer is `explorer` wide, or
/// `None` while the explorer is hidden.
pub(crate) fn window_bar(
    state: &AppState,
    explorer: Option<Pixels>,
    window: &Window,
    cx: &App,
) -> TitleBar {
    let fullscreen = window.is_fullscreen();
    // In fullscreen the Kit's inner row starts this far in.
    let inset = if fullscreen {
        rems(0.75).to_pixels(window.rem_size())
    } else {
        px(0.)
    };
    TitleBar::new()
        .bg(cx.theme().background)
        .border_color(cx.theme().border)
        .pl_0()
        .child(explorer_part(
            state,
            explorer.map(|width| width - inset),
            lead(fullscreen),
            cx,
        ))
        .child(stage_part(state, explorer.is_some(), cx))
}

/// The part over the explorer: `width` wide, or only the lights' room and
/// the sidebar button while the explorer is hidden.
fn explorer_part(
    state: &AppState,
    width: Option<Pixels>,
    lead: Pixels,
    cx: &App,
) -> impl IntoElement {
    let part = h_flex()
        .id("bar-explorer")
        .test_support()
        .h_full()
        .flex_shrink_0()
        .items_center()
        .gap_1()
        // The lights' room, before the sidebar button.
        .pl(lead)
        .child(sidebar_button(width.is_none()));
    let Some(width) = width else {
        return part;
    };
    let open = !state.is_empty();
    let slow = matches!(state.status(), ScanStatus::Scanning { slow: true });
    let warning = state.watch_error().cloned();
    part.w(width)
        .pr_2()
        .relative()
        // Empty, so the window moves from here.
        .child(div().flex_1())
        .when_some(warning, |part, message| {
            part.child(warning_refresh(&message, cx))
        })
        .when(open, |part| {
            part.child(command(
                "add-folder",
                Icons::FolderPlus,
                "Add Folder",
                AddFolder,
            ))
            .child(more(state))
        })
        .when(slow, |part| {
            part.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .child(Progress::new("scan-progress").loading(true).xsmall()),
            )
        })
}

/// Hides or shows the explorer: the Kit's panel icons. A Kit `Button`, not
/// the Kit's `SidebarToggleButton`, which in 0.10.0 takes no tooltip.
fn sidebar_button(hidden: bool) -> Button {
    let (icon, label) = if hidden {
        (Icons::PanelLeftOpen, "Show Sidebar")
    } else {
        (Icons::PanelLeftClose, "Hide Sidebar")
    };
    command("sidebar", icon, label, ToggleSidebar)
}

/// Refresh in the warning colour, while a folder's changes are not
/// followed: its tooltip says why.
fn warning_refresh(message: &SharedString, cx: &App) -> Button {
    Button::new("refresh")
        .ghost()
        .small()
        .icon(Icon::new(IconName::RefreshCw).text_color(cx.theme().warning))
        .tooltip(format!(
            "Refresh. Changes on disk are not followed: {message}"
        ))
        .accessibility_label("Refresh")
        .on_click(|_, window, cx| window.dispatch_action(Box::new(Refresh), cx))
}

/// More (…): the explorer's commands, each showing its key.
fn more(state: &AppState) -> impl IntoElement {
    let has_tree = state.explorer().is_some();
    let checked = state.selected_count() > 0;
    let shows_ignored = state.shows_ignored();
    Button::new("more")
        .ghost()
        .small()
        .icon(Icons::Ellipsis)
        .tooltip("More")
        .accessibility_label("More")
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            menu.menu_with_disabled("Expand All", Box::new(ExpandAll), !has_tree)
                .menu_with_disabled("Collapse All", Box::new(CollapseAll), !has_tree)
                .separator()
                .menu_with_check("Show Ignored", shows_ignored, Box::new(ToggleShowIgnored))
                .menu("Refresh", Box::new(Refresh))
                .separator()
                .menu_with_disabled("Select All", Box::new(SelectAllFiles), !has_tree)
                .menu_with_disabled("Deselect All", Box::new(DeselectAll), !checked)
        })
}

/// The part over the stage: Back and Forward, its title and Present.
/// Beside the explorer its left border is the rule that continues the
/// split's divider.
fn stage_part(state: &AppState, beside_explorer: bool, cx: &App) -> impl IntoElement {
    h_flex()
        .id("bar-stage")
        .test_support()
        .flex_1()
        .min_w_0()
        .h_full()
        .items_center()
        .gap_1()
        .px_2()
        .when(beside_explorer, |part| {
            part.border_l_1().border_color(cx.theme().border)
        })
        .child(command("back", Icons::ChevronLeft, "Back", GoBack).disabled(!state.can_go_back()))
        .child(
            command("forward", Icons::ChevronRight, "Forward", GoForward)
                .disabled(!state.can_go_forward()),
        )
        .when_some(state.stage_title(), |part, title| {
            part.child(
                div()
                    .id("stage-title")
                    .test_support()
                    .role(Role::Label)
                    .aria_label(title.clone())
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(title),
            )
        })
        // Empty, so the window moves from here.
        .child(div().flex_1())
        .child(
            command("present", Icons::Play, "Present", StartPresenting)
                .disabled(state.current().is_none()),
        )
}
