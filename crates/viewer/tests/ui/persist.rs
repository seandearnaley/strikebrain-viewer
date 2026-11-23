//! The saved state, end to end: a real folder and a real state file in
//! temporary folders, saved by the app as it runs and quits, restored by a
//! second app launched over the same file.

use std::path::Path;

use explorer::{Direction, Sort, SortKey};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, ScrollDelta, TestAppContext, WindowBounds, point, px,
    size,
};
use viewer::{AppState, AppView, Launch, SAVE_DELAY, StateStore, open_main_window};

use crate::{click, drag_divider, filter_value, folder, labels, node, read, row, rows_now, write};

/// Launches the app as the binary does, without following the disk.
fn launch(
    cx: &mut TestAppContext,
    folder: Option<&Path>,
    store: &StateStore,
) -> (AnyWindowHandle, Entity<AppView>) {
    let opened = launch_unsettled(cx, folder, store);
    cx.run_until_parked();
    opened
}

/// Launches as [`launch`] does, but returns before the first scan lands.
fn launch_unsettled(
    cx: &mut TestAppContext,
    folder: Option<&Path>,
    store: &StateStore,
) -> (AnyWindowHandle, Entity<AppView>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        viewer::init(cx);
    });
    cx.update(|cx| {
        open_main_window(folder.map(Path::to_owned), Some(store.clone()), false, cx)
            .expect("open the main window")
    })
}

/// What a state file says about the folder: everything before the window,
/// whose display the test platform names afresh in each app.
fn folder_part(state_file: &str) -> &str {
    state_file
        .split_once("\"window\"")
        .map_or(state_file, |(folder, _)| folder)
}

/// Opens `dir`, checks every file, quits: the state file then holds a
/// view of `dir` with three checks. Returns what the file holds.
fn save_three_checked(cx: &mut TestAppContext, dir: &Path, store: &StateStore) -> String {
    let (_, view) = launch(cx, Some(dir), store);
    let state = view.read_with(cx, |view, _| view.state().clone());
    state.update(cx, AppState::select_all);
    cx.run_until_parked();
    assert_eq!(read(cx, &view, AppState::selected_count), 3);
    cx.quit();
    let saved = std::fs::read_to_string(store.path()).expect("saved on quit");
    assert!(
        saved.contains("\"folders\": [\n      \"\"\n    ]"),
        "{saved}"
    );
    saved
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
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));

    let (handle, view) = launch(cx, Some(dir.path()), &store);
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
    let written = std::fs::read_to_string(store.path()).expect("saved after the delay");
    assert!(written.contains("\"query\": \"-secret\""), "{written}");

    cx.update_window(handle, |_, window, cx| {
        window.scroll("tree", ScrollDelta::Pixels(point(px(0.), px(-300.))), cx);
        window.render_frame(cx);
    })
    .unwrap();
    let before = seen(cx, handle, &view);
    assert_eq!(before.current.as_deref(), Some("docs/b.md"));
    assert_eq!(before.checked, ["docs/a.md", "docs/b.md", "README.md"]);
    assert_eq!(
        before.sort,
        Sort {
            key: SortKey::Size,
            direction: Direction::Descending
        }
    );
    assert!(before.shows_ignored);
    // Scrolled: the largest folder, first under Size, is above the top.
    assert_ne!(before.top.as_deref(), Some("many"));
    assert_eq!(before.explorer_width, 450.);
    assert_eq!(before.window.get_bounds().size, size(px(1100.), px(760.)));

    // Quitting saves the rest: the scroll alone changed nothing that
    // schedules a save.
    cx.quit();
    let mut cx = cx.new_app();
    let (handle, view) = launch(&mut cx, None, &StateStore::at(store.path()));
    let after = seen(&mut cx, handle, &view);
    assert_eq!(after, before);
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 3);
    cx.quit();
}

#[gpui_kit::test]
fn another_folder_opens_fresh_in_the_saved_window(cx: &mut TestAppContext) {
    let saved_folder = folder(&[("a.txt", "a"), ("b.txt", "b")]);
    let other = folder(&[("c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));

    let (handle, view) = launch(cx, Some(saved_folder.path()), &store);
    let a = node(cx, &view, "a.txt");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(a)).click("check", cx);
    })
    .unwrap();
    drag_divider(cx, handle, 380.);
    cx.quit();

    let mut cx = cx.new_app();
    let (handle, view) = launch(&mut cx, Some(other.path()), &StateStore::at(store.path()));
    assert_eq!(rows_now(&mut cx, handle), ["c.txt"]);
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 0);
    // The explorer keeps its width: that belongs to the window.
    let width = cx.update(|cx| view.read(cx).explorer_width(cx));
    assert_eq!(width, 380.);
    cx.quit();
}

