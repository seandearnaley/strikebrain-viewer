//! Back and forward through what the stage showed: steps added, replaced
//! by the arrows, passed over when they cannot be shown, from the bar, the
//! keys and the mouse.

use explorer::{SavedFolder, SavedSelection};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, InputEvent as _, Modifiers, MouseButton, MouseDownEvent,
    NavigationDirection, TestAppContext, point, px,
};
use viewer::keys::{DeselectAll, GoBack, GoForward, Refresh, SelectAllFiles};
use viewer::{AppState, Launch, SavedView, Session, StageView};

use crate::stage::code_view;
use crate::{
    PROJECT, click, current, dispatch, folder, node, node_at, open_app, open_app_with_session,
    press, read, row, state, tree,
};

/// Back on this platform.
const BACK: &str = if cfg!(target_os = "macos") {
    "cmd-["
} else {
    "alt-left"
};

/// Forward on this platform.
const FORWARD: &str = if cfg!(target_os = "macos") {
    "cmd-]"
} else {
    "alt-right"
};

/// Presses the mouse's back or forward button over the stage.
fn mouse(cx: &mut TestAppContext, handle: AnyWindowHandle, direction: NavigationDirection) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let event = MouseDownEvent {
            button: MouseButton::Navigate(direction),
            position: point(px(500.), px(300.)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        };
        window.dispatch_event(event.to_platform_input(), cx);
    })
    .expect("press the mouse's button");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn back_and_forward_walk_what_the_stage_showed(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let (two, ten, readme) = (
        node(cx, &view, "file2.txt"),
        node(cx, &view, "file10.txt"),
        node(cx, &view, "README.md"),
    );
    assert!(!read(cx, &view, AppState::can_go_back));

    click(cx, handle, row(two));
    click(cx, handle, row(ten));
    dispatch(cx, handle, GoBack);
    assert_eq!(current(cx, &view), Some(two));
    assert!(read(cx, &view, AppState::can_go_forward));
    dispatch(cx, handle, GoForward);
    assert_eq!(current(cx, &view), Some(ten));

    // A new step drops the steps ahead.
    dispatch(cx, handle, GoBack);
    click(cx, handle, row(readme));
    assert!(!read(cx, &view, AppState::can_go_forward));
    dispatch(cx, handle, GoBack);
    assert_eq!(current(cx, &view), Some(two));
}

#[gpui_kit::test]
fn the_arrows_replace_the_step_and_the_gallery_is_a_step(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let (two, ten) = (node(cx, &view, "file2.txt"), node(cx, &view, "file10.txt"));

    dispatch(cx, handle, SelectAllFiles);
    assert!(read(cx, &view, AppState::shows_gallery));
    click(cx, handle, row(two));
    // To the stage, then → to the next checked file: the same step.
    press(cx, handle, "tab");
    press(cx, handle, "right");
    assert_eq!(current(cx, &view), Some(ten));

    dispatch(cx, handle, GoBack);
    assert!(read(cx, &view, AppState::shows_gallery));
    dispatch(cx, handle, GoForward);
    assert_eq!(current(cx, &view), Some(ten));
    assert!(!read(cx, &view, AppState::shows_gallery));
}

#[gpui_kit::test]
fn back_passes_over_what_cannot_be_shown(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let two = node(cx, &view, "file2.txt");

    dispatch(cx, handle, SelectAllFiles);
    click(cx, handle, row(two));
    // Nothing checked: the stage is blank and the gallery cannot show.
    dispatch(cx, handle, DeselectAll);
    assert_eq!(current(cx, &view), None);
    assert!(read(cx, &view, AppState::can_go_back));

    // From a blank stage, Back shows the step last shown.
    dispatch(cx, handle, GoBack);
    assert_eq!(current(cx, &view), Some(two));
    // Before it, only the gallery, which cannot show.
    assert!(!read(cx, &view, AppState::can_go_back));
    dispatch(cx, handle, GoBack);
    assert_eq!(current(cx, &view), Some(two));
}

