//! Back and forward through what the stage showed: steps added, replaced
//! by the arrows, passed over when they cannot be shown, from the bar, the
//! keys and the mouse.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, InputEvent as _, Modifiers, MouseButton, MouseDownEvent, NavigationDirection,
    TestAppContext, point, px,
};
use viewer::keys::{DeselectAll, GoBack, GoForward, SelectAllFiles};
use viewer::{AppState, Launch, SavedView, Session};

use crate::stage::code_view;
use crate::{
    PROJECT, click, current, dispatch, folder, node, open_app, open_app_with_session, press, read,
    row, tree,
};

/// Back on this platform.
const BACK: &str = if cfg!(target_os = "macos") {
    "cmd-["
} else {
    "alt-left"
};

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

    // The mouse's back button, anywhere over the window.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let event = MouseDownEvent {
            button: MouseButton::Navigate(NavigationDirection::Back),
            position: point(px(500.), px(300.)),
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        };
        window.dispatch_event(event.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
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
