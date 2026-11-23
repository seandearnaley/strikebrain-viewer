//! The toolbar and the selection bar.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Role, TestAppContext};
use viewer::AppState;

use crate::{PROJECT, click, focus, node, open_app, press, read, row, rows_now, tree};

#[gpui_kit::test]
fn every_toolbar_button_is_a_labelled_icon(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("toolbar").role(), Some(Role::Toolbar));
        for (id, label) in [
            ("expand-all", "Expand all"),
            ("collapse-all", "Collapse all"),
            ("show-ignored", "Show ignored"),
            ("refresh", "Refresh"),
            ("select-all", "Select all"),
        ] {
            let button = window.find(id);
            assert_eq!(button.label(), Some(label), "{id}");
            // 24 px icon buttons in a 36 px bar.
            assert_eq!(button.bounds().size.height, gpui_kit::px(24.), "{id}");
        }
        assert_eq!(
            window.find("toolbar").bounds().size.height,
            gpui_kit::px(36.)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn expand_all_and_collapse_all_open_and_close_every_folder(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));

    click(cx, handle, "expand-all");
    assert_eq!(
        rows_now(cx, handle),
        [
            "docs",
            "a.md",
            "b.md",
            "src",
            "app",
            "view.rs",
            "lib.rs",
            "main.rs",
            "file2.txt",
            "file10.txt",
            "README.md"
        ]
    );

    // Focus on a file deep inside src moves to src when it closes.
    for _ in 0..5 {
        press(cx, handle, "down");
    }
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "src/app/view.rs")));
    click(cx, handle, "collapse-all");
    assert_eq!(
        rows_now(cx, handle),
        ["docs", "src", "file2.txt", "file10.txt", "README.md"]
    );
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "src")));
}

#[gpui_kit::test]
fn select_all_turns_into_clear_while_anything_is_checked(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let readme = node(cx, &view, "README.md");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("clear").is_none());
        window.within(row(readme)).click("check", cx);
        assert!(window.try_find("select-all").is_none());
        assert_eq!(window.find("clear").label(), Some("Clear"));
    })
    .unwrap();

    click(cx, handle, "clear");
    assert_eq!(read(cx, &view, AppState::selected_count), 0);
    assert_eq!(read(cx, &view, AppState::current), None);

    click(cx, handle, "select-all");
    assert_eq!(read(cx, &view, AppState::selected_count), 8);
    // An empty stage takes the first checked file.
    assert_eq!(
        read(cx, &view, AppState::current),
        Some(node(cx, &view, "docs/a.md"))
    );
}

#[gpui_kit::test]
fn the_selection_bar_counts_checked_files_and_clears_them(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let docs = node(cx, &view, "docs");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("selection-bar").is_none());
        window.within(row(docs)).click("check", cx);
        assert_eq!(window.find("selection-bar").label(), Some("2 selected"));
    })
    .unwrap();

    click(cx, handle, "clear-selection");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("selection-bar").is_none());
        assert_eq!(window.find(row(docs)).checked(), Some(false));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_selection_bar_never_covers_a_row_and_never_moves_one(cx: &mut TestAppContext) {
    let names: Vec<String> = (0..200).map(|i| format!("file{i:03}.txt")).collect();
    let paths: Vec<&str> = names.iter().map(String::as_str).collect();
    let (handle, view) = open_app(cx, tree(&paths));
    let (first, last) = (
        node(cx, &view, "file000.txt"),
        node(cx, &view, "file199.txt"),
    );

    let top_before = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find(row(first)).bounds().top()
        })
        .unwrap();
    press(cx, handle, "space");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Showing the bar moved no row.
        assert_eq!(window.find(row(first)).bounds().top(), top_before);
    })
    .unwrap();

    press(cx, handle, "end");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let bar = window.find("selection-bar").bounds();
        let last_row = window.find(row(last)).bounds();
        assert!(last_row.bottom() <= bar.top(), "{last_row:?} under {bar:?}");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_selection_bar_groups_the_digits_of_its_count(cx: &mut TestAppContext) {
    let names: Vec<String> = (0..1_234).map(|i| format!("f{i:04}")).collect();
    let paths: Vec<&str> = names.iter().map(String::as_str).collect();
    let (handle, _) = open_app(cx, tree(&paths));

    click(cx, handle, "select-all");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("selection-bar").label(), Some("1,234 selected"));
    })
    .unwrap();
}
