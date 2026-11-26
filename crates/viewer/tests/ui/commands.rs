//! The explorer's commands as actions: what the bar's More menu, the menu
//! bar and the keys dispatch, and what tests and agents dispatch by name.

use gpui_kit::TestAppContext;
use viewer::AppState;
use viewer::keys::{CollapseAll, DeselectAll, ExpandAll, SelectAllFiles};

use crate::{PROJECT, dispatch, focus, node, open_app, press, read, rows_now, tree};

#[gpui_kit::test]
fn expand_all_and_collapse_all_open_and_close_every_folder(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));

    dispatch(cx, handle, ExpandAll);
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
    dispatch(cx, handle, CollapseAll);
    assert_eq!(
        rows_now(cx, handle),
        ["docs", "src", "file2.txt", "file10.txt", "README.md"]
    );
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "src")));
}

#[gpui_kit::test]
fn select_all_and_deselect_all_check_and_clear_every_file(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));

    dispatch(cx, handle, SelectAllFiles);
    assert_eq!(read(cx, &view, AppState::selected_count), 8);
    // An empty stage takes the first checked file.
    assert_eq!(
        read(cx, &view, AppState::current),
        Some(node(cx, &view, "docs/a.md"))
    );

    dispatch(cx, handle, DeselectAll);
    assert_eq!(read(cx, &view, AppState::selected_count), 0);
    assert_eq!(read(cx, &view, AppState::current), None);
}
