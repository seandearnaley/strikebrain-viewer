//! Live refresh against real folders and real file-system events. These
//! tests let the scheduler park, so the watcher's thread can wake the app,
//! and wait in real time with generous limits: events take some
//! milliseconds to arrive, then the app gathers them for 150 ms.

use std::path::Path;
use std::time::{Duration, Instant};

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, Point, TestAppContext, WindowBounds,
    WindowOptions, px, size,
};
use viewer::keys::SelectAllFiles;
use viewer::{AppState, AppView, Launch, ScanStatus, Session};

use crate::races::click_when_the_tree_swaps;
use crate::stage::code_view;
use crate::{folder, node, not_found, read, row, write};

/// Opens the app over `dir`, following it on disk, in a 1000 x 700 window.
fn open_watching(cx: &mut TestAppContext, dir: &Path) -> (AnyWindowHandle, Entity<AppView>) {
    open_watching_all(cx, &[dir])
}

/// Opens the app over `dirs`, following each on disk.
fn open_watching_all(
    cx: &mut TestAppContext,
    dirs: &[&Path],
) -> (AnyWindowHandle, Entity<AppView>) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_kit::init(cx);
        viewer::init(cx);
    });
    let launch = Launch::Folders(dirs.iter().map(|dir| dir.to_path_buf()).collect());
    let opened = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Point::default(),
                    size(px(1000.), px(700.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let session = Session {
                    watch: true,
                    ..Session::default()
                };
                cx.new(|cx| AppView::with_session(launch, session, window, cx))
            },
        )
        .expect("open test window")
    });
    cx.run_until_parked();
    opened
}

/// Waits, in real time, until `done` holds; fails after ten seconds.
async fn until(
    cx: &mut TestAppContext,
    what: &str,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        cx.background_executor
            .timer(Duration::from_millis(20))
            .await;
    }
}

/// Lets `duration` of real time pass, with the app running.
async fn pause(cx: &mut TestAppContext, duration: Duration) {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        cx.background_executor
            .timer(Duration::from_millis(20))
            .await;
        cx.run_until_parked();
    }
}

/// The tree's paths, every node below each folder, inside its folder.
fn paths(cx: &mut TestAppContext, view: &Entity<AppView>) -> Vec<String> {
    read(cx, view, |state| {
        let tree = state.explorer().expect("a tree is on screen").tree();
        tree.ids()
            .filter_map(|id| {
                let folder = tree.folder_of(id).filter(|&folder| folder != id)?;
                Some(tree.path_from(folder, id))
            })
            .collect()
    })
}

fn has(cx: &mut TestAppContext, view: &Entity<AppView>, path: &str) -> bool {
    paths(cx, view).iter().any(|each| each == path)
}

fn current(cx: &mut TestAppContext, view: &Entity<AppView>) -> Option<String> {
    read(cx, view, |state| {
        let tree = state.explorer()?.tree();
        let current = state.current()?;
        Some(tree.path_from(tree.folder_of(current)?, current))
    })
}

/// The text the code view holds.
fn code(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find("stage-code")
            .map(|_| code_view(window).value().unwrap_or_default().to_owned())
    })
    .expect("read the stage")
}

/// Waits until no scan runs and a quiet spell shows nothing more coming.
async fn settled(cx: &mut TestAppContext, view: &Entity<AppView>) -> u64 {
    until(cx, "the scans settle", |cx| {
        read(cx, view, |state| state.status() == ScanStatus::Idle)
    })
    .await;
    loop {
        let scans = read(cx, view, AppState::scans);
        pause(cx, Duration::from_millis(400)).await;
        if read(cx, view, AppState::scans) == scans {
            return scans;
        }
    }
}