#[gpui_kit::test]
fn the_keys_the_mouse_and_the_bar_go_back(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let (two, ten, readme) = (
        node(cx, &view, "file2.txt"),
        node(cx, &view, "file10.txt"),
        node(cx, &view, "README.md"),
    );
    click(cx, handle, row(two));
    click(cx, handle, row(ten));
    click(cx, handle, row(readme));

    // From the filter, whose input passes the key on.
    press(cx, handle, "secondary-f");
    press(cx, handle, BACK);
    assert_eq!(current(cx, &view), Some(ten));

    // The mouse's back and forward buttons, anywhere over the window.
    mouse(cx, handle, NavigationDirection::Back);
    assert_eq!(current(cx, &view), Some(two));
    mouse(cx, handle, NavigationDirection::Forward);
    assert_eq!(current(cx, &view), Some(ten));
    mouse(cx, handle, NavigationDirection::Back);
    assert_eq!(current(cx, &view), Some(two));

    click(cx, handle, "forward");
    assert_eq!(current(cx, &view), Some(ten));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("back").label(), Some("Back"));
        assert_eq!(window.find("forward").label(), Some("Forward"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_bars_arrows_are_disabled_with_nowhere_to_go(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let disabled = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            (
                window.find("back").disabled() == Some(true),
                window.find("forward").disabled() == Some(true),
            )
        })
        .unwrap()
    };
    let (two, ten) = (node(cx, &view, "file2.txt"), node(cx, &view, "file10.txt"));
    assert_eq!(disabled(cx), (true, true));
    click(cx, handle, row(two));
    click(cx, handle, row(ten));
    assert_eq!(disabled(cx), (false, true));
    dispatch(cx, handle, GoBack);
    assert_eq!(disabled(cx), (true, false));
}

#[gpui_kit::test]
fn the_code_view_passes_back_on(cx: &mut TestAppContext) {
    let dir = folder(&[("a.rs", "fn a() {}\n"), ("b.rs", "fn b() {}\n")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let (a, b) = (node(cx, &view, "a.rs"), node(cx, &view, "b.rs"));
    click(cx, handle, row(a));
    click(cx, handle, row(b));

    // A click in the code view gives its read-only editor focus.
    click(cx, handle, "stage-code");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(code_view(window).focused(), Some(true));
    })
    .unwrap();
    press(cx, handle, BACK);
    assert_eq!(current(cx, &view), Some(a));
}

#[gpui_kit::test]
fn a_launch_shows_its_saved_file_as_the_first_step(cx: &mut TestAppContext) {
    let session = Session {
        view: Some(SavedView {
            current: Some("README.md".into()),
            ..SavedView::default()
        }),
        ..Session::default()
    };
    let (handle, view) = open_app_with_session(cx, tree(PROJECT), session);
    let (two, readme) = (node(cx, &view, "file2.txt"), node(cx, &view, "README.md"));
    assert_eq!(current(cx, &view), Some(readme));

    click(cx, handle, row(two));
    dispatch(cx, handle, GoBack);
    assert_eq!(current(cx, &view), Some(readme));
}

#[gpui_kit::test]
fn back_passes_over_a_file_no_longer_in_the_tree(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("c.txt", "c")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    for name in ["c.txt", "a.txt", "b.txt"] {
        let id = node(cx, &view, name);
        click(cx, handle, row(id));
    }

    std::fs::remove_file(dir.path().join("a.txt")).unwrap();
    dispatch(cx, handle, Refresh);
    let (b, c) = (node(cx, &view, "b.txt"), node(cx, &view, "c.txt"));
    assert_eq!(current(cx, &view), Some(b));

    dispatch(cx, handle, GoBack);
    assert_eq!(current(cx, &view), Some(c));
    assert!(!read(cx, &view, AppState::can_go_back));
    dispatch(cx, handle, GoForward);
    assert_eq!(current(cx, &view), Some(b));
    assert!(!read(cx, &view, AppState::can_go_forward));
}

