//! Input and background work in every order the test scheduler can be
//! made to take: it runs nothing until the test parks, so each test lines
//! up its events and jobs first and only then lets them run.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use explorer::{Explorer, SortKey};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Pixels, Point, Subscription, TestAppContext,
};
use viewer::keys::{Refresh, ToggleShowIgnored};
use viewer::{AppState, AppView, Launch, ScanStatus};

use crate::{
    click_without_frame, current, filter_value, folder, node, open_app, read, row, rows_now, state,
    under, write,
};

/// The rows' names in an explorer.
fn names(explorer: &Explorer) -> Vec<String> {
    explorer
        .rows()
        .iter()
        .map(|&id| explorer.tree().name(id).to_owned())
        .collect()
}

/// Records the rows each time the state changes while a tree is on screen.
fn log_rows(
    cx: &mut TestAppContext,
    view: &Entity<AppView>,
) -> (Rc<RefCell<Vec<Vec<String>>>>, Subscription) {
    let seen: Rc<RefCell<Vec<Vec<String>>>> = Rc::default();
    let log = Rc::clone(&seen);
    let state = state(cx, view);
    let subscription = cx.update(|cx| {
        cx.observe(&state, move |state, cx| {
            if let Some(explorer) = state.read(cx).explorer() {
                log.borrow_mut().push(names(explorer));
            }
        })
    });
    (seen, subscription)
}

/// Arms a click at `position` that fires the moment another tree lands,
/// inside the update that swaps it and before any frame is drawn: the click
/// reaches the listeners the last frame drew for the old tree, as a click
/// does that the platform delivers between a swap and the next frame.
pub(crate) fn click_when_the_tree_swaps(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<AppView>,
    position: Point<Pixels>,
) -> Subscription {
    let tree = |state: &AppState| state.explorer().map(|e| Arc::as_ptr(e.tree()));
    let before = read(cx, view, tree);
    let armed = Rc::new(Cell::new(true));
    let state = state(cx, view);
    cx.update(|cx| {
        cx.observe(&state, move |state, cx| {
            if armed.get() && tree(state.read(cx)) != before {
                armed.set(false);
                cx.update_window(handle, |_, window, cx| {
                    click_without_frame(window, position, cx);
                })
                .expect("click in the test window");
            }
        })
    })
}

/// Where the last frame drew the row of `path`, and its checkbox.
fn drawn_at(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<AppView>,
    path: &str,
) -> (Point<Pixels>, Point<Pixels>) {
    let node = node(cx, view, path);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        (
            window.find(row(node)).bounds().center(),
            window.within(row(node)).find("check").bounds().center(),
        )
    })
    .expect("read the frame")
}

/// A click drawn before a refresh swapped the tree, delivered after the swap
/// and before the next frame: it lands on nothing, neither past the new
/// tree's end nor on the file that now has the old row's id.
#[gpui_kit::test]
fn a_click_drawn_before_a_refresh_does_nothing(cx: &mut TestAppContext) {
    let dir = folder(&[
        ("a.txt", "a"),
        ("b.txt", "b"),
        ("c.txt", "c"),
        ("d.txt", "d"),
    ]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));

    // d.txt's id is past the end of the next tree, which holds a and b.
    let (_, d_check) = drawn_at(cx, handle, &view, "d.txt");
    for gone in ["c.txt", "d.txt"] {
        std::fs::remove_file(dir.path().join(gone)).unwrap();
    }
    let _click = click_when_the_tree_swaps(cx, handle, &view, d_check);
    state(cx, &view).update(cx, AppState::refresh);
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), under(dir.path(), &["a.txt", "b.txt"]));
    assert_eq!(read(cx, &view, AppState::selected_count), 0);

    // a.txt's id names b.txt in the next tree.
    let (a_row, _) = drawn_at(cx, handle, &view, "a.txt");
    std::fs::remove_file(dir.path().join("a.txt")).unwrap();
    let _click = click_when_the_tree_swaps(cx, handle, &view, a_row);
    state(cx, &view).update(cx, AppState::refresh);
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), under(dir.path(), &["b.txt"]));
    assert_eq!(current(cx, &view), None);
    assert_eq!(read(cx, &view, AppState::selected_count), 0);
}

