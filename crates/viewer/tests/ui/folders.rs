//! Several folders: each a top-level row, its remove button and ⌘⌫, the
//! filter over them, and the toolbar's Add Folder.

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use viewer::{AppState, AppView, Launch};

use crate::{
    current, folder, labels, name_of, node, node_at, not_found, open_app, press, read, row,
    rows_now, type_query, under,
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
    press(cx, handle, "secondary-backspace");
    assert_eq!(rows_now(cx, handle).len(), 5);

    press(cx, handle, "up");
    press(cx, handle, "secondary-backspace");
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
    press(cx, handle, "secondary-backspace");
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

#[gpui_kit::test]
fn the_row_at_the_top_stays_there_as_a_folder_above_it_changes(cx: &mut TestAppContext) {
    let names = |prefix: &str| -> Vec<(String, String)> {
        (0..40)
            .map(|i| (format!("{prefix}{i:02}.txt"), "x".to_owned()))
            .collect()
    };
    let (a, b) = (names("a"), names("b"));
    let (one, two) = (folder(&pairs(&a)), folder(&pairs(&b)));
    let launch = Launch::Folders(vec![one.path().to_owned(), two.path().to_owned()]);
    let (handle, view) = open_app(cx, launch);
    let top = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            labels(window).first().cloned().unwrap_or_default()
        })
        .unwrap()
    };
    // Scrolled to the end, into the second folder.
    cx.update_window(handle, |_, window, cx| {
        let delta = gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-5000.));
        window.scroll("tree", gpui_kit::ScrollDelta::Pixels(delta), cx);
    })
    .unwrap();
    let before = top(cx);
    assert!(before.starts_with('b'), "{before}");

    // A file appears in the first folder, above the top row.
    crate::write(one.path(), "a-new.txt", "n");
    crate::click(cx, handle, "refresh");
    let rows = |cx: &mut TestAppContext| read(cx, &view, |s| s.explorer().unwrap().rows().len());
    assert_eq!(rows(cx), 83);
    assert_eq!(top(cx), before);

    // The first folder goes.
    let first = node_at(cx, &view, one.path());
    crate::state(cx, &view).update(cx, |state, cx| state.remove_folder(first, cx));
    cx.run_until_parked();
    assert_eq!(rows(cx), 41);
    assert_eq!(top(cx), before);
}

/// Borrowed `(path, text)` pairs.
fn pairs(names: &[(String, String)]) -> Vec<(&str, &str)> {
    names
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect()
}

/// How many rows are named `name`, scrolled away or not.
fn count(cx: &mut TestAppContext, view: &Entity<AppView>, name: &str) -> usize {
    read(cx, view, |state| {
        let explorer = state.explorer().expect("a tree is on screen");
        let tree = explorer.tree();
        explorer
            .rows()
            .iter()
            .filter(|&&id| tree.name(id) == name)
            .count()
    })
}

fn title(cx: &mut TestAppContext, view: &Entity<AppView>) -> String {
    read(cx, view, |state| state.title().to_string())
}

