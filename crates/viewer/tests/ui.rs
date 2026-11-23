//! UI integration tests: the real root view in a headless window, driven by
//! clicks and key presses, checked through accessibility facts, geometry
//! and the application state.

use std::path::PathBuf;
use std::sync::Arc;

use explorer::{NodeId, Tree};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, Point, Role, TestAppContext,
    Window, WindowBounds, WindowOptions, point, px, size,
};
use viewer::{AppView, EXPLORER_MIN_WIDTH, EXPLORER_WIDTH, Launch};

/// A small project. Display order: docs, src, file2.txt, file10.txt,
/// README.md; inside src: app, lib.rs, main.rs.
const PROJECT: &[&str] = &[
    "README.md",
    "file10.txt",
    "file2.txt",
    "docs/a.md",
    "docs/b.md",
    "src/main.rs",
    "src/lib.rs",
    "src/app/view.rs",
];

fn tree(paths: &[&str]) -> Launch {
    Launch::Tree(Arc::new(
        Tree::from_paths(paths).expect("the fixture paths form a tree"),
    ))
}

/// The folder `tests/fixture`, scanned from disk.
fn fixture_folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixture")
}

/// Opens the app in a 1000 x 700 window with the app's key bindings.
fn open_app(cx: &mut TestAppContext, launch: Launch) -> (AnyWindowHandle, Entity<AppView>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        viewer::init(cx);
    });
    let opened = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Point::default(),
                    size(px(1000.), px(700.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| AppView::new(launch, window, cx)),
        )
        .expect("open test window")
    });
    cx.run_until_parked();
    opened
}

/// The node at `path` in the tree on screen.
fn node(cx: &mut TestAppContext, view: &Entity<AppView>, path: &str) -> NodeId {
    cx.update(|cx| {
        let state = view.read(cx).state().read(cx);
        let explorer = state.explorer().expect("a tree is on screen");
        explorer
            .tree()
            .find(path)
            .expect("the path is in the fixture")
    })
}

fn row(id: NodeId) -> ElementId {
    ("row", id.index()).into()
}

/// The labels of the tree rows on screen, top to bottom.
fn labels(window: &Window) -> Vec<String> {
    window
        .find_all_by_role(Role::TreeItem)
        .iter()
        .filter(|row| row.visible())
        .map(|row| row.label().expect("every row has a label").to_owned())
        .collect()
}

/// The name on the stage, if any.
fn stage(window: &Window) -> Option<String> {
    window
        .try_find("stage-file")
        .map(|file| file.label().expect("the stage names its file").to_owned())
}

fn current(cx: &mut TestAppContext, view: &Entity<AppView>) -> Option<NodeId> {
    cx.update(|cx| view.read(cx).state().read(cx).current())
}

fn focus(cx: &mut TestAppContext, view: &Entity<AppView>) -> Option<NodeId> {
    cx.update(|cx| view.read(cx).state().read(cx).focus())
}

/// Drags the divider from its starting position to `to_x`, at the explorer's mid-height.
fn drag_divider(cx: &mut TestAppContext, handle: AnyWindowHandle, to_x: f32) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let explorer = window.find("explorer-pane").bounds();
        assert_eq!(explorer.right(), px(EXPLORER_WIDTH));

        let mid_height = explorer.center().y;
        window.drag(
            point(px(EXPLORER_WIDTH), mid_height),
            point(px(to_x), mid_height),
            cx,
        );
        window.render_frame(cx);
    })
    .expect("drag the divider in the test window");
}

// Layout

