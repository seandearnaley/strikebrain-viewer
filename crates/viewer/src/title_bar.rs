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
//! stage's title and Present, and left of Present an update ready to
//! install. Its buttons dispatch the window's actions, which the root view
//! handles, so a button, a menu item and a key run the same code. Windows
//! and Linux draw no menu bar, so there More ends with the update commands,
//! and shows with nothing open too, holding only them.
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
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, TitleBar, h_flex,
};
use gpui_kit::{
    Action, Anchor, App, IntoElement, ParentElement as _, Pixels, Role, SharedString, Styled as _,
    TestSupportExt as _, Window, div, prelude::*, px, rems,
};

use crate::keys::{
    AddFolder, CheckForUpdates, CollapseAll, DeselectAll, ExpandAll, GoBack, GoForward,
    InstallUpdate, Refresh, SelectAllFiles, StartPresenting, ToggleCheckAutomatically,
    ToggleShowIgnored, ToggleSidebar,
};
use crate::state::{AppState, ScanStatus};
use crate::update::{How, Offer, Updater};

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
/// `None` while the explorer is hidden. With an `updater`, it shows the
/// update ready to install and, where there is no menu bar, the update
/// commands.
pub(crate) fn window_bar(
    app_state: &gpui_kit::Entity<AppState>,
    explorer: Option<Pixels>,
    updater: Option<&Updater>,
    window: &Window,
    cx: &App,
) -> TitleBar {
    let state = app_state.read(cx);
    let fullscreen = window.is_fullscreen();
    // In fullscreen the Kit's inner row starts this far in.
    let inset = if fullscreen {
        rems(0.75).to_pixels(window.rem_size())
    } else {
        px(0.)
    };
    // macOS has the update commands in its app menu.
    let automatic = updater
        .filter(|_| !cfg!(target_os = "macos"))
        .map(Updater::automatic);
    TitleBar::new()
        .bg(cx.theme().background)
        .border_color(cx.theme().border)
        .pl_0()
        .child(explorer_part(
            state,
            explorer.map(|width| width - inset),
            lead(fullscreen),
            automatic,
            cx,
        ))
        .child(stage_part(app_state, explorer.is_some(), updater, cx))
}

/// The part over the explorer: `width` wide, or only the lights' room and
/// the sidebar button while the explorer is hidden. With `automatic`, More
/// ends with the update commands, and shows with nothing open, holding only
/// them.
fn explorer_part(
    state: &AppState,
    width: Option<Pixels>,
    lead: Pixels,
    automatic: Option<bool>,
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
    let slow =
        state.explorer().is_none() && matches!(state.status(), ScanStatus::Scanning { slow: true });
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
        })
        // Where More holds the update commands it shows with nothing open
        // too, holding only them: the explorer's would act on nothing.
        .when(open || automatic.is_some(), |part| {
            part.child(more(state, open, automatic))
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

/// More (…): with something `open`, the explorer's commands, each showing
/// its key. With `automatic`, Check for Updates… and Check Automatically,
/// checked as `automatic` says, after a separator when the explorer's
/// commands come first.
fn more(state: &AppState, open: bool, automatic: Option<bool>) -> impl IntoElement {
    let has_tree = state.explorer().is_some();
    let partial = state
        .explorer()
        .is_some_and(|e| e.tree().folders().any(|top| e.tree().is_partial(top)));
    let expand = if partial {
        "Expand Listed Folders"
    } else {
        "Expand All"
    };
    let select = if partial {
        "Select Listed Files"
    } else {
        "Select All"
    };
    let checked = state.selected_count() > 0;
    let shows_ignored = state.shows_ignored();
    Button::new("more")
        .ghost()
        .small()
        .icon(Icons::Ellipsis)
        .tooltip("More")
        .accessibility_label("More")
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            menu.when(open, |menu| {
                menu.menu_with_disabled(expand, Box::new(ExpandAll), !has_tree)
                    .menu_with_disabled("Collapse All", Box::new(CollapseAll), !has_tree)
                    .separator()
                    .menu_with_check("Show Ignored", shows_ignored, Box::new(ToggleShowIgnored))
                    .menu("Refresh", Box::new(Refresh))
                    .separator()
                    .menu_with_disabled(select, Box::new(SelectAllFiles), !has_tree)
                    .menu_with_disabled("Deselect All", Box::new(DeselectAll), !checked)
            })
            .when_some(automatic, |menu, on| {
                menu.when(open, PopupMenu::separator)
                    .menu("Check for Updates…", Box::new(CheckForUpdates))
                    .menu_with_check(
                        "Check Automatically",
                        on,
                        Box::new(ToggleCheckAutomatically),
                    )
            })
        })
}

