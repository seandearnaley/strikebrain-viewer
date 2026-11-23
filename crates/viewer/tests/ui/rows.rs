//! Rows: order, indent, semantics, and opening folders.

use gpui_kit::AppContext as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Role, TestAppContext, px};
use viewer::{AppState, Launch};

use crate::{
    Locked, PROJECT, current, focus, folder, labels, node, open_app, press, read, row, rows_now,
    stage, tree,
};

#[gpui_kit::test]
fn rows_render_in_order_with_their_indent(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let (src, app) = (node(cx, &view, "src"), node(cx, &view, "src/app"));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            labels(window),
            ["docs", "src", "file2.txt", "file10.txt", "README.md"]
        );

        window.within(row(src)).click("chevron", cx);
        window.within(row(app)).click("chevron", cx);
        assert_eq!(
            labels(window),
            [
                "docs",
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

        // 28 px rows plus a 1 px rule; 20 px of indent per level.
        let src_row = window.find(row(src)).bounds();
        let app_row = window.find(row(app)).bounds();
        assert_eq!(src_row.size.height, px(29.));
        assert_eq!(app_row.top() - src_row.top(), px(29.));
        let src_check = window.within(row(src)).find("check").bounds();
        let app_check = window.within(row(app)).find("check").bounds();
        assert_eq!(app_check.left() - src_check.left(), px(20.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn folders_sort_before_files_and_file2_before_file10(cx: &mut TestAppContext) {
    let (handle, _) = open_app(
        cx,
        tree(&[
            "file10.txt",
            "zeta/x.txt",
            "file2.txt",
            "Alpha.txt",
            "beta/",
        ]),
    );

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            labels(window),
            ["beta", "zeta", "Alpha.txt", "file2.txt", "file10.txt"]
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn rows_carry_their_tree_semantics(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let (src, lib) = (node(cx, &view, "src"), node(cx, &view, "src/lib.rs"));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("tree").role(), Some(Role::Tree));
        window.within(row(src)).click("chevron", cx);

        let folder = window.find(row(src));
        assert_eq!(folder.role(), Some(Role::TreeItem));
        assert_eq!(folder.label(), Some("src"));
        assert_eq!(folder.expanded(), Some(true));
        assert_eq!(folder.checked(), Some(false));

        let file = window.find(row(lib));
        assert_eq!(file.expanded(), None);
        assert_eq!(file.selected(), Some(false));
    })
    .unwrap();
}

#[gpui_kit::test]
fn clicking_a_folder_row_does_not_open_it_the_chevron_does(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let src = node(cx, &view, "src");

    cx.update_window(handle, |_, window, cx| {
        window.click(row(src), cx);
        assert_eq!(window.find(row(src)).expanded(), Some(false));
        assert_eq!(labels(window).len(), 5);
        assert_eq!(stage(window), None);
    })
    .unwrap();
    assert_eq!(focus(cx, &view), Some(src));

    cx.update_window(handle, |_, window, cx| {
        window.within(row(src)).click("chevron", cx);
        assert_eq!(window.find(row(src)).expanded(), Some(true));
        assert_eq!(labels(window).len(), 8);

        window.within(row(src)).click("chevron", cx);
        assert_eq!(window.find(row(src)).expanded(), Some(false));
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_unreadable_folder_shows_a_warning_and_never_opens(cx: &mut TestAppContext) {
    let dir = folder(&[("locked/secret.txt", "x"), ("open.txt", "x")]);
    let Some(_lock) = Locked::new(&dir.path().join("locked")) else {
        return;
    };
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));
    let locked = node(cx, &view, "locked");
    assert_eq!(rows_now(cx, handle), ["locked", "open.txt"]);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let folder = window.find(row(locked));
        assert_eq!(folder.label(), Some("locked"));
        // No disclosure state: a screen reader hears no "collapsed".
        assert_eq!(folder.expanded(), None);
        assert!(window.within(row(locked)).find("warning").visible());
        // A press on the warning, where a chevron would be, opens nothing.
        window.within(row(locked)).click("chevron", cx);
    })
    .unwrap();
    assert_eq!(rows_now(cx, handle), ["locked", "open.txt"]);
    assert_eq!(focus(cx, &view), Some(locked));

    press(cx, handle, "right");
    assert_eq!(rows_now(cx, handle), ["locked", "open.txt"]);
    assert_eq!(focus(cx, &view), Some(locked));
}

#[gpui_kit::test]
fn a_symlink_is_a_file_row_that_never_opens(cx: &mut TestAppContext) {
    let dir = folder(&[("real/inside.txt", "x")]);
    std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));
    let link = node(cx, &view, "link");
    // Folders first; the link to a folder sorts with the files.
    assert_eq!(rows_now(cx, handle), ["real", "link"]);

    cx.update_window(handle, |_, window, cx| {
        let row_facts = window.find(row(link));
        assert_eq!(row_facts.expanded(), None);
        assert!(window.within(row(link)).try_find("warning").is_none());
        window.within(row(link)).click("chevron", cx);
    })
    .unwrap();
    // A press where a chevron would be is a press on the row: it shows the
    // link on the stage, as any file.
    assert_eq!(rows_now(cx, handle), ["real", "link"]);
    assert_eq!(current(cx, &view), Some(link));

    press(cx, handle, "right");
    assert_eq!(rows_now(cx, handle), ["real", "link"]);
    press(cx, handle, "space");
    assert_eq!(read(cx, &view, AppState::selected_count), 1);
}