#[gpui_kit::test]
fn window_shows_explorer_left_of_stage(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, Launch::Empty);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);

        let explorer = window.find("explorer-pane");
        let stage = window.find("stage-pane");

        assert!(explorer.visible());
        assert!(stage.visible());
        // The divider is positioned absolutely over the panel boundary, so it
        // takes no width from either pane.
        assert_eq!(explorer.bounds().size.width, px(EXPLORER_WIDTH));
        assert!(stage.bounds().left() >= explorer.bounds().right());
        assert!(stage.bounds().size.width > px(600.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn dragging_the_divider_resizes_the_explorer(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, Launch::Empty);

    drag_divider(cx, handle, 400.);

    cx.update_window(handle, |_, window, _| {
        let explorer = window.find("explorer-pane").bounds();
        let stage = window.find("stage-pane").bounds();

        assert_eq!(explorer.size.width, px(400.));
        assert_eq!(stage.left(), explorer.right());
    })
    .unwrap();
}

#[gpui_kit::test]
fn explorer_cannot_shrink_below_its_minimum(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, Launch::Empty);

    drag_divider(cx, handle, 50.);

    cx.update_window(handle, |_, window, _| {
        let explorer = window.find("explorer-pane").bounds();

        assert_eq!(explorer.size.width, px(EXPLORER_MIN_WIDTH));
    })
    .unwrap();
}

// Rows

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

// Checks and the stage

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

// Keyboard

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

    // A 700 px tree holds 24 whole 29 px rows.
    press(cx, "pagedown");
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "file024.txt")));
    assert!(focus_row_visible(cx));
    press(cx, "pagedown");
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "file048.txt")));
    assert!(focus_row_visible(cx));
    press(cx, "pageup");
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "file024.txt")));
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

// Scale

#[gpui_kit::test]
fn a_hundred_thousand_rows_build_only_a_screenful(cx: &mut TestAppContext) {
    let names: Vec<String> = (0..100_000).map(|i| format!("f{i:06}")).collect();
    let paths: Vec<&str> = names.iter().map(String::as_str).collect();
    let (handle, view) = open_app(cx, tree(&paths));
    let last = node(cx, &view, "f099999");
    // A 700 px tree shows 25 rows, partly the last; allow one either side.
    let screenful = 26;

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let built = window.find_all_by_role(Role::TreeItem).len();
        assert!(built <= screenful, "built {built} rows");

        window.press("end", cx);
        let built = window.find_all_by_role(Role::TreeItem).len();
        assert!(built <= screenful, "built {built} rows");
        assert!(window.find(row(last)).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_size_column_shows_only_when_the_tree_is_wide(cx: &mut TestAppContext) {
    let (handle, _) = open_app(
        cx,
        Launch::Tree(Arc::new(
            Tree::from_paths_with_sizes(&[("big.bin", 3 * 1024 * 1024), ("note.txt", 512)])
                .unwrap(),
        )),
    );

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find_all("size").len(), 2);
    })
    .unwrap();

    drag_divider(cx, handle, 280.);

    cx.update_window(handle, |_, window, _| {
        assert!(window.find_all("size").is_empty());
    })
    .unwrap();
}

// Launch

#[gpui_kit::test]
fn a_folder_argument_scans_the_folder_and_names_the_window(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, Launch::Folder(fixture_folder()));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(labels(window), ["nested", "alpha.txt", "beta.md"]);
        assert_eq!(window.find("tree").label(), Some("fixture"));
        assert_eq!(window.find_all("size").len(), 3);
    })
    .unwrap();
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("fixture"));
}

#[gpui_kit::test]
fn an_unreadable_folder_shows_the_os_error(cx: &mut TestAppContext) {
    let missing = fixture_folder().join("no-such-folder");
    let (handle, _) = open_app(cx, Launch::Folder(missing));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let error = window.find("scan-error");
        assert!(error.visible());
        assert_eq!(
            error.label(),
            Some("No such file or directory (os error 2)")
        );
        assert!(window.try_find("tree").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn open_folder_and_command_o_show_the_folder_picker(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, Launch::Empty);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("tree").is_none());
        assert_eq!(stage(window), None);
        window.click("open-folder", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files && !options.multiple);
        Some(vec![fixture_folder()])
    });
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(labels(window), ["nested", "alpha.txt", "beta.md"]);
        window.press("cmd-o", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    // Cancelling the picker keeps the folder.
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("fixture"));
}
