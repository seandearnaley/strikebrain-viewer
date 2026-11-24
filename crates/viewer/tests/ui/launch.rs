//! Adding folders: from the command line, the picker and the system, and
//! a folder that cannot be read.

use explorer::NodeId;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Entity, ExternalPaths, FileDropEvent, InputEvent as _, TestAppContext, point,
    px,
};
use viewer::{AppState, AppView, Launch};

use crate::{
    fixture_folder, folder, labels, name_of, node, node_at, not_found, open_app, read, row,
    rows_now, stage, under,
};

/// The first top-level folder of the tree on screen.
fn first_folder(cx: &mut TestAppContext, view: &Entity<AppView>) -> NodeId {
    read(cx, view, |state| {
        let tree = state.explorer().expect("a tree is on screen").tree();
        tree.folders().next().expect("a folder")
    })
}

#[gpui_kit::test]
fn a_folder_argument_scans_the_folder_and_names_the_window(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, Launch::Folders(vec![fixture_folder()]));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // The folder is a top-level row, open, as it always is.
        assert_eq!(
            labels(window),
            ["fixture", "nested", "alpha.txt", "beta.md"]
        );
        assert_eq!(window.find("tree").label(), Some("fixture"));
        assert_eq!(window.find_all("size").len(), 4);
    })
    .unwrap();
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("fixture"));
}

#[gpui_kit::test]
fn several_folders_open_in_the_order_given_and_the_title_is_the_apps(cx: &mut TestAppContext) {
    let (b, a) = (folder(&[("b.txt", "b")]), folder(&[("a.txt", "a")]));
    let launch = Launch::Folders(vec![b.path().to_owned(), a.path().to_owned()]);
    let (handle, view) = open_app(cx, launch);
    assert_eq!(
        rows_now(cx, handle),
        [
            name_of(b.path()),
            "b.txt".into(),
            name_of(a.path()),
            "a.txt".into()
        ]
    );
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("Strikebrain Viewer"));
    // Each folder row's tooltip is its full path.
    let top = node_at(cx, &view, a.path());
    let tooltip = read(cx, &view, |state| state.folder_tooltip(top));
    let path = explorer::canonical(a.path()).unwrap();
    assert_eq!(tooltip.as_deref(), path.to_str());
    // A file's row has none.
    let file = node(cx, &view, "b.txt");
    assert_eq!(read(cx, &view, |state| state.folder_tooltip(file)), None);
}

#[gpui_kit::test]
fn an_unreadable_folder_is_a_row_with_the_os_error(cx: &mut TestAppContext) {
    let missing = fixture_folder().join("no-such-folder");
    let (handle, view) = open_app(cx, Launch::Folders(vec![missing]));

    let top = first_folder(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(labels(window), ["no-such-folder"]);
        // A warning where the chevron would be; it never opens.
        assert_eq!(window.find(row(top)).expanded(), None);
        assert!(window.within(row(top)).try_find("warning").is_some());
    })
    .unwrap();
    // The tooltip gives the path, where its parent resolves to, and the
    // operating system's message alone, without Rust's error code.
    let tooltip = read(cx, &view, |state| state.folder_tooltip(top)).unwrap();
    let resolved = explorer::canonical(&fixture_folder())
        .unwrap()
        .join("no-such-folder");
    assert_eq!(
        tooltip.as_ref(),
        format!("{}\n{}", resolved.display(), not_found())
    );
}

#[gpui_kit::test]
fn add_folder_and_command_o_show_the_picker_which_adds_several(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, Launch::Empty);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("tree").is_none());
        assert_eq!(stage(window), None);
        assert_eq!(window.find("add-folder").label(), Some("Add Folder"));
        window.click("add-folder", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    let other = folder(&[("c.txt", "c")]);
    let other_path = other.path().to_owned();
    cx.simulate_path_prompt_response(move |options| {
        assert!(options.directories && !options.files && options.multiple);
        Some(vec![fixture_folder(), other_path])
    });
    cx.run_until_parked();
    assert_eq!(
        rows_now(cx, handle),
        [
            "fixture".to_owned(),
            "nested".into(),
            "alpha.txt".into(),
            "beta.md".into(),
            name_of(other.path()),
            "c.txt".into()
        ]
    );

    // ⌘O adds beside what is open; cancelling the picker adds nothing.
    cx.update_window(handle, |_, window, cx| window.press("secondary-o", cx))
        .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle).len(), 6);
    // The toolbar's button does the same.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("add-folder").label(), Some("Add Folder"));
        window.click("add-folder", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("Strikebrain Viewer"));
}

