//! The saved state, end to end: real folders and a real state file in
//! temporary folders, saved by the app as it runs and quits, restored by a
//! second app launched over the same file.

use std::path::{Path, PathBuf};

use explorer::{Direction, Kind, SavedSelection, Sort, SortKey};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, ScrollDelta, TestAppContext, WindowBounds, point, px,
    size,
};
use viewer::{
    AppState, AppView, Launch, SAVE_DELAY, Saved, StageView, StateStore, open_main_window,
};

use crate::{
    checked, click, drag_divider, filter_value, folder, labels, name_of, node, node_at, not_found,
    read, row, rows_now, under, write,
};

/// Launches the app as the binary does, without following the disk.
fn launch(
    cx: &mut TestAppContext,
    folders: &[&Path],
    store: &StateStore,
) -> (AnyWindowHandle, Entity<AppView>) {
    let opened = launch_unsettled(cx, folders, store);
    cx.run_until_parked();
    opened
}

/// Launches as [`launch`] does, but returns before the first scans land.
fn launch_unsettled(
    cx: &mut TestAppContext,
    folders: &[&Path],
    store: &StateStore,
) -> (AnyWindowHandle, Entity<AppView>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        viewer::init(cx);
    });
    cx.update(|cx| {
        let folders = folders.iter().map(|folder| folder.to_path_buf()).collect();
        open_main_window(folders, Some(store.clone()), false, cx).expect("open the main window")
    })
}

/// What the state file at `path` holds.
fn saved(path: &Path) -> Saved {
    StateStore::at(path).load().expect("a state file")
}

/// What a state file says about the folders: everything before the window,
/// whose display the test platform names afresh in each app.
fn folder_part(state_file: &str) -> &str {
    state_file
        .split_once("\"window\"")
        .map_or(state_file, |(folder, _)| folder)
}

/// Opens `dir`, checks every file, quits: the state file then holds a
/// view of `dir` with three checks, as one folder entry. Returns what the
/// file holds.
fn save_three_checked(cx: &mut TestAppContext, dir: &Path, store: &StateStore) -> String {
    let (_, view) = launch(cx, &[dir], store);
    let state = view.read_with(cx, |view, _| view.state().clone());
    state.update(cx, AppState::select_all);
    cx.run_until_parked();
    assert_eq!(read(cx, &view, AppState::selected_count), 3);
    cx.quit();
    let view = saved(store.path()).view;
    assert_eq!(view.folders.len(), 1);
    assert_eq!(view.folders[0].selection.folders, [""]);
    std::fs::read_to_string(store.path()).expect("saved on quit")
}

/// Everything a view shows that the saved state should bring back.
#[derive(Debug, PartialEq)]
struct Seen {
    title: Option<String>,
    /// Every row, on screen or scrolled away.
    rows: Vec<String>,
    /// The first row drawn.
    top: Option<String>,
    open: Vec<String>,
    checked: Vec<String>,
    current: Option<String>,
    query: String,
    shows_ignored: bool,
    sort: Sort,
    explorer_width: f32,
    window: WindowBounds,
}

fn seen(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<AppView>) -> Seen {
    let query = filter_value(cx, handle);
    let (top, window) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            (labels(window).first().cloned(), window.window_bounds())
        })
        .expect("read the window");
    let (title, explorer_width) = cx.update(|cx| {
        let view = view.read(cx);
        (
            view.title().map(ToString::to_string),
            view.explorer_width(cx),
        )
    });
    read(cx, view, |state| {
        let explorer = state.explorer().expect("a tree is on screen");
        let tree = explorer.tree();
        Seen {
            title,
            rows: explorer.rows().iter().map(|&id| tree.path(id)).collect(),
            top,
            open: explorer.open_paths().collect(),
            checked: explorer.selected_paths().collect(),
            current: state.current().map(|id| tree.path(id)),
            query,
            shows_ignored: state.shows_ignored(),
            sort: explorer.sort(),
            explorer_width,
            window,
        }
    })
}

