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
    cx.update(|cx| {
        gpui_kit::init(cx);
        viewer::init(cx);
    });
    let opened = cx.update(|cx| {
        open_main_window(folder.map(Path::to_owned), Some(store.clone()), false, cx)
            .expect("open the main window")
    });
    cx.run_until_parked();
    opened
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