#[gpui_kit::test]
async fn files_created_deleted_renamed_and_changed_on_disk_show(cx: &mut TestAppContext) {
    let dir = folder(&[
        ("a.txt", "first a"),
        ("b.txt", "first b"),
        ("c.txt", "first c"),
        ("sub/inner.txt", "inner"),
    ]);
    let (handle, view) = open_watching(cx, dir.path());
    settled(cx, &view).await;
    // Every file checked, b.txt on the stage.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(SelectAllFiles), cx);
    })
    .unwrap();
    let b = node(cx, &view, "b.txt");
    cx.update_window(handle, |_, window, cx| window.click(row(b), cx))
        .unwrap();
    until(cx, "b.txt is on the stage", |cx| {
        code(cx, handle).as_deref() == Some("first b")
    })
    .await;

    write(dir.path(), "new.txt", "new");
    until(cx, "a new file shows", |cx| has(cx, &view, "new.txt")).await;

    std::fs::remove_file(dir.path().join("a.txt")).unwrap();
    until(cx, "a deleted file goes", |cx| !has(cx, &view, "a.txt")).await;

    std::fs::rename(
        dir.path().join("sub/inner.txt"),
        dir.path().join("sub/moved.txt"),
    )
    .unwrap();
    until(cx, "a renamed file shows under its new name", |cx| {
        has(cx, &view, "sub/moved.txt") && !has(cx, &view, "sub/inner.txt")
    })
    .await;

    // The file on the stage changes: the stage reads it again.
    write(dir.path(), "b.txt", "second b, longer");
    until(cx, "the stage shows the new text", |cx| {
        code(cx, handle).as_deref() == Some("second b, longer")
    })
    .await;

    // The file on the stage goes: the stage moves to its neighbour.
    std::fs::remove_file(dir.path().join("b.txt")).unwrap();
    until(cx, "the stage moves to the next checked file", |cx| {
        current(cx, &view).as_deref() == Some("c.txt")
    })
    .await;
    until(cx, "the stage shows it", |cx| {
        code(cx, handle).as_deref() == Some("first c")
    })
    .await;
    // Checks carried across every rescan by path; with every file checked,
    // the files that appeared are checked too.
    assert_eq!(
        crate::checked(cx, &view),
        ["sub/moved.txt", "c.txt", "new.txt"]
    );
}

#[gpui_kit::test]
async fn a_change_rescans_only_the_folder_it_is_in(cx: &mut TestAppContext) {
    let (one, two) = (folder(&[("a.txt", "a")]), folder(&[("b.txt", "b")]));
    let (_, view) = open_watching_all(cx, &[one.path(), two.path()]);
    // One scan per folder, and maybe a rescan if a watcher hears the
    // fixture's own writes: what counts is what follows.
    let scans = settled(cx, &view).await;
    assert!(scans >= 2, "one scan per folder");

    write(two.path(), "new.txt", "n");
    until(cx, "the new file shows", |cx| has(cx, &view, "new.txt")).await;
    assert_eq!(settled(cx, &view).await, scans + 1, "one folder rescanned");
}

#[gpui_kit::test]
async fn a_file_opened_on_its_own_is_followed_alone(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("b.txt", "b")]);
    let file = dir.path().join("b.txt");
    let (_, view) = open_watching(cx, &file);
    let scans = settled(cx, &view).await;

    // Its folder is not followed: a change beside the file reads nothing.
    write(dir.path(), "a.txt", "a again");
    pause(cx, Duration::from_millis(600)).await;
    assert_eq!(read(cx, &view, AppState::scans), scans, "no rescan");

    // The file changes: it is read again, once.
    write(dir.path(), "b.txt", "b again");
    until(cx, "the file is read again", |cx| {
        read(cx, &view, AppState::scans) > scans
    })
    .await;
    assert_eq!(settled(cx, &view).await, scans + 1, "one rescan");

    // Deleted: its row is unreadable, with the system's message.
    std::fs::remove_file(&file).unwrap();
    let tooltip = |cx: &mut TestAppContext| {
        read(cx, &view, |state| {
            let top = state.explorer()?.tree().folders().next()?;
            state.folder_tooltip(top)
        })
    };
    until(cx, "the row is unreadable", |cx| {
        tooltip(cx).is_some_and(|text| text.contains('\n'))
    })
    .await;
    let tooltip = tooltip(cx).unwrap();
    assert_eq!(tooltip.lines().nth(1), Some(not_found().as_str()));
}

#[gpui_kit::test]
async fn a_busy_folder_the_ignore_rules_hide_never_rescans(cx: &mut TestAppContext) {
    let dir = folder(&[
        (".gitignore", "target/\n*.log\n"),
        ("src/main.rs", "fn main() {}"),
        ("target/.keep", ""),
    ]);
    let (_, view) = open_watching(cx, dir.path());
    let scans = settled(cx, &view).await;

    // A build: 400 files written and rewritten under target/ over about a
    // second, and a log at the top.
    for round in 0..20 {
        for file in 0..10 {
            let path = format!("target/debug/deps/unit{file}.o");
            write(dir.path(), &path, format!("round {round}"));
            write(
                dir.path(),
                &format!("target/debug/build/{round}-{file}.d"),
                "d",
            );
        }
        write(dir.path(), "build.log", format!("round {round}"));
        pause(cx, Duration::from_millis(40)).await;
    }
    pause(cx, Duration::from_millis(600)).await;
    assert_eq!(read(cx, &view, AppState::scans), scans, "no rescan");
    assert!(!has(cx, &view, "target"));

    // The watcher is alive: a change it shows rescans, once.
    write(dir.path(), "src/lib.rs", "");
    until(cx, "the new file shows", |cx| has(cx, &view, "src/lib.rs")).await;
    let after = settled(cx, &view).await;
    assert_eq!(after, scans + 1, "one rescan for one change");
}

