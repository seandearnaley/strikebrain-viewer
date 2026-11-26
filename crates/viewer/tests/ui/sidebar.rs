//! Hiding the explorer: the stage takes the window, the explorer comes
//! back at its width, focus never stays in what hid, and the choice is
//! saved.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, OwnedMenuItem, Pixels, TestAppContext, px, size,
};
use viewer::keys::{StartPresenting, ToggleSidebar};
use viewer::{AppState, Launch, SAVE_DELAY, ScanStatus, Session, StateStore};

use crate::{
    PROJECT, click, dispatch, drag_divider, is_focused, node, node_at, open_app,
    open_app_with_session, press, read, row, tree,
};

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

/// The explorer's, the stage's and the bar's explorer part's bounds after a
/// fresh frame.
fn panes(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
) -> (Bounds<Pixels>, Bounds<Pixels>, Bounds<Pixels>) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        (
            window.find("explorer-pane").bounds(),
            window.find("stage-pane").bounds(),
            window.find("bar-explorer").bounds(),
        )
    })
    .expect("read the panes")
}

#[gpui_kit::test]
fn a_narrower_window_narrows_the_explorer_down_to_the_stages_minimum(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    drag_divider(cx, handle, 700.);
    cx.simulate_window_resize(handle, size(px(600.), px(700.)));
    cx.run_until_parked();
    let (explorer, stage, bar) = panes(cx, handle);
    // The stage keeps its 220 px inside the window; the explorer gives way.
    assert_eq!(explorer.size.width, px(380.));
    assert_eq!(stage.left(), px(380.));
    assert_eq!(stage.right(), px(600.));
    assert_eq!(bar.right(), px(380.));
}

#[gpui_kit::test]
fn the_explorer_shown_in_a_narrower_window_leaves_the_stage_its_minimum(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    drag_divider(cx, handle, 700.);
    dispatch(cx, handle, ToggleSidebar);
    cx.simulate_window_resize(handle, size(px(600.), px(700.)));
    cx.run_until_parked();
    dispatch(cx, handle, ToggleSidebar);
    let (explorer, stage, bar) = panes(cx, handle);
    assert_eq!(explorer.size.width, px(380.));
    assert_eq!(stage.left(), px(380.));
    assert_eq!(stage.right(), px(600.));
    assert_eq!(bar.right(), px(380.));
}

#[gpui_kit::test]
fn a_window_narrowed_and_widened_while_hidden_keeps_the_explorers_width(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    drag_divider(cx, handle, 700.);
    dispatch(cx, handle, ToggleSidebar);
    cx.simulate_window_resize(handle, size(px(600.), px(700.)));
    cx.run_until_parked();
    cx.simulate_window_resize(handle, size(px(1000.), px(700.)));
    cx.run_until_parked();
    dispatch(cx, handle, ToggleSidebar);
    let (explorer, stage, _) = panes(cx, handle);
    assert_eq!(explorer.size.width, px(700.));
    assert_eq!(stage.right(), px(1000.));
}

#[gpui_kit::test]
fn a_window_launched_hidden_and_narrowed_shows_the_explorer_within_it(cx: &mut TestAppContext) {
    // The split is first laid out when the explorer shows.
    let (handle, _) = open_app_with_session(
        cx,
        tree(PROJECT),
        Session {
            explorer_hidden: true,
            ..Default::default()
        },
    );
    cx.simulate_window_resize(handle, size(px(440.), px(700.)));
    cx.run_until_parked();
    dispatch(cx, handle, ToggleSidebar);
    let (explorer, stage, _) = panes(cx, handle);
    assert_eq!(explorer.size.width, px(220.));
    assert_eq!(stage.right(), px(440.));
}

#[gpui_kit::test]
fn leaving_a_presentation_in_a_narrower_window_leaves_the_stage_its_minimum(
    cx: &mut TestAppContext,
) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let readme = node(cx, &view, "README.md");
    click(cx, handle, row(readme));
    drag_divider(cx, handle, 700.);
    dispatch(cx, handle, StartPresenting);
    assert!(read(cx, &view, AppState::is_presenting));
    cx.simulate_window_resize(handle, size(px(600.), px(700.)));
    cx.run_until_parked();
    press(cx, handle, "escape");
    let (explorer, stage, _) = panes(cx, handle);
    assert_eq!(explorer.size.width, px(380.));
    assert_eq!(stage.right(), px(600.));
}