#[gpui_kit::test]
fn a_saved_folder_that_is_gone_or_a_broken_file_opens_empty(cx: &mut TestAppContext) {
    let state_dir = tempfile::tempdir().unwrap();
    let path = state_dir.path().join("state.json");

    // The saved folder was deleted.
    let gone = folder(&[("a.txt", "a")]);
    let store = StateStore::at(&path);
    let (_, view) = launch(cx, Some(gone.path()), &store);
    assert!(read(cx, &view, |state| state.explorer().is_some()));
    cx.quit();
    std::fs::remove_dir_all(gone.path()).unwrap();
    let mut cx = cx.new_app();
    let (handle, view) = launch(&mut cx, None, &StateStore::at(&path));
    assert!(read(&mut cx, &view, AppState::is_empty));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("open-folder").is_some());
    })
    .unwrap();
    cx.quit();

    // A file that is not a state: the same, and it is replaced on quit.
    write(state_dir.path(), "state.json", "{ not json");
    let mut cx = cx.new_app();
    let (_, view) = launch(&mut cx, None, &StateStore::at(&path));
    assert!(read(&mut cx, &view, AppState::is_empty));
    cx.quit();
    let replaced = std::fs::read_to_string(&path).unwrap();
    assert!(replaced.contains("\"version\": 1"), "{replaced}");
}