#[gpui_kit::test]
fn a_folder_already_open_or_inside_one_is_not_added_again_but_shown(cx: &mut TestAppContext) {
    let dir = folder(&[("deep/inner/x.txt", "x"), ("top.txt", "t")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let rows = rows_now(cx, handle);
    assert_eq!(rows, [name_of(dir.path()), "deep".into(), "top.txt".into()]);

    // The same folder by another spelling, and a folder inside it.
    let state = crate::state(cx, &view);
    let inner = dir.path().join("deep/inner");
    state.update(cx, |state, cx| {
        state.open_paths(
            vec![dir.path().join("deep/.."), inner.clone()],
            Vec::new(),
            cx,
        );
    });
    cx.run_until_parked();
    assert_eq!(read(cx, &view, AppState::scans), 1, "nothing scanned again");
    // The inner folder's row opens and takes focus.
    assert_eq!(
        rows_now(cx, handle),
        [
            name_of(dir.path()),
            "deep".into(),
            "inner".into(),
            "top.txt".into()
        ]
    );
    let focus = read(cx, &view, AppState::focus);
    assert_eq!(focus, Some(node_at(cx, &view, &inner)));
}

#[gpui_kit::test]
fn a_folder_added_around_open_folders_takes_them_in(cx: &mut TestAppContext) {
    let dir = folder(&[("one/a.txt", "a"), ("one/b.txt", "b"), ("two/c.txt", "c")]);
    let one = dir.path().join("one");
    let (handle, view) = open_app(cx, Launch::Folders(vec![one.clone()]));
    let a = node(cx, &view, "a.txt");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(a)).click("check", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(read(cx, &view, AppState::selected_count), 1);

    let state = crate::state(cx, &view);
    state.update(cx, |state, cx| {
        state.open_paths(vec![dir.path().to_owned()], Vec::new(), cx);
    });
    cx.run_until_parked();
    // One folder now, holding the other, which stays open, with its check.
    assert_eq!(
        rows_now(cx, handle),
        [
            name_of(dir.path()),
            "one".into(),
            "a.txt".into(),
            "b.txt".into(),
            "two".into()
        ]
    );
    let checked = read(cx, &view, |state| {
        state
            .explorer()
            .unwrap()
            .selected_paths()
            .collect::<Vec<_>>()
    });
    assert_eq!(checked, [format!("{}/one/a.txt", name_of(dir.path()))]);
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some(name_of(dir.path()).as_str()));
}

#[gpui_kit::test]
fn a_file_argument_opens_its_folder_and_shows_the_file(cx: &mut TestAppContext) {
    let dir = folder(&[("notes/a.txt", "a"), ("notes/b.txt", "b"), ("c.txt", "c")]);
    let file = dir.path().join("notes/b.txt");
    let (handle, view) = open_app(cx, Launch::Folders(vec![file.clone()]));

    // The folder the file is in opens, not the file's place above it.
    let notes = dir.path().join("notes");
    assert_eq!(rows_now(cx, handle), under(&notes, &["a.txt", "b.txt"]));
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("notes"));
    // The file is on the stage, unchecked, and its row is the focus row.
    let b = node_at(cx, &view, &file);
    assert_eq!(crate::current(cx, &view), Some(b));
    assert_eq!(crate::focus(cx, &view), Some(b));
    assert_eq!(read(cx, &view, AppState::selected_count), 0);
}

#[gpui_kit::test]
fn a_file_handed_over_shows_in_the_open_folder_or_adds_its_own(cx: &mut TestAppContext) {
    let dir = folder(&[("deep/down/a.txt", "a"), ("b.txt", "b")]);
    let other = folder(&[("c.txt", "c")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    assert_eq!(rows_now(cx, handle), under(dir.path(), &["deep", "b.txt"]));

    // A file the open folder lists: no folder is added, the folders above
    // the file open, and it shows.
    let inside = dir.path().join("deep/down/a.txt");
    view.update(cx, |view, cx| view.open_paths(vec![inside.clone()], cx));
    cx.run_until_parked();
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &["deep", "down", "a.txt", "b.txt"])
    );
    let a = node_at(cx, &view, &inside);
    assert_eq!(crate::current(cx, &view), Some(a));
    assert_eq!(crate::focus(cx, &view), Some(a));

    // A file elsewhere: its folder is added after those open, and it shows
    // once that folder's scan lands.
    let outside = other.path().join("c.txt");
    view.update(cx, |view, cx| view.open_paths(vec![outside.clone()], cx));
    cx.run_until_parked();
    let rows = rows_now(cx, handle);
    assert_eq!(rows[rows.len() - 2..], under(other.path(), &["c.txt"]));
    let c = node_at(cx, &view, &outside);
    assert_eq!(crate::current(cx, &view), Some(c));
    assert_eq!(crate::focus(cx, &view), Some(c));
}

