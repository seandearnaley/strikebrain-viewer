//! Several folders: each a top-level row, its remove button and ⌘⌫, the
//! filter over them, and the toolbar's Add Folder.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use viewer::{AppState, AppView, Launch};

use crate::{
    current, folder, labels, name_of, node, node_at, open_app, press, read, row, rows_now,
    type_query, under,
};

/// Two folders: `one` holding a.txt and docs/, `two` holding b.md.
fn two_folders(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<AppView>,
) {
    let one = folder(&[("a.txt", "a"), ("docs/guide.md", "g")]);
    let two = folder(&[("b.md", "b")]);
    let launch = Launch::Folders(vec![one.path().to_owned(), two.path().to_owned()]);
    let (handle, view) = open_app(cx, launch);
    (one, two, handle, view)
}

/// Whether the row of `id` shows its remove button, after a fresh frame.
fn shows_remove(cx: &mut TestAppContext, handle: AnyWindowHandle, id: explorer::NodeId) -> bool {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within(row(id)).try_find("remove-folder").is_some()
    })
    .expect("read the row")
}

#[gpui_kit::test]
fn a_folder_row_shows_its_remove_button_while_hovered(cx: &mut TestAppContext) {
    let (one, two, handle, view) = two_folders(cx);
    let (top_one, top_two) = (
        node_at(cx, &view, one.path()),
        node_at(cx, &view, two.path()),
    );
    let a = node(cx, &view, "a.txt");
    assert!(!shows_remove(cx, handle, top_one));
    assert!(!shows_remove(cx, handle, top_two));

    cx.update_window(handle, |_, window, cx| window.hover(row(top_two), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(shows_remove(cx, handle, top_two));
    assert!(!shows_remove(cx, handle, top_one));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let button = window.within(row(top_two)).find("remove-folder");
        assert_eq!(button.label(), Some("Remove Folder"));
        // Over the Size cell, in place of the size.
        assert!(window.within(row(top_two)).try_find("size").is_none());
        assert!(window.within(row(top_one)).try_find("size").is_some());
    })
    .unwrap();

    // A file's row has none, hovered or not.
    cx.update_window(handle, |_, window, cx| window.hover(row(a), cx))
        .unwrap();
    cx.run_until_parked();
    assert!(!shows_remove(cx, handle, a));
    assert!(!shows_remove(cx, handle, top_two));

    // A press removes the folder, and nothing else happens to the row.
    cx.update_window(handle, |_, window, cx| window.hover(row(top_two), cx))
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within(row(top_two)).click("remove-folder", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), under(one.path(), &["docs", "a.txt"]));
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some(name_of(one.path()).as_str()));
}

#[gpui_kit::test]
fn command_backspace_removes_the_focused_folder_with_its_checks(cx: &mut TestAppContext) {
    let (one, two, handle, view) = two_folders(cx);
    let top_two = node_at(cx, &view, two.path());
    let b = node_at(cx, &view, &two.path().join("b.md"));
    let a = node(cx, &view, "a.txt");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(a)).click("check", cx);
        window.within(row(b)).click("check", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(read(cx, &view, AppState::selected_count), 2);
    assert_eq!(current(cx, &view), Some(b));

    // The keys bring the focus to the second folder's row: its button shows
    // while the ring does.
    press(cx, handle, "end");
    press(cx, handle, "up");
    assert_eq!(read(cx, &view, AppState::focus), Some(top_two));
    assert!(shows_remove(cx, handle, top_two));

    // ⌘⌫ on a file row does nothing.
    press(cx, handle, "down");
    press(cx, handle, "cmd-backspace");
    assert_eq!(rows_now(cx, handle).len(), 5);

    press(cx, handle, "up");
    press(cx, handle, "cmd-backspace");
    assert_eq!(rows_now(cx, handle), under(one.path(), &["docs", "a.txt"]));
    // Its checks went with it; the stage moved to the checked file left.
    assert_eq!(read(cx, &view, AppState::selected_count), 1);
    assert_eq!(current(cx, &view), Some(a));
    // Focus is on the row now where it was.
    assert_eq!(read(cx, &view, AppState::focus), Some(a));
}

#[gpui_kit::test]
fn removing_the_last_folder_shows_the_empty_state(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let top = node(cx, &view, "");
    assert_eq!(read(cx, &view, AppState::focus), Some(top));
    press(cx, handle, "cmd-backspace");
    assert!(read(cx, &view, AppState::is_empty));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("tree").is_none());
        assert_eq!(window.find("add-folder").label(), Some("Add Folder"));
    })
    .unwrap();
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("Strikebrain Viewer"));
}

#[gpui_kit::test]
fn a_folder_with_no_match_hides_and_its_rows_sit_a_level_down(cx: &mut TestAppContext) {
    let (one, two, handle, view) = two_folders(cx);
    let (top_one, a, guide) = (
        node_at(cx, &view, one.path()),
        node(cx, &view, "a.txt"),
        node_at(cx, &view, &one.path().join("docs/guide.md")),
    );
    // A folder's own rows are indented one 12 px step from its row.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let mut check = |id| window.within(row(id)).find("check").bounds().left();
        assert_eq!(check(a) - check(top_one), gpui_kit::px(12.));
    })
    .unwrap();

    type_query(cx, handle, ".md");
    assert_eq!(
        rows_now(cx, handle),
        [
            name_of(one.path()),
            "docs".into(),
            "guide.md".into(),
            name_of(two.path()),
            "b.md".into()
        ]
    );
    assert!(read(cx, &view, |state| state.explorer().unwrap().row_of(guide)).is_some());
    type_query(cx, handle, " b");
    assert_eq!(rows_now(cx, handle), under(two.path(), &["b.md"]));
}

#[gpui_kit::test]
fn the_toolbar_adds_a_folder_left_of_show_ignored(cx: &mut TestAppContext) {
    let (_one, _two, handle, _view) = two_folders(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let add = window.find("add-folder");
        let show = window.find("show-ignored");
        assert_eq!(add.label(), Some("Add Folder"));
        assert!(add.bounds().right() <= show.bounds().left());
        assert_eq!(labels(window).len(), 5);
        window.click("add-folder", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
}
