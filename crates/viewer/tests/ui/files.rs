//! Files opened on their own: a top-level row each, their folders never
//! read; shown in an open folder that lists them; taken in by a folder
//! added later; removed, followed and never saved.

use gpui_kit::TestAppContext;
use viewer::{AppState, Launch, Session, StateStore};

use crate::{
    checked, current, focus, folder, node_at, open_app, open_app_with_session, read, rows_now,
    under,
};

#[gpui_kit::test]
fn a_file_argument_is_a_row_of_its_own_and_its_folder_is_not_read(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b")]);
    let file = dir.path().join("b.txt");
    let (handle, view) = open_app(cx, Launch::Folders(vec![file.clone()]));

    // The file alone: a.txt beside it was never listed.
    assert_eq!(rows_now(cx, handle), ["b.txt"]);
    let b = node_at(cx, &view, &file);
    assert_eq!(current(cx, &view), Some(b));
    assert_eq!(focus(cx, &view), Some(b));
    assert_eq!(read(cx, &view, AppState::selected_count), 0);
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("b.txt"));
}

#[gpui_kit::test]
fn a_file_handed_over_shows_in_the_open_folder_or_as_its_own_row(cx: &mut TestAppContext) {
    let dir = folder(&[("deep/down/a.txt", "a"), ("b.txt", "b")]);
    let other = folder(&[("c.txt", "c"), ("d.txt", "d")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));

    // A file the open folder lists: the folders above it open, and it shows.
    let inside = dir.path().join("deep/down/a.txt");
    view.update(cx, |view, cx| view.open_paths(vec![inside.clone()], cx));
    cx.run_until_parked();
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &["deep", "down", "a.txt", "b.txt"])
    );
    assert_eq!(current(cx, &view), Some(node_at(cx, &view, &inside)));

    // A file elsewhere: a row of its own after the folder, nothing beside it.
    let outside = other.path().join("c.txt");
    view.update(cx, |view, cx| view.open_paths(vec![outside.clone()], cx));
    cx.run_until_parked();
    let rows = rows_now(cx, handle);
    assert_eq!(rows.last().map(String::as_str), Some("c.txt"));
    assert!(!rows.iter().any(|row| row == "d.txt"));
    let c = node_at(cx, &view, &outside);
    assert_eq!(current(cx, &view), Some(c));
    assert_eq!(focus(cx, &view), Some(c));
}

#[gpui_kit::test]
fn files_opened_together_are_rows_checked_and_shown_as_the_gallery(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("c.txt", "c")]);
    let other = folder(&[("d.txt", "d")]);
    let (b, d) = (dir.path().join("b.txt"), other.path().join("d.txt"));
    let (handle, view) = open_app(cx, Launch::Folders(vec![b.clone(), d.clone()]));

    assert_eq!(rows_now(cx, handle), ["b.txt", "d.txt"]);
    assert_eq!(checked(cx, &view), ["b.txt", "d.txt"]);
    assert!(read(cx, &view, AppState::shows_gallery));

    // One more file alone shows in the file view and is not checked.
    let a = dir.path().join("a.txt");
    view.update(cx, |view, cx| view.open_paths(vec![a.clone()], cx));
    cx.run_until_parked();
    assert_eq!(current(cx, &view), Some(node_at(cx, &view, &a)));
    assert!(!read(cx, &view, AppState::shows_gallery));
    assert_eq!(checked(cx, &view), ["b.txt", "d.txt"]);

    // Two handed over while running join the checks, one already there.
    let c = dir.path().join("c.txt");
    view.update(cx, |view, cx| {
        view.open_paths(vec![b.clone(), c.clone()], cx);
    });
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), ["b.txt", "d.txt", "a.txt", "c.txt"]);
    assert_eq!(checked(cx, &view), ["b.txt", "d.txt", "c.txt"]);
    assert!(read(cx, &view, AppState::shows_gallery));
    let c = node_at(cx, &view, &c);
    assert_eq!(read(cx, &view, AppState::tile), Some(c));
    assert_eq!(focus(cx, &view), Some(c));
}

#[gpui_kit::test]
fn a_folder_added_later_takes_in_a_file_row_with_its_check(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b")]);
    let (b, a) = (dir.path().join("b.txt"), dir.path().join("a.txt"));
    let (handle, view) = open_app(cx, Launch::Empty);
    // Two together, so both are checked.
    view.update(cx, |view, cx| {
        view.open_paths(vec![a.clone(), b.clone()], cx);
    });
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), ["a.txt", "b.txt"]);

    view.update(cx, |view, cx| {
        view.open_paths(vec![dir.path().to_owned()], cx);
    });
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), under(dir.path(), &["a.txt", "b.txt"]));
    assert_eq!(checked(cx, &view), ["a.txt", "b.txt"]);
}

