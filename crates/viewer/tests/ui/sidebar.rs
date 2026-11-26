//! Hiding the explorer: the stage takes the window, the explorer comes
//! back at its width, focus never stays in what hid, and the choice is
//! saved.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, OwnedMenuItem, TestAppContext, px, size};
use viewer::keys::ToggleSidebar;
use viewer::{Launch, SAVE_DELAY, StateStore};

use crate::{PROJECT, click, dispatch, drag_divider, is_focused, open_app, press, tree};

/// The key that hides and shows the sidebar on this platform.
const SIDEBAR_KEY: &str = if cfg!(target_os = "macos") {
    "ctrl-cmd-s"
} else {
    "ctrl-b"
};

#[gpui_kit::test]
fn hiding_gives_the_stage_the_window_and_showing_brings_the_width_back(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    drag_divider(cx, handle, 400.);

    click(cx, handle, "sidebar");
    assert!(cx.update(|cx| view.read(cx).explorer_hidden()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("explorer-pane").is_none());
        let stage = window.find("stage-pane").bounds();
        assert_eq!(stage.left(), px(0.));
        assert_eq!(stage.size.width, px(1000.));
        assert_eq!(window.find("sidebar").label(), Some("Show Sidebar"));
        // Add Folder and More hide with the explorer.
        assert!(window.try_find("add-folder").is_none());
        assert!(window.try_find("more").is_none());
    })
    .unwrap();

    press(cx, handle, SIDEBAR_KEY);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("explorer-pane").bounds().right(), px(400.));
        assert_eq!(window.find("sidebar").label(), Some("Hide Sidebar"));
    })
    .unwrap();
}

/// The View menu's first item: Hide Sidebar or Show Sidebar.
fn sidebar_item(cx: &mut TestAppContext) -> String {
    let menus = cx.update(|cx| cx.get_menus().unwrap_or_default());
    let view = menus
        .iter()
        .find(|menu| menu.name.as_ref() == "View")
        .expect("a View menu");
    view.items
        .iter()
        .find_map(|item| match item {
            OwnedMenuItem::Action { name, .. } => Some(name.clone()),
            _ => None,
        })
        .expect("the View menu starts with an action")
}

#[gpui_kit::test]
fn the_view_menu_names_what_its_item_will_do(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    assert_eq!(sidebar_item(cx), "Hide Sidebar");
    dispatch(cx, handle, ToggleSidebar);
    assert_eq!(sidebar_item(cx), "Show Sidebar");
    dispatch(cx, handle, ToggleSidebar);
    assert_eq!(sidebar_item(cx), "Hide Sidebar");
}

#[gpui_kit::test]
fn hiding_moves_focus_to_the_stage_and_command_f_shows_it_again(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    // The tree holds focus at launch.
    dispatch(cx, handle, ToggleSidebar);
    assert!(is_focused(cx, handle, "stage"));
    // Tab stays on the stage.
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));

    press(
        cx,
        handle,
        if cfg!(target_os = "macos") {
            "cmd-f"
        } else {
            "ctrl-f"
        },
    );
    assert!(!cx.update(|cx| view.read(cx).explorer_hidden()));
    assert!(is_focused(cx, handle, "filter"));
}

#[gpui_kit::test]
fn esc_on_the_stage_leaves_focus_there_while_the_explorer_is_hidden(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    dispatch(cx, handle, ToggleSidebar);
    assert!(is_focused(cx, handle, "stage"));
    // Esc in the file view, with fewer than two files checked, would focus
    // the explorer.
    press(cx, handle, "escape");
    assert!(is_focused(cx, handle, "stage"));
}

#[gpui_kit::test]
fn a_window_resize_keeps_the_explorers_width(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    let explorer = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("explorer-pane").bounds().size.width
        })
        .unwrap()
    };
    let before = explorer(cx);
    cx.simulate_window_resize(handle, size(px(1400.), px(700.)));
    cx.run_until_parked();
    assert_eq!(explorer(cx), before);
}

#[gpui_kit::test]
fn the_explorer_keeps_its_width_through_a_resize_while_hidden(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    drag_divider(cx, handle, 400.);
    dispatch(cx, handle, ToggleSidebar);
    cx.simulate_window_resize(handle, size(px(1400.), px(700.)));
    cx.run_until_parked();
    dispatch(cx, handle, ToggleSidebar);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let explorer = window.find("explorer-pane").bounds();
        assert_eq!(explorer.size.width, px(400.));
        assert_eq!(window.find("stage-pane").bounds().right(), px(1400.));
        // The bar's explorer part follows it.
        assert_eq!(window.find("bar-explorer").bounds().right(), px(400.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_launch_with_nothing_open_shows_the_explorer(cx: &mut TestAppContext) {
    // Saved hidden, but nothing to open: the Add Folder button is the
    // empty state, and it lives in the explorer.
    let (handle, view) = crate::open_app_with_session(
        cx,
        Launch::Empty,
        viewer::Session {
            explorer_hidden: true,
            ..Default::default()
        },
    );
    assert!(!cx.update(|cx| view.read(cx).explorer_hidden()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("add-folder").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_launch_saved_hidden_opens_hidden_with_the_stage_focused(cx: &mut TestAppContext) {
    let (handle, view) = crate::open_app_with_session(
        cx,
        tree(PROJECT),
        viewer::Session {
            explorer_hidden: true,
            ..Default::default()
        },
    );
    assert!(cx.update(|cx| view.read(cx).explorer_hidden()));
    assert!(is_focused(cx, handle, "stage"));
}

#[gpui_kit::test]
fn hiding_the_explorer_is_saved(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(dir.path().join("state.json"));
    let (handle, _) = crate::open_app_with_session(
        cx,
        tree(PROJECT),
        viewer::Session {
            store: Some(store.clone()),
            ..Default::default()
        },
    );
    dispatch(cx, handle, ToggleSidebar);
    cx.executor().advance_clock(SAVE_DELAY);
    cx.run_until_parked();
    assert!(store.load().expect("a state file").explorer_hidden);
}

#[gpui_kit::test]
fn adding_a_folder_shows_the_explorer(cx: &mut TestAppContext) {
    let dir = crate::folder(&[("a.txt", "a")]);
    let (handle, view) = open_app(cx, tree(PROJECT));
    dispatch(cx, handle, ToggleSidebar);
    view.update(cx, |view, cx| {
        view.open_paths(vec![dir.path().to_owned()], cx);
    });
    cx.run_until_parked();
    assert!(!cx.update(|cx| view.read(cx).explorer_hidden()));
}

#[gpui_kit::test]
fn opening_a_file_leaves_the_explorer_hidden(cx: &mut TestAppContext) {
    let dir = crate::folder(&[("a.txt", "a")]);
    let (handle, view) = open_app(cx, tree(PROJECT));
    dispatch(cx, handle, ToggleSidebar);
    view.update(cx, |view, cx| {
        view.open_paths(vec![dir.path().join("a.txt")], cx);
    });
    cx.run_until_parked();
    assert!(cx.update(|cx| view.read(cx).explorer_hidden()));
}