/// Both added before either scan lands. Which lands first is the
/// scheduler's to choose; the order the scans land in is forced in
/// `state/scan.rs`'s unit test.
#[gpui_kit::test]
fn a_second_folder_added_before_the_first_lands_keeps_its_place(cx: &mut TestAppContext) {
    let first = folder(&[("deep/f.txt", ""), ("from-first.txt", "1")]);
    let second = folder(&[("from-second.txt", "2")]);
    let (handle, view) = open_app(cx, Launch::Empty);

    let state = state(cx, &view);
    state.update(cx, |state, cx| {
        state.open_paths(vec![first.path().to_owned()], Vec::new(), cx);
    });
    state.update(cx, |state, cx| {
        state.open_paths(vec![second.path().to_owned()], Vec::new(), cx);
    });
    cx.run_until_parked();

    // Adding never replaces: both show, in the order added.
    let mut rows = under(first.path(), &["deep", "from-first.txt"]);
    rows.extend(under(second.path(), &["from-second.txt"]));
    assert_eq!(rows_now(cx, handle), rows);
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("Strikebrain Viewer"));
    assert_eq!(
        read(cx, &view, |state| state.status().clone()),
        ScanStatus::Idle
    );
}

#[gpui_kit::test]
fn a_query_typed_during_the_first_scan_applies_when_the_tree_lands(cx: &mut TestAppContext) {
    let dir = folder(&[("main.rs", ""), ("notes.md", "")]);
    let (handle, view) = open_app(cx, Launch::Empty);
    let (seen, _log) = log_rows(cx, &view);

    // The scan has not run: the filter is on screen, the tree is not.
    state(cx, &view).update(cx, |state, cx| {
        state.open_paths(vec![dir.path().to_owned()], Vec::new(), cx);
    });
    cx.update_window(handle, |_, window, cx| {
        window.click("filter", cx);
        window.input("main", cx);
    })
    .unwrap();
    assert!(read(cx, &view, |state| state.explorer().is_none()));
    cx.run_until_parked();

    let rows = under(dir.path(), &["main.rs"]);
    assert_eq!(rows_now(cx, handle), rows);
    assert_eq!(filter_value(cx, handle), "main");
    // The tree never showed unfiltered.
    assert!(seen.borrow().iter().all(|seen| *seen == rows));
}

#[gpui_kit::test]
fn a_refresh_while_a_filter_job_is_pending_shows_the_new_tree_filtered(cx: &mut TestAppContext) {
    let dir = folder(&[("main.rs", ""), ("notes.md", "")]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let (seen, _log) = log_rows(cx, &view);
    write(dir.path(), "main2.rs", "");

    // A filter job and a rescan, both waiting.
    cx.update_window(handle, |_, window, cx| {
        window.click("filter", cx);
        window.input("main", cx);
        window.dispatch_action(Box::new(Refresh), cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &["main.rs", "main2.rs"])
    );
    assert_eq!(filter_value(cx, handle), "main");
    // The new tree never showed unfiltered.
    for rows in seen.borrow().iter() {
        assert!(!(rows.contains(&"main2.rs".to_owned()) && rows.contains(&"notes.md".to_owned())));
    }
}

#[gpui_kit::test]
fn show_ignored_clicked_twice_before_a_scan_lands_shows_them_hidden(cx: &mut TestAppContext) {
    let dir = folder(&[
        (".gitignore", "secret.txt\n"),
        ("secret.txt", "s"),
        ("keep.txt", "k"),
    ]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let (seen, _log) = log_rows(cx, &view);

    cx.update_window(handle, |_, window, cx| {
        window.dispatch_action(Box::new(ToggleShowIgnored), cx);
        window.dispatch_action(Box::new(ToggleShowIgnored), cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert!(!read(cx, &view, AppState::shows_ignored));
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &[".gitignore", "keep.txt"])
    );
    assert!(
        seen.borrow()
            .iter()
            .flatten()
            .all(|name| name != "secret.txt")
    );
    assert_eq!(
        read(cx, &view, |state| state.status().clone()),
        ScanStatus::Idle
    );
}

/// A refresh that starts the moment an opened tree lands, while its sizes
/// are still to be read: the old read never lands, and the Size column
/// shows the refreshed tree's sizes.
#[gpui_kit::test]
fn sizes_read_for_a_replaced_tree_never_land(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let (handle, view) = open_app(cx, Launch::Empty);
    let state = state(cx, &view);
    let armed = Rc::new(Cell::new(true));
    let fired = Rc::clone(&armed);
    let path = dir.path().to_owned();
    let _refresh = cx.update(|cx| {
        cx.observe(&state, move |state, cx| {
            let landed = state
                .read(cx)
                .explorer()
                .is_some_and(|explorer| !explorer.tree().has_sizes());
            if fired.get() && landed {
                fired.set(false);
                write(&path, "a.txt", "abcd");
                state.update(cx, AppState::refresh);
            }
        })
    });

    state.update(cx, |state, cx| {
        state.open_paths(vec![dir.path().to_owned()], Vec::new(), cx);
    });
    cx.run_until_parked();

    assert!(!armed.get(), "the opened tree landed before its sizes");
    let a = node(cx, &view, "a.txt");
    assert_eq!(
        read(cx, &view, |state| state.explorer().unwrap().tree().size(a)),
        Some(4)
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.within(row(a)).find("size").visible());
    })
    .unwrap();
}

/// The scan's progress bar shows once a scan has run for 200 ms. The test
/// moves the simulated clock past 200 ms before any task runs, runs the
/// expired timer alone, then the foreground tasks in the order they were
/// spawned: the slow mark comes before the task that starts the scan job.
#[gpui_kit::test]
fn a_scan_slower_than_200_ms_shows_the_progress_bar_until_it_lands(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let (handle, view) = open_app(cx, Launch::Empty);
    let status = |cx: &mut TestAppContext| read(cx, &view, |state| state.status().clone());

    state(cx, &view).update(cx, |state, cx| {
        state.open_paths(vec![dir.path().to_owned()], Vec::new(), cx);
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("scan-progress").is_none());
    })
    .unwrap();

    cx.dispatcher
        .scheduler()
        .clock()
        .advance(Duration::from_millis(200));
    while cx.dispatcher.tick(true) {}
    while status(cx) == (ScanStatus::Scanning { slow: false }) && cx.dispatcher.tick(false) {}

    assert_eq!(status(cx), ScanStatus::Scanning { slow: true });
    assert!(read(cx, &view, |state| state.explorer().is_none()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("scan-progress").visible());
    })
    .unwrap();

    cx.run_until_parked();
    assert_eq!(status(cx), ScanStatus::Idle);
    assert_eq!(rows_now(cx, handle), under(dir.path(), &["a.txt"]));
    cx.update_window(handle, |_, window, _| {
        assert!(window.try_find("scan-progress").is_none());
    })
    .unwrap();
}

