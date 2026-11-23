//! Rows: order, indent, semantics, and opening folders.

use gpui_kit::AppContext as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Role, TestAppContext, px};

use crate::{PROJECT, focus, labels, node, open_app, row, stage, tree};

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
