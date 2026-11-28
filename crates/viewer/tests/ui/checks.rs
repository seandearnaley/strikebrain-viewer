//! Checkboxes, the selection bar and the file on the stage.

use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use viewer::keys::SelectAllFiles;

use crate::{PROJECT, click, current, dispatch, node, open_app, press, row, stage_now, tree};

#[gpui_kit::test]
fn a_mixed_folder_completes_on_click(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let docs = node(cx, &view, "docs");
    let (a, b) = (node(cx, &view, "docs/a.md"), node(cx, &view, "docs/b.md"));

    cx.update_window(handle, |_, window, cx| {
        window.within(row(docs)).click("chevron", cx);
        window.within(row(a)).click("check", cx);
        let folder = window.within(row(docs)).find("check");
        assert_eq!(folder.indeterminate(), Some(true));

        window.within(row(docs)).click("check", cx);
        assert_eq!(window.within(row(docs)).find("check").checked(), Some(true));
        assert_eq!(window.within(row(b)).find("check").checked(), Some(true));
        assert_eq!(window.find(row(docs)).checked(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_fully_checked_folder_clears_on_click(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let docs = node(cx, &view, "docs");
    let (a, b) = (node(cx, &view, "docs/a.md"), node(cx, &view, "docs/b.md"));

    cx.update_window(handle, |_, window, cx| {
        window.within(row(docs)).click("chevron", cx);
        window.within(row(docs)).click("check", cx);
        assert_eq!(window.within(row(a)).find("check").checked(), Some(true));
        assert_eq!(window.within(row(b)).find("check").checked(), Some(true));

        window.within(row(docs)).click("check", cx);
        assert_eq!(
            window.within(row(docs)).find("check").checked(),
            Some(false)
        );
        assert_eq!(window.within(row(a)).find("check").checked(), Some(false));
        assert_eq!(window.within(row(b)).find("check").checked(), Some(false));
    })
    .unwrap();
}

#[gpui_kit::test]
fn space_toggles_the_checkbox(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let (docs, file2) = (node(cx, &view, "docs"), node(cx, &view, "file2.txt"));

    cx.update_window(handle, |_, window, cx| {
        window.press("space", cx);
        assert_eq!(window.find(row(docs)).checked(), Some(true));
        window.press("space", cx);
        assert_eq!(window.find(row(docs)).checked(), Some(false));

        window.press("down", cx);
        window.press("down", cx);
        window.press("space", cx);
        assert_eq!(window.find(row(file2)).checked(), Some(true));
    })
    .unwrap();
    assert_eq!(stage_now(cx, handle).as_deref(), Some("file2.txt"));
}

#[gpui_kit::test]
fn checking_a_file_shows_it_on_the_stage(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let file10 = node(cx, &view, "file10.txt");

    assert_eq!(stage_now(cx, handle), None);

    cx.update_window(handle, |_, window, cx| {
        window.within(row(file10)).click("check", cx);
        assert_eq!(window.find(row(file10)).selected(), Some(true));
    })
    .unwrap();
    assert_eq!(stage_now(cx, handle).as_deref(), Some("file10.txt"));
    assert_eq!(current(cx, &view), Some(file10));
}

#[gpui_kit::test]
fn clicking_a_file_row_shows_it_checked_or_not(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let readme = node(cx, &view, "README.md");

    cx.update_window(handle, |_, window, cx| {
        window.click(row(readme), cx);
        assert_eq!(window.find(row(readme)).checked(), Some(false));
        assert_eq!(window.find(row(readme)).selected(), Some(true));
    })
    .unwrap();
    assert_eq!(stage_now(cx, handle).as_deref(), Some("README.md"));
}

#[gpui_kit::test]
fn unchecking_the_current_file_moves_the_stage_to_its_neighbour(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let ids = ["file2.txt", "file10.txt", "README.md"].map(|path| node(cx, &view, path));
    let [file2, file10, readme] = ids;

    let check = |cx: &mut TestAppContext, file| {
        cx.update_window(handle, |_, window, cx| {
            window.within(row(file)).click("check", cx);
        })
        .unwrap();
    };
    for file in ids {
        check(cx, file);
    }
    // Three checked files show the gallery (milestone 7); the file on the
    // stage is the one checked last.
    assert_eq!(current(cx, &view), Some(readme));

    // The middle file: the stage moves to the next checked file.
    cx.update_window(handle, |_, window, cx| window.click(row(file10), cx))
        .unwrap();
    assert_eq!(stage_now(cx, handle).as_deref(), Some("file10.txt"));
    check(cx, file10);
    assert_eq!(current(cx, &view), Some(readme));

    // The last file: no next, so the previous one, the only one left.
    check(cx, readme);
    assert_eq!(stage_now(cx, handle).as_deref(), Some("file2.txt"));

    // The only file: the stage empties.
    check(cx, file2);
    assert_eq!(stage_now(cx, handle), None);
}

#[gpui_kit::test]
fn enter_opens_the_focused_folder_and_then_its_child(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let docs = node(cx, &view, "docs");
    let first = node(cx, &view, "docs/a.md");
    press(cx, handle, "enter");
    assert_eq!(current(cx, &view), Some(docs));
    assert!(crate::read(cx, &view, |s| s
        .explorer()
        .unwrap()
        .is_open(docs)));
    press(cx, handle, "enter");
    assert_eq!(current(cx, &view), Some(first));
    assert_eq!(crate::read(cx, &view, viewer::AppState::selected_count), 0);
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

    dispatch(cx, handle, SelectAllFiles);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("selection-bar").label(), Some("1,234 selected"));
    })
    .unwrap();
}