#[gpui_kit::test]
fn back_and_forward_do_nothing_while_presenting(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let (two, ten, readme) = (
        node(cx, &view, "file2.txt"),
        node(cx, &view, "file10.txt"),
        node(cx, &view, "README.md"),
    );
    click(cx, handle, row(two));
    click(cx, handle, row(ten));
    click(cx, handle, row(readme));
    dispatch(cx, handle, GoBack);
    // Steps lie both ways.
    assert_eq!(current(cx, &view), Some(ten));

    click(cx, handle, "present");
    assert!(read(cx, &view, AppState::is_presenting));
    press(cx, handle, BACK);
    press(cx, handle, FORWARD);
    mouse(cx, handle, NavigationDirection::Back);
    mouse(cx, handle, NavigationDirection::Forward);
    // The menu bar's items.
    dispatch(cx, handle, GoBack);
    dispatch(cx, handle, GoForward);
    assert_eq!(current(cx, &view), Some(ten));
    assert!(read(cx, &view, AppState::is_presenting));
}

#[gpui_kit::test]
fn back_to_the_gallery_from_the_code_view_keeps_the_gallerys_keys(cx: &mut TestAppContext) {
    let dir = folder(&[("a.rs", "a"), ("b.rs", "b"), ("c.rs", "c")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let (a, b) = (node(cx, &view, "a.rs"), node(cx, &view, "b.rs"));
    dispatch(cx, handle, SelectAllFiles);
    // To the stage, then the first tile opens in the code view.
    press(cx, handle, "tab");
    press(cx, handle, "enter");
    assert_eq!(current(cx, &view), Some(a));
    click(cx, handle, "stage-code");

    press(cx, handle, BACK);
    assert!(read(cx, &view, AppState::shows_gallery));
    assert_eq!(read(cx, &view, AppState::tile), Some(a));
    // The gallery's keys: → moves its focus.
    press(cx, handle, "right");
    assert_eq!(read(cx, &view, AppState::tile), Some(b));
}

#[gpui_kit::test]
fn files_opened_together_at_launch_are_one_step(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("c.txt", "c")]);
    let (b, c) = (dir.path().join("b.txt"), dir.path().join("c.txt"));
    // Their folder with them, so a.txt is in the tree too.
    let launch = Launch::Folders(vec![dir.path().to_owned(), b, c]);
    let (handle, view) = open_app(cx, launch);
    assert!(read(cx, &view, AppState::shows_gallery));
    assert!(!read(cx, &view, AppState::can_go_back));

    // The gallery is the only step before a file shown from the tree.
    let a = node(cx, &view, "a.txt");
    click(cx, handle, row(a));
    dispatch(cx, handle, GoBack);
    assert!(read(cx, &view, AppState::shows_gallery));
    assert!(!read(cx, &view, AppState::can_go_back));
}

#[gpui_kit::test]
fn back_from_files_opened_together_shows_what_showed_before(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("c.txt", "c")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let a = node(cx, &view, "a.txt");
    click(cx, handle, row(a));

    let (b, c) = (dir.path().join("b.txt"), dir.path().join("c.txt"));
    view.update(cx, |view, cx| view.open_paths(vec![b, c], cx));
    cx.run_until_parked();
    assert!(read(cx, &view, AppState::shows_gallery));
    dispatch(cx, handle, GoBack);
    assert_eq!(current(cx, &view), Some(a));
    assert!(!read(cx, &view, AppState::shows_gallery));
}

#[gpui_kit::test]
fn a_file_opened_at_launch_is_the_first_step_not_the_saved_one(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b")]);
    let saved = explorer::canonical(&dir.path().join("a.txt")).unwrap();
    let session = Session {
        view: Some(SavedView {
            current: Some(saved),
            ..SavedView::default()
        }),
        ..Session::default()
    };
    let b = dir.path().join("b.txt");
    let (_, view) = open_app_with_session(cx, Launch::Folders(vec![b.clone()]), session);
    assert_eq!(current(cx, &view), Some(node_at(cx, &view, &b)));
    assert!(!read(cx, &view, AppState::can_go_back));
}