#[gpui_kit::test]
fn a_view_is_restored_over_a_tree_built_in_memory(cx: &mut TestAppContext) {
    use std::sync::Arc;

    use explorer::{KindFilter, SavedSelection, Tree};
    use viewer::{FolderView, Session};

    let tree = Arc::new(Tree::from_paths(&["docs/a.md", "docs/b.png", "src/main.rs"]).unwrap());
    let session = Session {
        view: Some(FolderView {
            expanded: vec!["src".into(), "gone".into()],
            selected: SavedSelection {
                folders: vec!["docs".into()],
                files: vec!["missing.txt".into()],
            },
            current: Some("gone.txt".into()),
            kind: KindFilter::Text,
            ..FolderView::default()
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
    let saved = save_three_checked(cx, dir.path(), &store);

    let mut cx = cx.new_app();
    let (_, view) = launch_unsettled(&mut cx, None, &StateStore::at(store.path()));
    // View > Refresh before the first scan lands.
    let state = view.read_with(&cx, |view, _| view.state().clone());
    state.update(&mut cx, AppState::refresh);
    cx.run_until_parked();
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 3);
    cx.quit();
    let written = std::fs::read_to_string(store.path()).unwrap();
    assert_eq!(folder_part(&written), folder_part(&saved));
}

#[gpui_kit::test]
fn show_ignored_during_the_first_scan_keeps_the_saved_view(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("sub/c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    save_three_checked(cx, dir.path(), &store);

    let mut cx = cx.new_app();
    let (_, view) = launch_unsettled(&mut cx, None, &StateStore::at(store.path()));
    let state = view.read_with(&cx, |view, _| view.state().clone());
    state.update(&mut cx, AppState::toggle_show_ignored);
    // Saved while the second scan runs: the waiting view, with the rule
    // just chosen.
    cx.executor().advance_clock(SAVE_DELAY);
    let waiting = std::fs::read_to_string(store.path()).unwrap();
    assert!(waiting.contains("\"show_ignored\": true"), "{waiting}");
    assert!(
        waiting.contains("\"folders\": [\n      \"\"\n    ]"),
        "{waiting}"
    );
    cx.run_until_parked();
    assert!(read(&mut cx, &view, AppState::shows_ignored));
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 3);
    cx.quit();
}

#[gpui_kit::test]
fn a_folder_that_vanishes_keeps_its_saved_view(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("sub/c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    let (_, view) = launch(cx, Some(dir.path()), &store);
    let state = view.read_with(cx, |view, _| view.state().clone());
    state.update(cx, AppState::select_all);
    cx.executor().advance_clock(SAVE_DELAY);
    cx.run_until_parked();
    let saved = std::fs::read_to_string(store.path()).expect("saved after the delay");
    assert!(
        saved.contains("\"folders\": [\n      \"\"\n    ]"),
        "{saved}"
    );

    // The folder is moved away (or its volume unmounted), and a refresh
    // finds nothing.
    let moved = tempfile::tempdir().unwrap();
    std::fs::rename(dir.path(), moved.path().join("away")).unwrap();
    state.update(cx, AppState::refresh);
    cx.run_until_parked();
    assert!(read(cx, &view, |state| state.explorer().is_none()));
    cx.executor().advance_clock(SAVE_DELAY);
    cx.run_until_parked();
    cx.quit();
    assert_eq!(std::fs::read_to_string(store.path()).unwrap(), saved);

    // Back where it was: the next launch finds it, with its view.
    std::fs::rename(moved.path().join("away"), dir.path()).unwrap();
    let mut cx = cx.new_app();
    let (_, view) = launch(&mut cx, None, &StateStore::at(store.path()));
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 3);
    cx.quit();
}

#[gpui_kit::test]
fn a_refresh_after_the_folder_returns_brings_its_view_back(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    let (handle, view) = launch(cx, Some(dir.path()), &store);
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
    assert!(read(cx, &view, |state| state.explorer().is_none()));
    std::fs::rename(moved.path().join("away"), dir.path()).unwrap();
    state.update(cx, AppState::refresh);
    cx.run_until_parked();
    let checked = read(cx, &view, |state| {
        state
            .explorer()
            .unwrap()
            .selected_paths()
            .collect::<Vec<_>>()
    });
    assert_eq!(checked, ["b.txt"]);
    cx.quit();
}

#[gpui_kit::test]
fn a_saved_folder_missing_at_launch_leaves_the_state_file_alone(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("sub/c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    let saved = save_three_checked(cx, dir.path(), &store);

    // Its volume is not mounted at launch: the empty state shows, and
    // quitting keeps what was saved.
    let moved = tempfile::tempdir().unwrap();
    std::fs::rename(dir.path(), moved.path().join("away")).unwrap();
    let mut cx = cx.new_app();
    let (_, view) = launch(&mut cx, None, &StateStore::at(store.path()));
    assert!(read(&mut cx, &view, AppState::is_empty));
    cx.executor().advance_clock(SAVE_DELAY);
    cx.run_until_parked();
    cx.quit();
    let written = std::fs::read_to_string(store.path()).unwrap();
    assert_eq!(folder_part(&written), folder_part(&saved));

    // Mounted again: the view comes back.
    std::fs::rename(moved.path().join("away"), dir.path()).unwrap();
    let mut cx = cx.new_app();
    let (_, view) = launch(&mut cx, None, &StateStore::at(store.path()));
    assert_eq!(read(&mut cx, &view, AppState::selected_count), 3);
    cx.quit();
}

#[gpui_kit::test]
fn another_folder_that_fails_to_open_keeps_the_saved_view(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b"), ("sub/c.txt", "c")]);
    let state_dir = tempfile::tempdir().unwrap();
    let store = StateStore::at(state_dir.path().join("state.json"));
    let saved = save_three_checked(cx, dir.path(), &store);

    let mut cx = cx.new_app();
    let missing = state_dir.path().join("no such folder");
    let (_, view) = launch(&mut cx, Some(&missing), &StateStore::at(store.path()));
    assert!(read(&mut cx, &view, |state| state.explorer().is_none()));
    cx.quit();
    let written = std::fs::read_to_string(store.path()).unwrap();
    assert_eq!(folder_part(&written), folder_part(&saved));
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
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let path = state_dir.path().join("state.json");
    let state_file = format!(
        r#"{{"version":1,"root":{root:?},"sort":{{"key":"size","direction":"descending"}},"first_visible_row":"f060.txt"}}"#
    );
    let focus_path = |cx: &mut TestAppContext, view: &Entity<AppView>| {
        read(cx, view, |state| {
            state
                .focus()
                .map(|id| state.explorer().unwrap().tree().path(id))
        })
    };

    // Undisturbed, the saved top row takes focus once sizes are read.
    std::fs::write(&path, &state_file).unwrap();
    let (handle, view) = launch(cx, None, &StateStore::at(&path));
    assert_eq!(focus_path(cx, &view).as_deref(), Some("f060.txt"));
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
    let (handle, view) = launch_unsettled(&mut cx, None, &StateStore::at(&path));
    let state = view.read_with(&cx, |view, _| view.state().clone());
    state.update(&mut cx, AppState::disk_changed);
    cx.run_until_parked();
    assert_eq!(read(&mut cx, &view, AppState::scans), 2);
    assert_eq!(focus_path(&mut cx, &view).as_deref(), Some("f060.txt"));
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
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("\"first_visible_row\": \"f050.txt\""),
        "{written}"
    );
}