/// The part over the stage: Back and Forward, its title, the update
/// `offer` if any, and Present. Beside the explorer its left border is the
/// rule that continues the split's divider.
fn stage_part(
    app_state: &gpui_kit::Entity<AppState>,
    beside_explorer: bool,
    updater: Option<&Updater>,
    cx: &App,
) -> impl IntoElement {
    let state = app_state.read(cx);
    h_flex()
        .id("bar-stage")
        .test_support()
        // The rest of the row, whatever the title's length. The Kit's row
        // holding both parts does not shrink and sets no minimum width, so
        // it is at least as wide as its content's narrowest layout, which
        // counts the whole title unless this part's own width is zero; the
        // title then truncates and Present stays at the right end.
        .flex_1()
        .w_0()
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
                    .child(breadcrumbs(app_state, title, cx)),
            )
        })
        // Empty, so the window moves from here.
        .child(div().flex_1())
        .when_some(
            (!beside_explorer)
                .then(|| state.folder_activity())
                .flatten(),
            |part, label| part.child(activity_label("folder-activity", label, cx)),
        )
        .when_some(updater.and_then(Updater::activity), |part, label| {
            part.child(activity_label("update-activity", label, cx))
        })
        .when_some(updater.and_then(Updater::offer), |part, offer| {
            part.child(update_button(offer))
        })
        .child(
            // Disabled while the stage is blank, which is when it has no
            // title.
            command("present", Icons::Play, "Present", StartPresenting).disabled(
                state.stage_title().is_none() || state.stage_view() == crate::StageView::Folder,
            ),
        )
}

pub(crate) fn activity_label(
    id: &'static str,
    label: SharedString,
    cx: &App,
) -> impl IntoElement + use<> {
    div()
        .id(id)
        .test_support()
        .role(Role::Status)
        .aria_label(label.clone())
        .min_w_0()
        .truncate()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(label)
}

/// The update ready to install: the Kit's primary button, small, named
/// for how it installs. Its tooltip gives the version, and the Kit gives
/// the tooltip to assistive technology as the button's description.
fn update_button(offer: &Offer) -> Button {
    Button::new("update")
        .primary()
        .small()
        .label(install_label(offer.how))
        .tooltip(format!("Version {}", offer.version))
        .on_click(|_, window, cx| window.dispatch_action(Box::new(InstallUpdate), cx))
}

/// What the update button says for an update that installs `how`.
fn install_label(how: How) -> &'static str {
    match how {
        How::Restart => "Restart to Update",
        How::Install => "Install Update",
    }
}

/// The path inside the open root, alongside Back and Forward. The current
/// entry is the last crumb and each ancestor opens its directory.
fn breadcrumbs(
    state: &gpui_kit::Entity<AppState>,
    title: SharedString,
    cx: &App,
) -> gpui_kit::AnyElement {
    use gpui_kit::component::breadcrumb::{Breadcrumb, BreadcrumbItem};
    let model = state.read(cx);
    let Some((tree, id)) = model
        .explorer()
        .zip(model.current())
        .filter(|_| !model.shows_gallery())
    else {
        return title.into_any_element();
    };
    let generation = model.staged().0;
    let mut ancestors: Vec<_> = std::iter::successors(Some(id), |id| tree.tree().parent(*id))
        .take_while(|id| *id != explorer::NodeId::ROOT)
        .collect();
    if ancestors.len() <= 1 {
        return title.into_any_element();
    }
    ancestors.reverse();
    let items = ancestors.into_iter().map(|node| {
        let item = BreadcrumbItem::new(tree.tree().name(node).to_owned())
            .min_w_0()
            .truncate();
        if node == id {
            return item;
        }
        let state = state.clone();
        item.on_click(move |_, _, cx| {
            state.update(cx, |state, cx| state.navigate(generation, node, cx));
        })
    });
    Breadcrumb::new()
        .min_w_0()
        .overflow_hidden()
        .children(items)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::install_label;
    use crate::update::How;

    #[test]
    fn the_update_button_says_how_the_update_installs() {
        assert_eq!(install_label(How::Restart), "Restart to Update");
        assert_eq!(install_label(How::Install), "Install Update");
    }
}
