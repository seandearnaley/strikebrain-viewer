//! Refresh and Show ignored, against real folders.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext};
use viewer::{AppState, Launch};

use crate::{
    checked, click, current, folder, node, open_app, press, read, row, rows_now, tree, type_query,
    under, write,
};

#[gpui_kit::test]
fn refresh_shows_the_disk_and_keeps_checks_and_the_stage(cx: &mut TestAppContext) {
    // z.txt stays unchecked, so the folder is only partly checked.
    let dir = folder(&[
        ("a.txt", "a"),
        ("b.txt", "b"),
        ("c.txt", "c"),
        ("z.txt", "z"),
    ]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &["a.txt", "b.txt", "c.txt", "z.txt"])
    );
    let ids = ["a.txt", "b.txt", "c.txt"].map(|path| node(cx, &view, path));
    cx.update_window(handle, |_, window, cx| {
        for id in ids {
            window.within(row(id)).click("check", cx);
        }
        // b.txt on the stage.
        window.click(row(ids[1]), cx);
    })
    .unwrap();
    assert_eq!(current(cx, &view), Some(ids[1]));

    std::fs::remove_file(dir.path().join("b.txt")).unwrap();
    write(dir.path(), "d.txt", "d");
    click(cx, handle, "refresh");

    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &["a.txt", "c.txt", "d.txt", "z.txt"])
    );
    let [a, c, d] = ["a.txt", "c.txt", "d.txt"].map(|path| node(cx, &view, path));
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find(row(a)).checked(), Some(true));
        assert_eq!(window.find(row(c)).checked(), Some(true));
        assert_eq!(window.find(row(d)).checked(), Some(false));
    })
    .unwrap();
    // The deleted file's place on the stage goes to the next checked file.
    assert_eq!(current(cx, &view), Some(c));
    assert_eq!(read(cx, &view, AppState::selected_count), 2);
}

#[gpui_kit::test]
fn a_file_added_to_a_fully_checked_folder_is_checked(cx: &mut TestAppContext) {
    // As a saved state restores a fully checked folder: every file in it.
    let dir = folder(&[("sub/a.txt", "a"), ("sub/b.txt", "b"), ("other.txt", "o")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let sub = node(cx, &view, "sub");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(sub)).click("check", cx);
    })
    .unwrap();
    write(dir.path(), "sub/new.txt", "n");
    write(dir.path(), "new.txt", "n");
    click(cx, handle, "refresh");
    assert_eq!(
        checked(cx, &view),
        ["sub/a.txt", "sub/b.txt", "sub/new.txt"]
    );
    assert_eq!(read(cx, &view, AppState::selected_count), 3);
}

#[gpui_kit::test]
fn refresh_keeps_open_folders_and_the_filter(cx: &mut TestAppContext) {
    let dir = folder(&[("src/main.rs", "fn main() {}"), ("notes.md", "n")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let src = node(cx, &view, "src");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(src)).click("chevron", cx);
    })
    .unwrap();
    type_query(cx, handle, "main");

    write(dir.path(), "src/lib.rs", "");
    click(cx, handle, "refresh");
    assert_eq!(rows_now(cx, handle), under(dir.path(), &["src", "main.rs"]));

    // Focus is still in the filter: Esc clears it.
    press(cx, handle, "escape");
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &["src", "lib.rs", "main.rs", "notes.md"])
    );
}

#[gpui_kit::test]
fn show_ignored_lists_ignored_files_only_while_on(cx: &mut TestAppContext) {
    let dir = folder(&[
        (".gitignore", "secret.txt\n"),
        ("secret.txt", "s"),
        ("keep.txt", "k"),
    ]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let keep = node(cx, &view, "keep.txt");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(keep)).click("check", cx);
    })
    .unwrap();
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &[".gitignore", "keep.txt"])
    );

    click(cx, handle, "show-ignored");
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &[".gitignore", "keep.txt", "secret.txt"])
    );
    assert!(read(cx, &view, AppState::shows_ignored));
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("show-ignored").checked(), Some(true));
    })
    .unwrap();
    // The check carried across by path.
    let keep = node(cx, &view, "keep.txt");
    assert!(read(cx, &view, |state| state
        .explorer()
        .unwrap()
        .is_selected(keep)));

    click(cx, handle, "show-ignored");
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &[".gitignore", "keep.txt"])
    );
    assert_eq!(read(cx, &view, AppState::selected_count), 1);
}

#[gpui_kit::test]
fn refresh_rescans_every_folder_each_on_its_own(cx: &mut TestAppContext) {
    let (one, two) = (folder(&[("a.txt", "a")]), folder(&[("b.txt", "b")]));
    let launch = Launch::Folders(vec![one.path().to_owned(), two.path().to_owned()]);
    let (handle, view) = open_app(cx, launch);
    assert_eq!(read(cx, &view, AppState::scans), 2);
    write(one.path(), "new.txt", "n");
    std::fs::remove_file(two.path().join("b.txt")).unwrap();
    click(cx, handle, "refresh");
    assert_eq!(read(cx, &view, AppState::scans), 4);
    let mut rows = under(one.path(), &["a.txt", "new.txt"]);
    rows.extend(under(two.path(), &[]));
    assert_eq!(rows_now(cx, handle), rows);
}

#[gpui_kit::test]
fn show_ignored_does_nothing_without_a_folder(cx: &mut TestAppContext) {
    // A tree built in memory has no folder to rescan.
    let (handle, view) = open_app(cx, tree(&["a.txt"]));

    click(cx, handle, "show-ignored");

    assert!(!read(cx, &view, AppState::shows_ignored));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("show-ignored").checked(), Some(false));
    })
    .unwrap();
}