/// Without the 200 ms, the scan lands with no bar ever shown.
#[gpui_kit::test]
fn a_fast_scan_never_shows_the_progress_bar(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a")]);
    let (handle, view) = open_app(cx, Launch::Empty);
    let state = state(cx, &view);
    let slow_seen = Rc::new(Cell::new(false));
    let seen = Rc::clone(&slow_seen);
    let _watch = cx.update(|cx| {
        cx.observe(&state, move |state, cx| {
            if state.read(cx).status() == (ScanStatus::Scanning { slow: true }) {
                seen.set(true);
            }
        })
    });

    state.update(cx, |state, cx| {
        state.open_paths(vec![dir.path().to_owned()], Vec::new(), cx);
    });
    cx.run_until_parked();

    assert!(!slow_seen.get());
    assert_eq!(rows_now(cx, handle), under(dir.path(), &["a.txt"]));
}

/// A Size sort chosen before the sizes are read: when they land the rows
/// reorder, and the focus row stays on screen at its new place.
#[gpui_kit::test]
fn sizes_landing_under_a_size_sort_keep_the_focus_row_on_screen(cx: &mut TestAppContext) {
    // f00 is the largest: first by name, last by size.
    let mut owned = vec![("f00.txt".to_owned(), "x".repeat(100))];
    owned.extend((1..60).map(|i| (format!("f{i:02}.txt"), "x".repeat(i))));
    let files: Vec<(&str, &str)> = owned
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let dir = folder(&files);
    let (handle, view) = open_app(cx, Launch::Empty);
    let state = state(cx, &view);
    let armed = Rc::new(Cell::new(true));
    let fired = Rc::clone(&armed);
    let _sort = cx.update(|cx| {
        cx.observe(&state, move |state, cx| {
            let landed = state
                .read(cx)
                .explorer()
                .is_some_and(|explorer| !explorer.tree().has_sizes());
            if fired.get() && landed {
                fired.set(false);
                // Focus on f00, the first row in the folder by name.
                cx.update_window(handle, |_, window, cx| window.press("down", cx))
                    .expect("press in the test window");
                state.update(cx, |state, cx| state.sort_by(SortKey::Size, cx));
            }
        })
    });

    state.update(cx, |state, cx| {
        state.open_paths(vec![dir.path().to_owned()], Vec::new(), cx);
    });
    cx.run_until_parked();

    assert!(!armed.get(), "the tree landed before its sizes");
    let f00 = node(cx, &view, "f00.txt");
    assert_eq!(crate::focus(cx, &view), Some(f00));
    assert_eq!(
        read(cx, &view, |state| state
            .explorer()
            .unwrap()
            .rows()
            .last()
            .copied()),
        Some(f00)
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(row(f00)).is_some_and(|row| row.visible()));
    })
    .unwrap();
}