#[gpui_kit::test]
fn a_file_gone_before_its_folder_is_read_never_shows(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let other = folder(&[("c.txt", "c")]);
    let (_, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));

    // Asked for, then gone from disk before its folder is read.
    let file = other.path().join("c.txt");
    view.update(cx, |view, cx| view.open_paths(vec![file.clone()], cx));
    std::fs::remove_file(&file).unwrap();
    cx.run_until_parked();
    assert_eq!(crate::current(cx, &view), None);

    // It is forgotten: coming back later does not put it on the stage.
    std::fs::write(&file, "c").unwrap();
    let state = crate::state(cx, &view);
    state.update(cx, AppState::refresh);
    cx.run_until_parked();
    assert_eq!(crate::current(cx, &view), None);
}

#[gpui_kit::test]
fn a_file_handed_over_to_an_empty_window_opens_its_folder_and_shows(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b")]);
    let (handle, view) = open_app(cx, Launch::Empty);
    let file = dir.path().join("b.txt");
    view.update(cx, |view, cx| view.open_paths(vec![file.clone()], cx));
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), under(dir.path(), &["a.txt", "b.txt"]));
    let b = node_at(cx, &view, &file);
    assert_eq!(crate::current(cx, &view), Some(b));
    assert_eq!(crate::focus(cx, &view), Some(b));
}

#[gpui_kit::test]
fn files_opened_together_are_checked_and_show_as_the_gallery(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("c.txt", "c")]);
    let other = folder(&[("d.txt", "d")]);
    let (b, d) = (dir.path().join("b.txt"), other.path().join("d.txt"));
    let (handle, view) = open_app(cx, Launch::Folders(vec![b.clone(), d.clone()]));

    // Each file's folder opens, in the order asked for.
    let mut rows = under(dir.path(), &["a.txt", "b.txt", "c.txt"]);
    rows.extend(under(other.path(), &["d.txt"]));
    assert_eq!(rows_now(cx, handle), rows);
    // Both files are checked, nothing else, and the gallery shows them.
    assert_eq!(crate::checked(cx, &view), ["b.txt", "d.txt"]);
    assert!(read(cx, &view, AppState::shows_gallery));

    // One more file alone shows in the file view and is not checked.
    let a = dir.path().join("a.txt");
    view.update(cx, |view, cx| view.open_paths(vec![a.clone()], cx));
    cx.run_until_parked();
    assert_eq!(crate::current(cx, &view), Some(node_at(cx, &view, &a)));
    assert!(!read(cx, &view, AppState::shows_gallery));
    assert_eq!(crate::checked(cx, &view), ["b.txt", "d.txt"]);

    // Two handed over while running join the checks, one already there.
    let c = dir.path().join("c.txt");
    view.update(cx, |view, cx| {
        view.open_paths(vec![b.clone(), c.clone()], cx);
    });
    cx.run_until_parked();
    assert_eq!(crate::checked(cx, &view), ["b.txt", "c.txt", "d.txt"]);
    assert!(read(cx, &view, AppState::shows_gallery));
    let c = node_at(cx, &view, &c);
    assert_eq!(read(cx, &view, AppState::tile), Some(c));
    assert_eq!(crate::focus(cx, &view), Some(c));
}

#[gpui_kit::test]
fn a_folder_dropped_on_the_window_is_added(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, Launch::Empty);
    let dropped = folder(&[("c.txt", "c")]);
    let paths = ExternalPaths([dropped.path().to_owned()].into_iter().collect());
    let position = point(px(100.), px(100.));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let entered = FileDropEvent::Entered { position, paths };
        window.dispatch_event(entered.to_platform_input(), cx);
        window.render_frame(cx);
        window.dispatch_event(FileDropEvent::Submit { position }.to_platform_input(), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), under(dropped.path(), &["c.txt"]));
}
