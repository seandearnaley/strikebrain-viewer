//! Input and background work in every order the test scheduler can be
//! made to take: it runs nothing until the test parks, so each test lines
//! up its events and jobs first and only then lets them run.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Pixels, Point, Subscription, TestAppContext,
};
use viewer::{AppState, AppView, Launch};

use crate::{click_without_frame, current, folder, node, open_app, read, row, rows_now, state};

/// Arms a click at `position` that fires the moment another tree lands,
/// inside the update that swaps it and before any frame is drawn: the click
/// reaches the listeners the last frame drew for the old tree, as a click
/// does that the platform delivers between a swap and the next frame.
fn click_when_the_tree_swaps(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<AppView>,
    position: Point<Pixels>,
) -> Subscription {
    let tree = |state: &AppState| state.explorer().map(|e| Arc::as_ptr(e.tree()));
    let before = read(cx, view, tree);
    let armed = Rc::new(Cell::new(true));
    let state = state(cx, view);
    cx.update(|cx| {
        cx.observe(&state, move |state, cx| {
            if armed.get() && tree(state.read(cx)) != before {
                armed.set(false);
                cx.update_window(handle, |_, window, cx| {
                    click_without_frame(window, position, cx);
                })
                .expect("click in the test window");
            }
        })
    })
}

/// Where the last frame drew the row of `path`, and its checkbox.
fn drawn_at(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<AppView>,
    path: &str,
) -> (Point<Pixels>, Point<Pixels>) {
    let node = node(cx, view, path);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        (
            window.find(row(node)).bounds().center(),
            window.within(row(node)).find("check").bounds().center(),
        )
    })
    .expect("read the frame")
}

/// A click drawn before a refresh swapped the tree, delivered after the swap
/// and before the next frame: it lands on nothing, neither past the new
/// tree's end nor on the file that now has the old row's id.
#[gpui_kit::test]
fn a_click_drawn_before_a_refresh_does_nothing(cx: &mut TestAppContext) {
    let dir = folder(&[
        ("a.txt", "a"),
        ("b.txt", "b"),
        ("c.txt", "c"),
        ("d.txt", "d"),
    ]);
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));

    // d.txt's id is past the end of the next tree, which holds a and b.
    let (_, d_check) = drawn_at(cx, handle, &view, "d.txt");
    for gone in ["c.txt", "d.txt"] {
        std::fs::remove_file(dir.path().join(gone)).unwrap();
    }
    let _click = click_when_the_tree_swaps(cx, handle, &view, d_check);
    state(cx, &view).update(cx, AppState::refresh);
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), ["a.txt", "b.txt"]);
    assert_eq!(read(cx, &view, AppState::selected_count), 0);

    // a.txt's id names b.txt in the next tree.
    let (a_row, _) = drawn_at(cx, handle, &view, "a.txt");
    std::fs::remove_file(dir.path().join("a.txt")).unwrap();
    let _click = click_when_the_tree_swaps(cx, handle, &view, a_row);
    state(cx, &view).update(cx, AppState::refresh);
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), ["b.txt"]);
    assert_eq!(current(cx, &view), None);
    assert_eq!(read(cx, &view, AppState::selected_count), 0);
}