#[gpui_kit::test]
fn a_folder_bringing_its_checks_to_a_blank_stage_is_a_step(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b")]);
    let (handle, view) = open_app(cx, Launch::Empty);
    // A folder added with a saved view waiting for it: open, a.txt checked.
    let saved = SavedFolder {
        path: explorer::canonical(dir.path()).unwrap(),
        open: vec![String::new()],
        selection: SavedSelection {
            files: vec!["a.txt".into()],
            ..SavedSelection::default()
        },
    };
    state(cx, &view).update(cx, |state, cx| {
        state.open_paths(vec![dir.path().to_owned()], vec![saved], cx);
    });
    cx.run_until_parked();
    let (a, b) = (node(cx, &view, "a.txt"), node(cx, &view, "b.txt"));
    assert_eq!(current(cx, &view), Some(a));

    click(cx, handle, row(b));
    dispatch(cx, handle, GoBack);
    assert_eq!(current(cx, &view), Some(a));
}

/// A launch of `dir` with a saved view of the gallery: a.txt and c.txt
/// checked, a.txt on the stage.
fn saved_gallery(dir: &std::path::Path) -> Session {
    let folder = explorer::canonical(dir).expect("the folder exists");
    Session {
        view: Some(SavedView {
            folders: vec![SavedFolder {
                path: folder.clone(),
                open: vec![String::new()],
                selection: SavedSelection {
                    files: vec!["a.txt".into(), "c.txt".into()],
                    ..SavedSelection::default()
                },
            }],
            current: Some(folder.join("a.txt")),
            view: StageView::Gallery,
            ..SavedView::default()
        }),
        ..Session::default()
    }
}

#[gpui_kit::test]
fn a_saved_gallery_is_the_first_step(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("c.txt", "c")]);
    let launch = Launch::Folders(vec![dir.path().to_owned()]);
    let (handle, view) = open_app_with_session(cx, launch, saved_gallery(dir.path()));
    assert!(read(cx, &view, AppState::shows_gallery));
    assert!(!read(cx, &view, AppState::can_go_back));

    let b = node(cx, &view, "b.txt");
    click(cx, handle, row(b));
    dispatch(cx, handle, GoBack);
    assert!(read(cx, &view, AppState::shows_gallery));
    assert!(!read(cx, &view, AppState::can_go_back));
}

#[gpui_kit::test]
fn a_saved_gallery_a_file_opened_at_launch_replaces_is_no_step(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("c.txt", "c")]);
    let b = dir.path().join("b.txt");
    let launch = Launch::Folders(vec![dir.path().to_owned(), b.clone()]);
    let (_, view) = open_app_with_session(cx, launch, saved_gallery(dir.path()));
    assert_eq!(current(cx, &view), Some(node_at(cx, &view, &b)));
    assert!(!read(cx, &view, AppState::shows_gallery));
    assert!(!read(cx, &view, AppState::can_go_back));
}

#[gpui_kit::test]
fn folder_click_opens_contents_and_children_join_navigation_history(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let docs = node(cx, &view, "docs");
    let child = node(cx, &view, "docs/a.md");
    click(cx, handle, row(docs));
    assert_eq!(current(cx, &view), Some(docs));
    crate::double_click(cx, handle, ("folder-entry", child.index()));
    assert_eq!(current(cx, &view), Some(child));
    dispatch(cx, handle, GoBack);
    assert_eq!(current(cx, &view), Some(docs));
    assert_eq!(read(cx, &view, AppState::selected_count), 0);
}

#[gpui_kit::test]
fn tab_then_arrow_and_enter_open_a_folder_child(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let docs = node(cx, &view, "docs");
    let first = node(cx, &view, "docs/a.md");
    click(cx, handle, row(docs));
    press(cx, handle, "tab");
    press(cx, handle, "down");
    press(cx, handle, "enter");
    assert_eq!(current(cx, &view), Some(first));
    assert!(!read(cx, &view, AppState::is_presenting));
}

#[gpui_kit::test]
fn clicking_an_ancestor_breadcrumb_opens_that_folder(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let docs = node(cx, &view, "docs");
    let file = node(cx, &view, "docs/a.md");
    click(cx, handle, row(docs));
    crate::double_click(cx, handle, ("folder-entry", file.index()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let title = window.find("stage-title").bounds();
        crate::click_without_frame(window, point(title.left() + px(9.), title.center().y), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(current(cx, &view), Some(docs));
    dispatch(cx, handle, GoBack);
    assert_eq!(current(cx, &view), Some(file));
}
