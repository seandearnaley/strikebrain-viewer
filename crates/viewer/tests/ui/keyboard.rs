//! Keys over the tree, and the focus ring.

use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;

use crate::{
    PROJECT, click, filter_value, focus, is_focused, node, open_app, press, row, rows_now, tree,
    type_query,
};

#[gpui_kit::test]
fn up_and_down_move_focus_and_stop_at_the_ends(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let ids = ["docs", "src", "file2.txt", "README.md"].map(|path| node(cx, &view, path));
    let [docs, src, file2, readme] = ids;
    assert_eq!(focus(cx, &view), Some(docs));

    let press = |cx: &mut TestAppContext, key: &str| {
        cx.update_window(handle, |_, window, cx| window.press(key, cx))
            .unwrap();
    };
    press(cx, "up");
    assert_eq!(focus(cx, &view), Some(docs));
    press(cx, "down");
    assert_eq!(focus(cx, &view), Some(src));
    press(cx, "down");
    assert_eq!(focus(cx, &view), Some(file2));
    press(cx, "up");
    assert_eq!(focus(cx, &view), Some(src));
    for _ in 0..10 {
        press(cx, "down");
    }
    assert_eq!(focus(cx, &view), Some(readme));
}

#[gpui_kit::test]
fn right_opens_then_descends_and_left_closes_then_ascends(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let paths = ["src", "src/app", "src/app/view.rs"];
    let [src, app, view_rs] = paths.map(|path| node(cx, &view, path));

    let press = |cx: &mut TestAppContext, key: &str| {
        cx.update_window(handle, |_, window, cx| window.press(key, cx))
            .unwrap();
    };
    let expanded = |cx: &mut TestAppContext, id| {
        cx.update_window(handle, |_, window, _| window.find(row(id)).expanded())
            .unwrap()
    };

    press(cx, "down");
    assert_eq!(focus(cx, &view), Some(src));
    press(cx, "right");
    assert_eq!(
        (focus(cx, &view), expanded(cx, src)),
        (Some(src), Some(true))
    );
    press(cx, "right");
    assert_eq!(focus(cx, &view), Some(app));
    press(cx, "right");
    assert_eq!(
        (focus(cx, &view), expanded(cx, app)),
        (Some(app), Some(true))
    );
    press(cx, "right");
    assert_eq!(focus(cx, &view), Some(view_rs));
    // A file goes nowhere on →.
    press(cx, "right");
    assert_eq!(focus(cx, &view), Some(view_rs));

    press(cx, "left");
    assert_eq!(focus(cx, &view), Some(app));
    press(cx, "left");
    assert_eq!(
        (focus(cx, &view), expanded(cx, app)),
        (Some(app), Some(false))
    );
    press(cx, "left");
    assert_eq!(focus(cx, &view), Some(src));
    press(cx, "left");
    assert_eq!(
        (focus(cx, &view), expanded(cx, src)),
        (Some(src), Some(false))
    );
    // A top-level row has no parent row to go to.
    press(cx, "left");
    assert_eq!(focus(cx, &view), Some(src));
}

#[gpui_kit::test]
fn home_end_and_page_keys_jump_and_keep_focus_on_screen(cx: &mut TestAppContext) {
    let names: Vec<String> = (0..200).map(|i| format!("file{i:03}.txt")).collect();
    let paths: Vec<&str> = names.iter().map(String::as_str).collect();
    let (handle, view) = open_app(cx, tree(&paths));

    let press = |cx: &mut TestAppContext, key: &str| {
        cx.update_window(handle, |_, window, cx| window.press(key, cx))
            .unwrap();
    };
    let focus_row_visible = |cx: &mut TestAppContext| {
        let id = focus(cx, &view).unwrap();
        cx.update_window(handle, |_, window, _| {
            let tree = window.find("tree").bounds();
            let focused = window.find(row(id)).bounds();
            focused.top() >= tree.top() && focused.bottom() <= tree.bottom()
        })
        .unwrap()
    };

    press(cx, "end");
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "file199.txt")));
    assert!(focus_row_visible(cx));

    press(cx, "home");
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "file000.txt")));
    assert!(focus_row_visible(cx));

    // Under 136 px of toolbar, filter, kinds and header, a 700 px window
    // leaves a 564 px tree: 19 whole 29 px rows.
    press(cx, "pagedown");
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "file019.txt")));
    assert!(focus_row_visible(cx));
    press(cx, "pagedown");
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "file038.txt")));
    assert!(focus_row_visible(cx));
    press(cx, "pageup");
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "file019.txt")));
    assert!(focus_row_visible(cx));
}