/// Adds `paths` as the picker or the Dock would, and lets every task run.
fn add(cx: &mut TestAppContext, view: &Entity<AppView>, paths: &[&std::path::Path]) {
    let paths = paths.iter().map(|path| path.to_path_buf()).collect();
    crate::state(cx, view).update(cx, |state, cx| state.open_paths(paths, Vec::new(), cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn an_outer_folder_unreadable_then_readable_holds_the_inner_once(cx: &mut TestAppContext) {
    let dir = folder(&[("d/sub/x.txt", "x"), ("d/top.txt", "t")]);
    let (d, sub) = (dir.path().join("d"), dir.path().join("d/sub"));
    let (handle, view) = open_app(cx, Launch::Folders(vec![sub.clone()]));
    let Some(lock) = crate::Locked::unlisted(&d) else {
        return;
    };
    // It cannot be listed: an unreadable row, holding nothing.
    add(cx, &view, &[&d]);
    assert_eq!(rows_now(cx, handle), ["sub", "x.txt", "d"]);
    // Asked for again, the inner folder is open already.
    add(cx, &view, &[&sub]);
    assert_eq!(rows_now(cx, handle), ["sub", "x.txt", "d"]);

    drop(lock);
    crate::click(cx, handle, "refresh");
    // Readable now, it takes the inner folder in: one copy of each file.
    assert_eq!(rows_now(cx, handle), ["d", "sub", "x.txt", "top.txt"]);
    assert_eq!(count(cx, &view, "x.txt"), 1);
    assert_eq!(title(cx, &view), "d");
}

#[gpui_kit::test]
fn an_inner_folder_rescanned_beside_an_unreadable_outer_stays_its_own(cx: &mut TestAppContext) {
    let dir = folder(&[("d/sub/x.txt", "x"), ("d/top.txt", "t")]);
    let (d, sub) = (dir.path().join("d"), dir.path().join("d/sub"));
    let (handle, view) = open_app(cx, Launch::Folders(vec![sub]));
    let Some(_lock) = crate::Locked::unlisted(&d) else {
        return;
    };
    add(cx, &view, &[&d]);
    crate::click(cx, handle, "refresh");
    // Both rescanned: the inner one is still a folder of its own, with its
    // rows, its title share and its watcher.
    assert_eq!(rows_now(cx, handle), ["sub", "x.txt", "d"]);
    assert_eq!(title(cx, &view), "Strikebrain Viewer");
    assert_eq!(read(cx, &view, AppState::scans), 4);
}

/// A project whose ignore rules hide `target/`.
fn project() -> tempfile::TempDir {
    folder(&[
        (".gitignore", "target/\n"),
        ("src/main.rs", "fn main() {}"),
        ("target/out.o", "o"),
    ])
}

#[gpui_kit::test]
fn a_folder_an_open_folder_ignores_is_added_as_its_own_and_taken_in_once_shown(
    cx: &mut TestAppContext,
) {
    let dir = project();
    let target = dir.path().join("target");
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    // Not in the tree: added as a folder of its own.
    add(cx, &view, &[&target]);
    let mut rows = under(dir.path(), &["src", ".gitignore"]);
    rows.extend(["target".to_owned(), "out.o".to_owned()]);
    assert_eq!(rows_now(cx, handle), rows);
    assert_eq!(title(cx, &view), "Strikebrain Viewer");

    // Show ignored: the outer folder lists it now, and takes it in.
    crate::click(cx, handle, "show-ignored");
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &["src", "target", "out.o", ".gitignore"])
    );
    assert_eq!(count(cx, &view, "out.o"), 1);
    assert_eq!(title(cx, &view), name_of(dir.path()));
}

#[gpui_kit::test]
fn a_folder_added_around_open_folders_takes_in_only_those_it_shows(cx: &mut TestAppContext) {
    let dir = project();
    let (src, target) = (dir.path().join("src"), dir.path().join("target"));
    let (handle, view) = open_app(cx, Launch::Folders(vec![target, src]));
    add(cx, &view, &[dir.path()]);
    // src is in its tree, taken in open; target, which its ignore rules
    // hide, stays a folder of its own.
    let mut rows = vec!["target".to_owned(), "out.o".to_owned()];
    rows.extend(under(dir.path(), &["src", "main.rs", ".gitignore"]));
    assert_eq!(rows_now(cx, handle), rows);
    assert_eq!(count(cx, &view, "main.rs"), 1);
    // Two folders asked for, both followed.
    assert_eq!(title(cx, &view), "Strikebrain Viewer");
}

#[gpui_kit::test]
fn a_folder_asked_for_inside_one_still_scanning_waits_for_its_scan(cx: &mut TestAppContext) {
    let dir = project();
    let (src, target) = (dir.path().join("src"), dir.path().join("target"));
    let (handle, view) = open_app(cx, Launch::Empty);
    // Both asked for before the outer folder's first scan has run.
    let state = crate::state(cx, &view);
    state.update(cx, |state, cx| {
        state.open_paths(vec![dir.path().to_owned()], Vec::new(), cx);
        state.open_paths(vec![src.clone()], Vec::new(), cx);
    });
    cx.run_until_parked();
    // Listed by the outer folder: the tree opens to its row, the focus row.
    assert_eq!(
        read(cx, &view, AppState::focus),
        Some(node_at(cx, &view, &src))
    );
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &["src", ".gitignore"])
    );

    // Hidden by its ignore rules: a folder of its own once the scan lands.
    let (handle, view) = open_app(cx, Launch::Empty);
    let state = crate::state(cx, &view);
    state.update(cx, |state, cx| {
        state.open_paths(vec![dir.path().to_owned()], Vec::new(), cx);
        state.open_paths(vec![target.clone()], Vec::new(), cx);
    });
    cx.run_until_parked();
    let mut rows = under(dir.path(), &["src", ".gitignore"]);
    rows.extend(["target".to_owned(), "out.o".to_owned()]);
    assert_eq!(rows_now(cx, handle), rows);
}

