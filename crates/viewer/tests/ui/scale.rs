//! A hundred thousand rows: every frame builds only what is on screen,
//! while scrolling, filtering and sorting.

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext};

use crate::{click, labels, node, open_app, press, read, row, tree, type_query};

/// A 564 px tree shows 20 rows, partly the last; allow one either side.
const SCREENFUL: usize = 21;

/// 100,000 files named f000000 to f099999.
fn hundred_thousand(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, gpui_kit::Entity<viewer::AppView>) {
    let names: Vec<String> = (0..100_000).map(|i| format!("f{i:06}")).collect();
    let paths: Vec<&str> = names.iter().map(String::as_str).collect();
    open_app(cx, tree(&paths))
}

/// How many rows the last frame built.
fn built(cx: &mut TestAppContext, handle: AnyWindowHandle) -> usize {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.find_all_by_role(Role::TreeItem).len()
    })
    .expect("count the rows")
}

#[gpui_kit::test]
fn a_hundred_thousand_rows_build_only_a_screenful(cx: &mut TestAppContext) {
    let (handle, view) = hundred_thousand(cx);
    let last = node(cx, &view, "f099999");

    assert!(built(cx, handle) <= SCREENFUL);
    press(cx, handle, "end");
    assert!(built(cx, handle) <= SCREENFUL);
    cx.update_window(handle, |_, window, _| {
        assert!(window.find(row(last)).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn filtering_and_sorting_a_hundred_thousand_rows_build_only_a_screenful(cx: &mut TestAppContext) {
    let (handle, view) = hundred_thousand(cx);

    // 10,000 names hold "f09".
    type_query(cx, handle, "f09");
    assert!(built(cx, handle) <= SCREENFUL);
    cx.update_window(handle, |_, window, _| {
        assert_eq!(labels(window).first().map(String::as_str), Some("f090000"));
    })
    .unwrap();

    type_query(cx, handle, "999");
    assert!(built(cx, handle) <= SCREENFUL);

    // Clearing the query keeps the focus row, f099990, on screen.
    press(cx, handle, "escape");
    assert!(built(cx, handle) <= SCREENFUL);
    let focus = crate::focus(cx, &view).unwrap();
    assert_eq!(focus, node(cx, &view, "f099990"));

    // Name, descending: the focus row stays on screen at its new place.
    click(cx, handle, "header-name");
    assert!(built(cx, handle) <= SCREENFUL);
    let first = read(cx, &view, |state| state.explorer().unwrap().rows()[0]);
    assert_eq!(first, node(cx, &view, "f099999"));
    cx.update_window(handle, |_, window, _| {
        assert!(window.find(row(focus)).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn opening_a_wide_folder_only_draws_visible_children(cx: &mut TestAppContext) {
    let names: Vec<String> = (0..100_000).map(|i| format!("wide/f{i:06}")).collect();
    let paths: Vec<&str> = names.iter().map(String::as_str).collect();
    let (handle, view) = open_app(cx, tree(&paths));
    let folder = node(cx, &view, "wide");
    let first = node(cx, &view, "wide/f000000");
    let far = node(cx, &view, "wide/f099999");
    click(cx, handle, row(folder));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(("folder-entry", first.index())).visible());
        assert!(window.try_find(("folder-entry", far.index())).is_none());
    })
    .unwrap();
    assert_eq!(read(cx, &view, viewer::AppState::scans), 0);
}