#[gpui_kit::test]
fn the_focus_ring_shows_after_a_key_press_and_clears_on_pointer_down(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let (src, readme) = (node(cx, &view, "src"), node(cx, &view, "README.md"));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("ring").is_none());

        window.press("down", cx);
        assert!(window.within(row(src)).try_find("ring").is_some());

        window.click(row(readme), cx);
        assert!(window.try_find("ring").is_none());
        assert_eq!(window.find("tree").focused(), Some(true));
    })
    .unwrap();
    assert_eq!(focus(cx, &view), Some(readme));
}

#[gpui_kit::test]
fn a_toolbar_click_hides_the_ring_and_keeps_focus_in_the_tree(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));

    press(cx, handle, "down");
    let src = node(cx, &view, "src");
    cx.update_window(handle, |_, window, _| {
        assert!(window.within(row(src)).try_find("ring").is_some());
    })
    .unwrap();

    click(cx, handle, "expand-all");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("ring").is_none());
    })
    .unwrap();
    assert!(is_focused(cx, handle, "tree"));

    // The next key brings the ring back.
    press(cx, handle, "down");
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("ring").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn tab_moves_from_the_filter_to_the_tree_to_the_stage_and_round(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let docs = node(cx, &view, "docs");
    // The tree has focus at launch.
    assert!(is_focused(cx, handle, "tree"));

    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "filter"));
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "tree"));
    // Arriving by key shows the ring.
    cx.update_window(handle, |_, window, _| {
        assert!(window.within(row(docs)).try_find("ring").is_some());
    })
    .unwrap();

    press(cx, handle, "shift-tab");
    assert!(is_focused(cx, handle, "filter"));
    press(cx, handle, "shift-tab");
    assert!(is_focused(cx, handle, "stage"));
    press(cx, handle, "shift-tab");
    assert!(is_focused(cx, handle, "tree"));
}

#[gpui_kit::test]
fn escape_in_the_tree_clears_the_filter(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    type_query(cx, handle, "main");
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "tree"));

    press(cx, handle, "escape");

    assert_eq!(filter_value(cx, handle), "");
    assert_eq!(
        rows_now(cx, handle),
        ["docs", "src", "file2.txt", "file10.txt", "README.md"]
    );
    assert!(is_focused(cx, handle, "tree"));
}

#[gpui_kit::test]
fn escape_in_the_filter_clears_a_query_then_moves_to_the_tree(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    type_query(cx, handle, "main");

    press(cx, handle, "escape");
    assert_eq!(filter_value(cx, handle), "");
    assert_eq!(rows_now(cx, handle).len(), 5);
    assert!(is_focused(cx, handle, "filter"));

    press(cx, handle, "escape");
    assert!(is_focused(cx, handle, "tree"));
}

#[gpui_kit::test]
fn command_f_focuses_the_filter_from_the_tree_and_the_stage(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));

    press(cx, handle, "cmd-f");
    assert!(is_focused(cx, handle, "filter"));
    cx.update_window(handle, |_, window, cx| window.input("main", cx))
        .unwrap();
    cx.run_until_parked();

    click(cx, handle, "stage");
    assert!(is_focused(cx, handle, "stage"));
    press(cx, handle, "cmd-f");
    assert!(is_focused(cx, handle, "filter"));
    // The query is selected, so typing replaces it.
    cx.update_window(handle, |_, window, cx| window.input("lib", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(filter_value(cx, handle), "lib");
    assert_eq!(rows_now(cx, handle), ["src", "lib.rs"]);
}

#[gpui_kit::test]
fn escape_on_the_stage_returns_to_the_tree(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));

    click(cx, handle, "stage");
    assert!(is_focused(cx, handle, "stage"));
    press(cx, handle, "escape");
    assert!(is_focused(cx, handle, "tree"));
}