#[gpui_kit::test]
fn quitting_and_reopening_returns_the_same_view(cx: &mut TestAppContext) {
    let mut files: Vec<(String, String)> = [
        (".gitignore", "secret.txt\n"),
        ("secret.txt", "s"),
        ("README.md", "read me first"),
        ("docs/a.md", "# A"),
        ("docs/b.md", "# B, a longer one"),
        ("src/main.rs", "fn main() {}"),
        ("src/lib.rs", ""),
    ]
    .iter()
    .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
    .collect();
    files.extend((0..60).map(|i| (format!("many/f{i:02}.txt"), "x".repeat(i))));
    let files: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let dir = folder(&files);
    let name = name_of(dir.path());
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));

    let (handle, view) = launch(cx, &[dir.path()], &store);
    // Show ignored, open two folders, check a folder and a file, put a
    // file on the stage, sort by size, filter, widen the explorer,
    // resize the window and scroll.
    click(cx, handle, "show-ignored");
    let [docs, many, readme] = ["docs", "many", "README.md"].map(|path| node(cx, &view, path));
    cx.update_window(handle, |_, window, cx| {
        window.within(row(readme)).click("check", cx);
        window.within(row(docs)).click("check", cx);
        window.within(row(docs)).click("chevron", cx);
        window.within(row(many)).click("chevron", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let b = node(cx, &view, "docs/b.md");
    click(cx, handle, row(b));
    click(cx, handle, "header-size");
    click(cx, handle, "header-size");
    cx.update_window(handle, |_, window, cx| {
        window.click("filter", cx);
        window.input("-secret", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.simulate_window_resize(handle, size(px(1100.), px(760.)));
    cx.run_until_parked();
    drag_divider(cx, handle, 450.);

    // The debounced save lands half a second after the changes.
    cx.executor().advance_clock(SAVE_DELAY);
    cx.run_until_parked();
    assert_eq!(saved(store.path()).view.query, "-secret");

    cx.update_window(handle, |_, window, cx| {
        window.scroll("tree", ScrollDelta::Pixels(point(px(0.), px(-300.))), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let before = seen(cx, handle, &view);
    assert_eq!(before.current, Some(format!("{name}/docs/b.md")));
    assert_eq!(
        before.checked,
        [
            format!("{name}/docs/a.md"),
            format!("{name}/docs/b.md"),
            format!("{name}/README.md")
        ]
    );
    assert_eq!(
        before.sort,
        Sort {
            key: SortKey::Size,
            direction: Direction::Descending
        }
    );
    assert!(before.shows_ignored);
    // Scrolled: the folder's row and its largest folder, first under Size,
    // are above the top.
    assert_ne!(before.top.as_deref(), Some(name.as_str()));
    assert_ne!(before.top.as_deref(), Some("many"));
    assert_eq!(before.explorer_width, 450.);
    assert_eq!(before.window.get_bounds().size, size(px(1100.), px(760.)));

    // Quitting saves the rest: the scroll alone changed nothing that
    // schedules a save.
    cx.quit();
    let mut cx = cx.new_app();
    let (handle, view) = launch(&mut cx, &[], &StateStore::at(store.path()));
    let after = seen(&mut cx, handle, &view);
    assert_eq!(after, before);
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 3);
    cx.quit();
}

#[gpui_kit::test]
fn several_folders_come_back_in_order_each_with_its_own_view(cx: &mut TestAppContext) {
    let one = folder(&[("a.txt", "a"), ("sub/b.txt", "b")]);
    let two = folder(&[("c.txt", "c"), ("d.txt", "d")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));

    let (handle, view) = launch(cx, &[one.path(), two.path()], &store);
    // In the first, the folder inside opened and a file in it checked; the
    // second closed with a file checked, on the stage.
    let sub = node(cx, &view, "sub");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(sub)).click("chevron", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let b = node(cx, &view, "sub/b.txt");
    let d = node_at(cx, &view, &two.path().join("d.txt"));
    let two_top = node_at(cx, &view, two.path());
    cx.update_window(handle, |_, window, cx| {
        window.within(row(b)).click("check", cx);
        window.within(row(d)).click("check", cx);
        window.within(row(two_top)).click("chevron", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let before = seen(cx, handle, &view);
    assert_eq!(before.title.as_deref(), Some("Strikebrain Viewer"));
    assert_eq!(
        before.current,
        Some(format!("{}/d.txt", name_of(two.path())))
    );
    let mut rows = under(one.path(), &["sub", "b.txt", "a.txt"]);
    rows.push(name_of(two.path()));
    assert_eq!(rows_now(cx, handle), rows);
    cx.quit();

    let file = saved(store.path());
    let paths: Vec<PathBuf> = file.view.folders.iter().map(|f| f.path.clone()).collect();
    let canonical = |dir: &Path| explorer::canonical(dir).unwrap();
    assert_eq!(paths, [canonical(one.path()), canonical(two.path())]);
    assert_eq!(file.view.folders[0].open, ["", "sub"]);
    assert_eq!(
        file.view.folders[1].selection,
        SavedSelection {
            folders: vec![],
            files: vec!["d.txt".into()],
        }
    );
    assert_eq!(file.view.current, Some(canonical(two.path()).join("d.txt")));

    let mut cx = cx.new_app();
    let (handle, view) = launch(&mut cx, &[], &StateStore::at(store.path()));
    assert_eq!(seen(&mut cx, handle, &view), before);
    assert_eq!(rows_now(&mut cx, handle), rows);
    cx.quit();
}

#[gpui_kit::test]
fn the_stage_comes_back_in_the_view_it_showed(cx: &mut TestAppContext) {
    let dir = folder(&[("a.rs", "a"), ("b.rs", "b"), ("c.rs", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    // The view, the file on the stage and, while the gallery shows, the
    // tile with focus.
    let shows = |cx: &mut TestAppContext, view: &Entity<AppView>| {
        read(cx, view, |state| {
            let tree = state.explorer().expect("a tree is on screen").tree();
            let name = |id| tree.name(id).to_owned();
            let tile = state.shows_gallery().then(|| state.tile().map(name));
            (
                state.stage_view(),
                state.current().map(name),
                tile.flatten(),
            )
        })
    };

    // The file view, on a file opened from the gallery.
    let (handle, view) = launch(cx, &[dir.path()], &store);
    click(cx, handle, "select-all");
    let b = node(cx, &view, "b.rs");
    click(cx, handle, ("tile", b.index()));
    let file_view = (StageView::File, Some("b.rs".to_owned()), None);
    assert_eq!(shows(cx, &view), file_view);
    cx.quit();
    assert_eq!(saved(store.path()).view.view, StageView::File);

    let mut cx = cx.new_app();
    let (_, view) = launch(&mut cx, &[], &StateStore::at(store.path()));
    assert_eq!(shows(&mut cx, &view), file_view);
    // Back to the gallery, then quit there.
    let state = view.read_with(&cx, |view, _| view.state().clone());
    state.update(&mut cx, AppState::show_gallery);
    cx.run_until_parked();
    cx.quit();
    assert_eq!(saved(store.path()).view.view, StageView::Gallery);

    let mut cx = cx.new_app();
    let (_, view) = launch(&mut cx, &[], &StateStore::at(store.path()));
    let b = Some("b.rs".to_owned());
    assert_eq!(shows(&mut cx, &view), (StageView::Gallery, b.clone(), b));
    cx.quit();
}

#[gpui_kit::test]
fn another_folder_opens_fresh_in_the_saved_window(cx: &mut TestAppContext) {
    let saved_folder = folder(&[("a.txt", "a"), ("b.txt", "b")]);
    let other = folder(&[("c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));

    let (handle, view) = launch(cx, &[saved_folder.path()], &store);
    let a = node(cx, &view, "a.txt");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(a)).click("check", cx);
    })
    .unwrap();
    drag_divider(cx, handle, 380.);
    cx.quit();

    let mut cx = cx.new_app();
    let (handle, view) = launch(&mut cx, &[other.path()], &StateStore::at(store.path()));
    assert_eq!(rows_now(&mut cx, handle), under(other.path(), &["c.txt"]));
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 0);
    // The explorer keeps its width: that belongs to the window.
    let width = cx.update(|cx| view.read(cx).explorer_width(cx));
    assert_eq!(width, 380.);
    cx.quit();
    // Exactly the folders asked for opened, and they are what is saved.
    let paths: Vec<PathBuf> = saved(store.path())
        .view
        .folders
        .into_iter()
        .map(|folder| folder.path)
        .collect();
    assert_eq!(paths, [explorer::canonical(other.path()).unwrap()]);
}

#[gpui_kit::test]
fn a_saved_folder_that_is_gone_or_a_broken_file_opens_empty(cx: &mut TestAppContext) {
    let state_dir = tempfile::tempdir().unwrap();
    let path = state_dir.path().join("state.json");

    // The saved folder was deleted.
    let gone = folder(&[("a.txt", "a")]);
    let store = StateStore::at(&path);
    let (_, view) = launch(cx, &[gone.path()], &store);
    assert!(read(cx, &view, |state| state.explorer().is_some()));
    cx.quit();
    std::fs::remove_dir_all(gone.path()).unwrap();
    let mut cx = cx.new_app();
    let (handle, view) = launch(&mut cx, &[], &StateStore::at(&path));
    assert!(read(&mut cx, &view, AppState::is_empty));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("add-folder").is_some());
    })
    .unwrap();
    cx.quit();
    // It was dropped: nothing of it is saved.
    assert_eq!(saved(&path).view.folders, []);

    // A file that is not a state: the same, and it is replaced on quit.
    write(state_dir.path(), "state.json", "{ not json");
    let mut cx = cx.new_app();
    let (_, view) = launch(&mut cx, &[], &StateStore::at(&path));
    assert!(read(&mut cx, &view, AppState::is_empty));
    cx.quit();
    let replaced = std::fs::read_to_string(&path).unwrap();
    assert!(replaced.contains("\"version\": 2"), "{replaced}");
}

#[gpui_kit::test]
fn a_view_is_restored_over_a_tree_built_in_memory(cx: &mut TestAppContext) {
    use std::sync::Arc;

    use explorer::{KindFilter, SavedFolder, SavedSelection, Tree};
    use viewer::{SavedView, Session};

    let tree = Arc::new(Tree::from_paths(&["docs/a.md", "docs/b.png", "src/main.rs"]).unwrap());
    // A tree built from paths is one folder whose path is empty.
    let session = Session {
        view: Some(SavedView {
            folders: vec![SavedFolder {
                path: PathBuf::new(),
                open: vec!["src".into(), "gone".into()],
                selection: SavedSelection {
                    folders: vec!["docs".into()],
                    files: vec!["missing.txt".into()],
                },
            }],
            current: Some("gone.txt".into()),
            kind: KindFilter::Text,
            ..SavedView::default()
        }),
        ..Session::default()
    };
    cx.update(|cx| {
        gpui_kit::init(cx);
        viewer::init(cx);
    });
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(gpui_kit::WindowOptions::default(), cx, |window, cx| {
            gpui_kit::AppContext::new(cx, |cx| {
                AppView::with_session(Launch::Tree(tree), session, window, cx)
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    // Text only: the picture is hidden; docs and src show open.
    assert_eq!(rows_now(cx, handle), ["docs", "a.md", "src", "main.rs"]);
    // The file on the stage is gone: the first checked file takes its place.
    let current = read(cx, &view, |state| {
        state
            .current()
            .map(|id| state.explorer().unwrap().tree().path(id))
    });
    assert_eq!(current.as_deref(), Some("docs/a.md"));
    assert_eq!(read(cx, &view, AppState::selected_count), 2);
    assert_eq!(filter_value(cx, handle), "");
}

#[gpui_kit::test]
fn refresh_during_the_first_scan_keeps_the_saved_view(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("sub/c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    let saved_file = save_three_checked(cx, dir.path(), &store);

    let mut cx = cx.new_app();
    let (_, view) = launch_unsettled(&mut cx, &[], &StateStore::at(store.path()));
    // View > Refresh before the first scan lands.
    let state = view.read_with(&cx, |view, _| view.state().clone());
    state.update(&mut cx, AppState::refresh);
    cx.run_until_parked();
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 3);
    cx.quit();
    let written = std::fs::read_to_string(store.path()).unwrap();
    assert_eq!(folder_part(&written), folder_part(&saved_file));
}

#[gpui_kit::test]
fn show_ignored_during_the_first_scan_keeps_the_saved_view(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("sub/c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    save_three_checked(cx, dir.path(), &store);

    let mut cx = cx.new_app();
    let (_, view) = launch_unsettled(&mut cx, &[], &StateStore::at(store.path()));
    let state = view.read_with(&cx, |view, _| view.state().clone());
    state.update(&mut cx, AppState::toggle_show_ignored);
    // Saved while the second scan runs: the waiting view, with the rule
    // just chosen.
    cx.executor().advance_clock(SAVE_DELAY);
    let waiting = saved(store.path()).view;
    assert!(waiting.show_ignored);
    assert_eq!(waiting.folders[0].selection.folders, [""]);
    cx.run_until_parked();
    assert!(read(&mut cx, &view, AppState::shows_ignored));
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 3);
    cx.quit();
}

#[gpui_kit::test]
fn a_folder_that_vanishes_keeps_its_checks_and_open_folders(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("sub/c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    let (handle, view) = launch(cx, &[dir.path()], &store);
    let state = view.read_with(cx, |view, _| view.state().clone());
    state.update(cx, AppState::select_all);
    let sub = node(cx, &view, "sub");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(sub)).click("chevron", cx);
    })
    .unwrap();
    cx.executor().advance_clock(SAVE_DELAY);
    cx.run_until_parked();
    let before = saved(store.path()).view.folders;
    assert_eq!(before[0].open, ["", "sub"]);
    assert_eq!(before[0].selection.folders, [""]);

    // The folder is moved away (or its volume unmounted), and a refresh
    // finds nothing: its row stays, unreadable, with the system's message.
    let moved = tempfile::tempdir().unwrap();
    std::fs::rename(dir.path(), moved.path().join("away")).unwrap();
    state.update(cx, AppState::refresh);
    cx.run_until_parked();
    let top = read(cx, &view, |state| {
        let tree = state.explorer().unwrap().tree();
        let top = tree.folders().next().unwrap();
        assert_eq!(tree.kind(top), Kind::Unreadable);
        top
    });
    let tooltip = read(cx, &view, |state| state.folder_tooltip(top)).unwrap();
    assert!(tooltip.ends_with(&not_found()), "{tooltip}");
    assert_eq!(read(cx, &view, AppState::selected_count), 0);
    cx.executor().advance_clock(SAVE_DELAY);
    cx.run_until_parked();
    cx.quit();
    // What it had is what is saved.
    assert_eq!(saved(store.path()).view.folders, before);

    // Back where it was: the next launch finds it, with its view.
    std::fs::rename(moved.path().join("away"), dir.path()).unwrap();
    let mut cx = cx.new_app();
    let (handle, view) = launch(&mut cx, &[], &StateStore::at(store.path()));
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 3);
    assert_eq!(
        rows_now(&mut cx, handle),
        under(dir.path(), &["sub", "c.txt", "a.txt", "b.txt"])
    );
    cx.quit();
}

#[gpui_kit::test]
fn a_refresh_after_the_folder_returns_brings_its_view_back(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    let (handle, view) = launch(cx, &[dir.path()], &store);
    let state = view.read_with(cx, |view, _| view.state().clone());
    let b = node(cx, &view, "b.txt");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(b)).click("check", cx);
    })
    .unwrap();
    assert_eq!(read(cx, &view, AppState::selected_count), 1);

    let moved = tempfile::tempdir().unwrap();
    std::fs::rename(dir.path(), moved.path().join("away")).unwrap();
    state.update(cx, AppState::refresh);
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), under(dir.path(), &[]));
    std::fs::rename(moved.path().join("away"), dir.path()).unwrap();
    state.update(cx, AppState::refresh);
    cx.run_until_parked();
    assert_eq!(checked(cx, &view), ["b.txt"]);
    cx.quit();
}

#[gpui_kit::test]
fn a_saved_folder_missing_at_launch_is_dropped(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("sub/c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    save_three_checked(cx, dir.path(), &store);

    // Gone at launch: the empty state shows, and quitting forgets it.
    let moved = tempfile::tempdir().unwrap();
    std::fs::rename(dir.path(), moved.path().join("away")).unwrap();
    let mut cx = cx.new_app();
    let (_, view) = launch(&mut cx, &[], &StateStore::at(store.path()));
    assert!(read(&mut cx, &view, AppState::is_empty));
    cx.quit();
    assert_eq!(saved(store.path()).view.folders, []);
}

#[gpui_kit::test]
fn a_folder_argument_that_cannot_be_read_opens_as_its_row(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("sub/c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    save_three_checked(cx, dir.path(), &store);

    let mut cx = cx.new_app();
    let missing = state_dir.path().join("no such folder");
    let (handle, view) = launch(&mut cx, &[&missing], &StateStore::at(store.path()));
    assert_eq!(rows_now(&mut cx, handle), ["no such folder"]);
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 0);
    cx.quit();
    // Exactly the folders asked for: the one saved before is not. One that
    // is gone is where its parent resolves to.
    let paths: Vec<PathBuf> = saved(store.path())
        .view
        .folders
        .into_iter()
        .map(|folder| folder.path)
        .collect();
    let resolved = explorer::canonical(state_dir.path())
        .unwrap()
        .join("no such folder");
    assert_eq!(paths, [resolved]);
}

#[gpui_kit::test]
fn a_disk_change_during_the_first_scan_keeps_the_saved_top_row(cx: &mut TestAppContext) {
    // Under Size, largest first: f119.txt is the first row, f060.txt the
    // 60th, far enough down to need scrolling to.
    let names: Vec<(String, String)> = (0..120)
        .map(|i| (format!("f{i:03}.txt"), "x".repeat(i)))
        .collect();
    let files: Vec<(&str, &str)> = names
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    let dir = folder(&files);
    let root = explorer::canonical(dir.path()).unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let path = state_dir.path().join("state.json");
    // A state file from before several folders.
    let state_file = format!(
        r#"{{"version":1,"root":{root:?},"sort":{{"key":"size","direction":"descending"}},"first_visible_row":"f060.txt"}}"#
    );
    let focus_path = |cx: &mut TestAppContext, view: &Entity<AppView>| {
        read(cx, view, |state| {
            let tree = state.explorer().unwrap().tree();
            state.focus().map(|id| tree.abs_path(id))
        })
    };

    // Undisturbed, the saved top row takes focus once sizes are read.
    std::fs::write(&path, &state_file).unwrap();
    let (handle, view) = launch(cx, &[], &StateStore::at(&path));
    assert_eq!(focus_path(cx, &view), Some(root.join("f060.txt")));
    let top = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            labels(window).first().cloned()
        })
        .unwrap();
    assert_eq!(
        top.as_deref(),
        Some("f060.txt"),
        "control: the saved row is at the top"
    );
    cx.quit();

    // The watcher reports a change while the first scan runs.
    std::fs::write(&path, &state_file).unwrap();
    let mut cx = cx.new_app();
    let (handle, view) = launch_unsettled(&mut cx, &[], &StateStore::at(&path));
    let state = view.read_with(&cx, |view, _| view.state().clone());
    state.update(&mut cx, AppState::disk_changed);
    cx.run_until_parked();
    assert_eq!(read(&mut cx, &view, AppState::scans), 2);
    assert_eq!(focus_path(&mut cx, &view), Some(root.join("f060.txt")));
    let top = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            labels(window).first().cloned()
        })
        .unwrap();
    assert_eq!(
        top.as_deref(),
        Some("f060.txt"),
        "the saved row is at the top"
    );
    // The row at the top is saved from the tree now, not the old file.
    cx.update_window(handle, |_, window, cx| {
        window.scroll("tree", ScrollDelta::Pixels(point(px(0.), px(-290.))), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.quit();
    assert_eq!(saved(&path).view.top_row, Some(root.join("f050.txt")));
}

#[gpui_kit::test]
fn a_saved_explorer_wider_than_the_window_fits_it(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let root = explorer::canonical(dir.path()).unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let path = state_dir.path().join("state.json");
    std::fs::write(
        &path,
        format!(
            r#"{{"version":1,"root":{root:?},"explorer_width":3000.0,
            "window":{{"state":"windowed","x":0,"y":0,"width":1000,"height":700,"display":null}}}}"#
        ),
    )
    .unwrap();
    let (handle, view) = launch(cx, &[], &StateStore::at(&path));
    let (window, explorer, stage) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            (
                window.viewport_size(),
                window.find("explorer-pane").bounds(),
                window.try_find("stage-pane").map(|stage| stage.bounds()),
            )
        })
        .unwrap();
    assert_eq!(window.width, px(1000.));
    let stage = stage.expect("the stage is on screen");
    assert!(
        stage.size.width >= px(220.),
        "the stage keeps room: {stage:?}, explorer {explorer:?}"
    );
    assert!(explorer.right() <= window.width);
    // What is saved is what is drawn.
    let width = cx.update(|cx| view.read(cx).explorer_width(cx));
    assert_eq!(px(width), explorer.size.width);
    cx.quit();
}

#[gpui_kit::test]
fn a_named_folder_missing_at_launch_keeps_its_saved_view(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("sub/c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    save_three_checked(cx, dir.path(), &store);
    let before = saved(store.path()).view.folders;

    // Named on the command line while its volume is away: an unreadable
    // row that keeps the view saved for it.
    let moved = tempfile::tempdir().unwrap();
    std::fs::rename(dir.path(), moved.path().join("away")).unwrap();
    let mut cx = cx.new_app();
    let (_, view) = launch(&mut cx, &[dir.path()], &StateStore::at(store.path()));
    assert!(read(&mut cx, &view, |state| state.explorer().is_some()));
    cx.quit();
    assert_eq!(saved(store.path()).view.folders, before);

    // Back: the view returns.
    std::fs::rename(moved.path().join("away"), dir.path()).unwrap();
    let mut cx = cx.new_app();
    let (_, view) = launch(&mut cx, &[], &StateStore::at(store.path()));
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 3);
    cx.quit();
}

#[gpui_kit::test]
fn a_folder_never_read_saves_as_open(cx: &mut TestAppContext) {
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    let missing = state_dir.path().join("not yet");
    let (_, _view) = launch(cx, &[&missing], &store);
    cx.quit();
    let folders = saved(store.path()).view.folders;
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].open, [""]);
}

#[gpui_kit::test]
fn a_file_argument_takes_its_folders_saved_view_and_the_stage(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    save_three_checked(cx, dir.path(), &store);

    // Launched on a file of the saved folder: the folder comes back with
    // its checks, and the file asked for is on the stage, alone.
    let mut cx = cx.new_app();
    let file = dir.path().join("b.txt");
    let (handle, view) = launch(&mut cx, &[&file], &StateStore::at(store.path()));
    assert_eq!(
        rows_now(&mut cx, handle),
        under(dir.path(), &["a.txt", "b.txt", "c.txt"])
    );
    assert_eq!(checked(&mut cx, &view), ["a.txt", "b.txt", "c.txt"]);
    let b = node_at(&mut cx, &view, &file);
    assert_eq!(read(&mut cx, &view, AppState::current), Some(b));
    assert_eq!(read(&mut cx, &view, AppState::focus), Some(b));
    assert!(!read(&mut cx, &view, AppState::shows_gallery));
}