#[gpui_kit::test]
async fn rapid_changes_while_clicking_rows_never_panic(cx: &mut TestAppContext) {
    let dir = folder(&[
        ("a.txt", "a"),
        ("b.txt", "b"),
        ("c.txt", "c"),
        ("d.txt", "d"),
    ]);
    let (handle, view) = open_watching(cx, dir.path());
    settled(cx, &view).await;
    for round in 0..12 {
        // Clicks aimed at every row and checkbox of this frame, delivered
        // the moment a rescan swaps the tree, before a frame is drawn for
        // the new one; the disk changes under them.
        let spots: Vec<_> = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window
                    .find_all_by_role(gpui_kit::Role::TreeItem)
                    .iter()
                    .flat_map(|row| {
                        let check = row.bounds().origin + gpui_kit::point(px(48.), px(14.));
                        [row.bounds().center(), check]
                    })
                    .collect()
            })
            .unwrap();
        let armed: Vec<_> = spots
            .into_iter()
            .map(|spot| click_when_the_tree_swaps(cx, handle, &view, spot))
            .collect();
        let scans = read(cx, &view, AppState::scans);
        // The last row goes and comes back: a click on it lands past the
        // end of the smaller tree.
        if round % 2 == 0 {
            std::fs::remove_file(dir.path().join("d.txt")).unwrap();
        } else {
            write(dir.path(), "d.txt", "d again");
        }
        until(cx, "the rescan lands", |cx| {
            read(cx, &view, |state| {
                state.scans() > scans && state.status() == ScanStatus::Idle
            })
        })
        .await;
        drop(armed);
    }
    // Whatever landed, the state is the disk's once it settles.
    settled(cx, &view).await;
    let mut on_disk: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let mut shown = paths(cx, &view);
    shown.sort();
    assert_eq!(shown, on_disk);
}

#[gpui_kit::test]
fn a_watcher_that_cannot_start_shows_on_the_refresh_button(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    let (handle, view) = open_watching(cx, &missing);
    let error = read(cx, &view, |state| {
        state.watch_error().map(ToString::to_string)
    });
    assert_eq!(error, Some(not_found()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let refresh = window.find("refresh");
        assert_eq!(refresh.label(), Some("Refresh"));
        let description = format!("Refresh. Changes on disk are not followed: {}", not_found());
        assert_eq!(refresh.description(), Some(description.as_str()));
    })
    .unwrap();

    // Once the folder exists, Refresh opens it and follows it.
    std::fs::create_dir(&missing).unwrap();
    write(&missing, "here.txt", "");
    cx.update_window(handle, |_, window, cx| window.click("refresh", cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(read(cx, &view, |state| state.watch_error().cloned()), None);
    assert!(has(cx, &view, "here.txt"));
    // Followed again, the warning Refresh goes.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("refresh").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn changes_during_a_scan_rescan_exactly_once_after_it(cx: &mut TestAppContext) {
    // No watcher: the changes are reported by hand, deterministically.
    let dir = folder(&[("a.txt", "a")]);
    let (_, view) = crate::open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let state = crate::state(cx, &view);
    let scans = read(cx, &view, AppState::scans);

    // While nothing runs, a change rescans at once.
    state.update(cx, AppState::disk_changed);
    assert_eq!(read(cx, &view, AppState::scans), scans + 1);
    cx.run_until_parked();

    // A scan running: five changes arrive, then it lands; one more scan
    // follows, and nothing after it.
    write(dir.path(), "b.txt", "b");
    state.update(cx, AppState::refresh);
    for _ in 0..5 {
        state.update(cx, AppState::disk_changed);
    }
    assert_eq!(read(cx, &view, AppState::scans), scans + 2);
    cx.run_until_parked();
    assert_eq!(read(cx, &view, AppState::scans), scans + 3);
    assert!(has(cx, &view, "b.txt"));

    // A scan started after the changes covers them: nothing trails it.
    state.update(cx, AppState::disk_changed);
    state.update(cx, AppState::refresh);
    cx.run_until_parked();
    assert_eq!(read(cx, &view, AppState::scans), scans + 5);
}