#[gpui_kit::test]
fn a_file_its_folders_rules_hide_is_a_row_of_its_own_once_the_folder_lands(
    cx: &mut TestAppContext,
) {
    let dir = folder(&[(".gitignore", "*.log\n"), ("a.txt", "a"), ("b.log", "b")]);
    let log = dir.path().join("b.log");
    let (handle, view) = open_app(cx, Launch::Empty);
    // Asked for before the folder's first scan has run: it waits for it.
    let state = crate::state(cx, &view);
    state.update(cx, |state, cx| {
        state.open_paths(vec![dir.path().to_owned()], Vec::new(), cx);
        state.open_paths(vec![log.clone()], Vec::new(), cx);
    });
    cx.run_until_parked();
    let rows = rows_now(cx, handle);
    assert_eq!(rows.first(), Some(&crate::name_of(dir.path())));
    assert_eq!(rows.last().map(String::as_str), Some("b.log"));
    let b = node_at(cx, &view, &log);
    assert_eq!(current(cx, &view), Some(b));
    assert_eq!(focus(cx, &view), Some(b));
}

#[gpui_kit::test]
fn a_file_gone_before_it_is_read_never_shows(cx: &mut TestAppContext) {
    let other = folder(&[("c.txt", "c")]);
    let (_, view) = open_app(cx, Launch::Empty);
    let file = other.path().join("c.txt");
    view.update(cx, |view, cx| view.open_paths(vec![file.clone()], cx));
    std::fs::remove_file(&file).unwrap();
    cx.run_until_parked();
    assert_eq!(current(cx, &view), None);

    // It is forgotten: coming back later does not put it on the stage.
    std::fs::write(&file, "c").unwrap();
    let state = crate::state(cx, &view);
    state.update(cx, AppState::refresh);
    cx.run_until_parked();
    assert_eq!(current(cx, &view), None);
}

#[gpui_kit::test]
fn a_file_row_that_could_not_be_read_is_read_again_when_asked_for(cx: &mut TestAppContext) {
    let other = folder(&[("c.txt", "c")]);
    let (handle, view) = open_app(cx, Launch::Empty);
    let file = other.path().join("c.txt");
    view.update(cx, |view, cx| view.open_paths(vec![file.clone()], cx));
    std::fs::remove_file(&file).unwrap();
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), ["c.txt"]);
    let scans = read(cx, &view, AppState::scans);

    // Back on disk and asked for again: read again, and shown.
    std::fs::write(&file, "c").unwrap();
    view.update(cx, |view, cx| view.open_paths(vec![file.clone()], cx));
    cx.run_until_parked();
    assert_eq!(read(cx, &view, AppState::scans), scans + 1);
    let c = node_at(cx, &view, &file);
    assert_eq!(current(cx, &view), Some(c));
    // Readable: its tooltip is its path alone.
    let path = explorer::canonical(&file).unwrap();
    let tooltip = read(cx, &view, |state| state.folder_tooltip(c));
    assert_eq!(tooltip.as_deref(), path.to_str());
}

#[gpui_kit::test]
fn command_backspace_removes_a_file_row(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let file = dir.path().join("a.txt");
    let (handle, view) = open_app(cx, Launch::Folders(vec![file]));
    // Its row is the focus row.
    crate::press(cx, handle, "secondary-backspace");
    assert!(read(cx, &view, AppState::is_empty));
}

#[gpui_kit::test]
fn a_file_row_is_never_saved(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let other = folder(&[("c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let path = state_dir.path().join("state.json");
    let session = Session {
        store: Some(StateStore::at(&path)),
        ..Session::default()
    };
    let launch = Launch::Folders(vec![dir.path().to_owned()]);
    let (_, view) = open_app_with_session(cx, launch, session);
    let file = other.path().join("c.txt");
    view.update(cx, |view, cx| view.open_paths(vec![file.clone()], cx));
    cx.run_until_parked();
    assert_eq!(current(cx, &view), Some(node_at(cx, &view, &file)));
    cx.quit();

    // The folder alone; the file on the stage is saved as any other is.
    let saved = StateStore::at(&path).load().expect("saved on quit").view;
    let folders: Vec<_> = saved.folders.iter().map(|folder| &folder.path).collect();
    assert_eq!(folders, [&explorer::canonical(dir.path()).unwrap()]);
    assert_eq!(saved.current, Some(explorer::canonical(&file).unwrap()));
}
