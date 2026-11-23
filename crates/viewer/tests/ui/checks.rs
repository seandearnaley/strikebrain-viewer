//! Checkboxes and the file on the stage.

use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;

use crate::{PROJECT, current, node, open_app, row, stage, tree};

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
        assert_eq!(stage(window).as_deref(), Some("file2.txt"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn checking_a_file_shows_it_on_the_stage(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let file10 = node(cx, &view, "file10.txt");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(stage(window), None);

        window.within(row(file10)).click("check", cx);
        assert_eq!(stage(window).as_deref(), Some("file10.txt"));
        assert_eq!(window.find(row(file10)).selected(), Some(true));
    })
    .unwrap();
    assert_eq!(current(cx, &view), Some(file10));
}

#[gpui_kit::test]
fn clicking_a_file_row_shows_it_checked_or_not(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let readme = node(cx, &view, "README.md");

    cx.update_window(handle, |_, window, cx| {
        window.click(row(readme), cx);
        assert_eq!(stage(window).as_deref(), Some("README.md"));
        assert_eq!(window.find(row(readme)).checked(), Some(false));
        assert_eq!(window.find(row(readme)).selected(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn unchecking_the_current_file_moves_the_stage_to_its_neighbour(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let ids = ["file2.txt", "file10.txt", "README.md"].map(|path| node(cx, &view, path));
    let [file2, file10, readme] = ids;

    cx.update_window(handle, |_, window, cx| {
        for file in ids {
            window.within(row(file)).click("check", cx);
        }
        assert_eq!(stage(window).as_deref(), Some("README.md"));

        // The middle file: the stage moves to the next checked file.
        window.click(row(file10), cx);
        window.within(row(file10)).click("check", cx);
        assert_eq!(stage(window).as_deref(), Some("README.md"));

        // The last file: no next, so the previous one.
        window.within(row(readme)).click("check", cx);
        assert_eq!(stage(window).as_deref(), Some("file2.txt"));

        // The only file: the stage empties.
        window.within(row(file2)).click("check", cx);
        assert_eq!(stage(window), None);
    })
    .unwrap();
}

#[gpui_kit::test]
fn enter_shows_a_file_and_does_nothing_on_a_folder(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));

    cx.update_window(handle, |_, window, cx| {
        // Focus starts on the first row, the docs folder.
        window.press("enter", cx);
        assert_eq!(stage(window), None);

        window.press("down", cx);
        window.press("down", cx);
        window.press("enter", cx);
        assert_eq!(stage(window).as_deref(), Some("file2.txt"));
    })
    .unwrap();
    let file2 = node(cx, &view, "file2.txt");
    assert_eq!(current(cx, &view), Some(file2));
    cx.update_window(handle, |_, window, _| {
        // Enter shows a file without checking it.
        assert_eq!(window.find(row(file2)).checked(), Some(false));
    })
    .unwrap();
}