#[gpui_kit::test]
fn a_launch_with_nothing_open_shows_the_explorer(cx: &mut TestAppContext) {
    // Saved hidden, but nothing to open: the Add Folder button is the
    // empty state, and it lives in the explorer.
    let (handle, view) = open_app_with_session(
        cx,
        Launch::Empty,
        Session {
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
    let (handle, view) = open_app_with_session(
        cx,
        tree(PROJECT),
        Session {
            explorer_hidden: true,
            ..Default::default()
        },
    );
    assert!(cx.update(|cx| view.read(cx).explorer_hidden()));
    assert!(is_focused(cx, handle, "stage"));
}

/// Opens `folders` with the explorer saved hidden, and lets every scan land.
fn open_hidden(
    cx: &mut TestAppContext,
    folders: Vec<std::path::PathBuf>,
) -> (AnyWindowHandle, gpui_kit::Entity<viewer::AppView>) {
    let opened = open_app_with_session(
        cx,
        Launch::Folders(folders),
        Session {
            explorer_hidden: true,
            ..Default::default()
        },
    );
    assert_eq!(read(cx, &opened.1, AppState::status), ScanStatus::Idle);
    opened
}

#[gpui_kit::test]
fn a_launch_of_a_folder_saved_hidden_stays_hidden_once_it_lands(cx: &mut TestAppContext) {
    let dir = crate::folder(&[("a.txt", "a")]);
    let (handle, view) = open_hidden(cx, vec![dir.path().to_owned()]);
    node_at(cx, &view, &dir.path().join("a.txt"));
    assert!(cx.update(|cx| view.read(cx).explorer_hidden()));
    assert!(is_focused(cx, handle, "stage"));
}

#[gpui_kit::test]
fn a_launch_of_a_folder_and_one_inside_it_stays_hidden_once_both_land(cx: &mut TestAppContext) {
    // The inner folder waits for the outer one's scan, then opens: after
    // the launch has settled, but asked for by it.
    let dir = crate::folder(&[("a.txt", "a"), ("inner/b.txt", "b")]);
    let inner = dir.path().join("inner");
    let (handle, view) = open_hidden(cx, vec![dir.path().to_owned(), inner.clone()]);
    node_at(cx, &view, &inner.join("b.txt"));
    assert!(cx.update(|cx| view.read(cx).explorer_hidden()));
    assert!(is_focused(cx, handle, "stage"));
}

#[gpui_kit::test]
fn the_explorer_stays_hidden_through_a_presentation(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let readme = node(cx, &view, "README.md");
    click(cx, handle, row(readme));
    dispatch(cx, handle, ToggleSidebar);
    // Focus moved to the stage, where Space presents.
    press(cx, handle, "space");
    assert!(read(cx, &view, AppState::is_presenting));
    press(cx, handle, "escape");
    assert!(!read(cx, &view, AppState::is_presenting));
    assert!(cx.update(|cx| view.read(cx).explorer_hidden()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("explorer-pane").is_none());
        assert_eq!(window.find("stage-pane").bounds().size.width, px(1000.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn hiding_the_explorer_is_saved(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(dir.path().join("state.json"));
    let (handle, _) = open_app_with_session(
        cx,
        tree(PROJECT),
        Session {
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
fn asking_again_for_a_folder_still_scanning_shows_the_explorer(cx: &mut TestAppContext) {
    let dir = crate::folder(&[("a.txt", "a")]);
    let (handle, view) = open_app(cx, tree(PROJECT));
    view.update(cx, |view, cx| {
        view.open_paths(vec![dir.path().to_owned()], cx);
    });
    // Hidden before the folder's first scan lands: nothing here lets it.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(ToggleSidebar), cx);
    })
    .unwrap();
    assert!(cx.update(|cx| view.read(cx).explorer_hidden()));
    view.update(cx, |view, cx| {
        view.open_paths(vec![dir.path().to_owned()], cx);
    });
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