/// Where the first row drawn sits below the top of the tree.
fn first_row_offset(cx: &mut TestAppContext, handle: AnyWindowHandle) -> gpui_kit::Pixels {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let tree = window.find("tree").bounds().top();
        let first = window
            .find_all_by_role(gpui_kit::Role::TreeItem)
            .into_iter()
            .find(gpui_kit::test::ElementSnapshot::visible)
            .expect("a row is drawn");
        first.bounds().top() - tree
    })
    .expect("read the tree")
}

#[gpui_kit::test]
fn a_rescan_leaves_the_scroll_where_it_was_to_the_pixel(cx: &mut TestAppContext) {
    let names: Vec<(String, String)> = (0..60)
        .map(|i| (format!("f{i:02}.txt"), "x".to_owned()))
        .collect();
    let dir = folder(&pairs(&names));
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    // Part of a row down: the first row drawn is cut by 10 px.
    cx.update_window(handle, |_, window, cx| {
        let delta = gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-10.));
        window.scroll("tree", gpui_kit::ScrollDelta::Pixels(delta), cx);
    })
    .unwrap();
    let before = first_row_offset(cx, handle);
    assert_eq!(before, gpui_kit::px(-10.));
    crate::click(cx, handle, "refresh");
    assert_eq!(read(cx, &view, AppState::scans), 2);
    assert_eq!(first_row_offset(cx, handle), before);
}

#[gpui_kit::test]
fn a_folder_added_while_the_app_runs_is_the_focus_row_on_screen(cx: &mut TestAppContext) {
    let names: Vec<(String, String)> = (0..60)
        .map(|i| (format!("f{i:02}.txt"), "x".to_owned()))
        .collect();
    let long = folder(&pairs(&names));
    let (handle, view) = open_app(cx, Launch::Folders(vec![long.path().to_owned()]));
    // The launch starts at the top.
    assert_eq!(rows_now(cx, handle)[0], name_of(long.path()));
    let other = folder(&[("b.txt", "b")]);
    add(cx, &view, &[other.path()]);
    let top = node_at(cx, &view, other.path());
    assert_eq!(read(cx, &view, AppState::focus), Some(top));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(row(top)).visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_folder_row_describes_its_path_and_why_it_cannot_be_read(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let missing = dir.path().join("gone");
    let launch = Launch::Folders(vec![dir.path().to_owned(), missing.clone()]);
    let (handle, view) = open_app(cx, launch);
    let (top, gone) = (node_at(cx, &view, dir.path()), node_at_missing(cx, &view));
    let path = explorer::canonical(dir.path()).unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // What a screen reader hears after the name: what the tooltip says.
        assert_eq!(window.find(row(top)).description(), path.to_str());
        // A folder that is gone is where its parent resolves to.
        let gone_at = path.join("gone");
        let expected = format!("{}\n{}", gone_at.display(), not_found());
        assert_eq!(
            window.find(row(gone)).description(),
            Some(expected.as_str())
        );
    })
    .unwrap();
}

/// The second top-level folder, which cannot be read.
fn node_at_missing(cx: &mut TestAppContext, view: &Entity<AppView>) -> explorer::NodeId {
    read(cx, view, |state| {
        let tree = state.explorer().expect("a tree is on screen").tree();
        tree.folders().nth(1).expect("two folders")
    })
}

#[gpui_kit::test]
fn the_sort_outlasts_the_last_folder(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    crate::click(cx, handle, "header-size");
    press(cx, handle, "secondary-backspace");
    assert!(read(cx, &view, AppState::is_empty));
    let other = folder(&[("b.txt", "b")]);
    add(cx, &view, &[other.path()]);
    let sort = read(cx, &view, |state| state.explorer().unwrap().sort());
    assert_eq!(sort.key, explorer::SortKey::Size);
}

#[gpui_kit::test]
fn a_folder_readable_again_shows_before_its_sizes_are_read(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let late = dir.path().join("late");
    let (handle, view) = open_app(cx, Launch::Folders(vec![late.clone()]));
    crate::write(&late, "a.txt", "abc");
    // The moment its tree is on screen, its sizes are still to be read.
    let sized_when_shown: std::rc::Rc<std::cell::Cell<Option<bool>>> = std::rc::Rc::default();
    let seen = std::rc::Rc::clone(&sized_when_shown);
    let state = crate::state(cx, &view);
    let _watch = cx.update(|cx| {
        cx.observe(&state, move |state, cx| {
            let Some(explorer) = state.read(cx).explorer() else {
                return;
            };
            let tree = explorer.tree();
            let top = tree.folders().next().expect("one folder");
            if seen.get().is_none() && tree.kind(top) == explorer::Kind::Dir {
                seen.set(Some(tree.size(top).is_some()));
            }
        })
    });
    crate::click(cx, handle, "refresh");
    assert_eq!(sized_when_shown.get(), Some(false));
    // And they come.
    let top = node_at(cx, &view, &late);
    assert_eq!(
        read(cx, &view, |state| state
            .explorer()
            .unwrap()
            .tree()
            .size(top)),
        Some(3)
    );
}

#[gpui_kit::test]
fn a_folder_removed_while_its_rescan_runs_stays_gone(cx: &mut TestAppContext) {
    let (one, two) = (folder(&[("a.txt", "a")]), folder(&[("b.txt", "b")]));
    let launch = Launch::Folders(vec![one.path().to_owned(), two.path().to_owned()]);
    let (handle, view) = open_app(cx, launch);
    let first = node_at(cx, &view, one.path());
    // Both rescans start; the first folder goes before either lands.
    crate::state(cx, &view).update(cx, |state, cx| {
        state.refresh(cx);
        state.remove_folder(first, cx);
    });
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), under(two.path(), &["b.txt"]));
    assert_eq!(title(cx, &view), name_of(two.path()));
}

#[gpui_kit::test]
fn a_folder_removed_while_its_sizes_are_read_stays_gone(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let (_, view) = open_app(cx, Launch::Empty);
    let state = crate::state(cx, &view);
    // The moment its tree lands, before its sizes are read, it goes.
    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let once = std::rc::Rc::clone(&fired);
    let _remove = cx.update(|cx| {
        cx.observe(&state, move |state, cx| {
            let landed = state.read(cx).explorer().and_then(|explorer| {
                let tree = explorer.tree();
                let top = tree.folders().next()?;
                tree.size(top).is_none().then_some(top)
            });
            if let Some(top) = landed.filter(|_| !once.get()) {
                once.set(true);
                state.update(cx, |state, cx| state.remove_folder(top, cx));
            }
        })
    });
    add(cx, &view, &[dir.path()]);
    assert!(fired.get(), "the tree landed before its sizes");
    assert!(read(cx, &view, AppState::is_empty));
}

#[gpui_kit::test]
fn a_size_sort_over_a_folder_that_cannot_be_read_settles(cx: &mut TestAppContext) {
    let dir = folder(&[("big.txt", "xxxxxxxx"), ("small.txt", "x")]);
    let missing = dir.path().join("gone");
    let launch = Launch::Folders(vec![dir.path().to_owned(), missing]);
    let (handle, view) = open_app(cx, launch);
    crate::click(cx, handle, "header-size");
    // Sorted inside the readable folder; the folders keep their order, and
    // every size is known, the unreadable folder's included.
    let mut rows = under(dir.path(), &["small.txt", "big.txt"]);
    rows.push("gone".to_owned());
    assert_eq!(rows_now(cx, handle), rows);
    assert!(read(cx, &view, |state| state
        .explorer()
        .unwrap()
        .tree()
        .has_sizes()));
}
