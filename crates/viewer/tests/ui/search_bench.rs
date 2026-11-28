//! Opt-in benchmark against a real mounted folder. Uses the normal window,
//! search worker, preview keys and rendering path on GPUI's test platform.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{click, current, dispatch, open_app_with_session, press, read, row, type_query};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use viewer::{AppView, Launch, Session};

fn status(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str) -> Option<String> {
    cx.run_until_parked();
    cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        w.try_find(id.to_owned())
            .and_then(|node| node.label().map(str::to_owned))
    })
    .expect("read the window status")
}

fn samples(cx: &mut TestAppContext, window: AnyWindowHandle, view: &Entity<AppView>) -> Vec<u128> {
    dispatch(cx, window, viewer::keys::FocusExplorer);
    let first = read(cx, view, |state| {
        let e = state.explorer().expect("benchmark folder has opened");
        e.rows()
            .iter()
            .copied()
            .find(|id| e.tree().is_file(*id))
            .expect("benchmark folder lists images")
    });
    click(cx, window, row(first));
    let mut samples = Vec::new();
    for i in 0..124 {
        let before = current(cx, view);
        let started = Instant::now();
        press(cx, window, if i % 8 < 4 { "down" } else { "up" });
        cx.update_window(window, |_, w, cx| w.render_frame(cx))
            .expect("render the preview");
        let elapsed = started.elapsed().as_micros();
        assert_ne!(before, current(cx, view), "preview must move");
        if i >= 4 {
            samples.push(elapsed);
        }
    }
    samples.sort_unstable();
    samples
}

#[gpui_kit::test]
#[ignore = "real NAS, opt-in model download and UI timing"]
fn browsing_during_content_index(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let folder = PathBuf::from(std::env::var_os("EXPLORER_SCAN_ROOT").unwrap());
    let store = PathBuf::from(std::env::var_os("STRIKEBRAIN_VIEWER_SEARCH_DIR").unwrap());
    let (window, view) = open_app_with_session(
        cx,
        Launch::Folders(vec![folder.clone()]),
        Session {
            search_dir: Some(store.clone()),
            ..Session::default()
        },
    );
    let before = samples(cx, window, &view);
    crate::search_scope(cx, window, "Contents");
    type_query(cx, window, "metallic skull sculpture");
    press(cx, window, "enter");
    click(cx, window, "search-start");
    let started = Instant::now();
    while status(cx, window, "search-status").is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "search did not yield a result"
        );
        std::thread::yield_now();
    }
    let first = started.elapsed();
    crate::search_scope(cx, window, "File Names");
    let start_active = status(cx, window, "search-activity").expect("indexing must be active");
    let active = samples(cx, window, &view);
    let end_active = status(cx, window, "search-activity").expect("indexing must still be active");
    crate::search_scope(cx, window, "Contents");
    let final_status = loop {
        if let Some(s) = status(cx, window, "search-status")
            && s.ends_with(" indexed")
        {
            break s;
        }
        assert!(
            started.elapsed() < Duration::from_secs(240),
            "indexing incomplete"
        );
        std::thread::park_timeout(Duration::from_millis(20));
    };
    let elapsed = started.elapsed();
    assert!(
        cx.update_window(window, |_, w, _| w
            .try_find(("search-hit", 0usize))
            .is_some())
            .unwrap()
    );
    click(cx, window, ("search-hit", 0usize));
    assert!(current(cx, &view).is_some());
    crate::search_scope(cx, window, "File Names");
    let after = samples(cx, window, &view);
    cx.update_window(window, |_, w, _| w.remove_window())
        .unwrap();
    drop(view);
    cx.run_until_parked();
    let (reopened, _) = open_app_with_session(
        cx,
        Launch::Folders(vec![folder]),
        Session {
            search_dir: Some(store),
            ..Session::default()
        },
    );
    crate::search_scope(cx, reopened, "Contents");
    type_query(cx, reopened, "metallic skull sculpture");
    let restarted = Instant::now();
    press(cx, reopened, "enter");
    while status(cx, reopened, "search-status").as_deref() != Some(&final_status) {
        assert_eq!(status(cx, reopened, "search-interrupted"), None);
        assert!(
            restarted.elapsed() < Duration::from_secs(120),
            "cached search did not finish"
        );
        std::thread::yield_now();
    }
    let reopen_seconds = restarted.elapsed().as_secs_f64();
    let report = serde_json::json!({"idle_before_us":before,"indexing_us":active,"idle_after_us":after,"start_active":start_active,"end_active":end_active,"result":final_status,"first_seconds":first.as_secs_f64(),"total_seconds":elapsed.as_secs_f64(),"reopen_seconds":reopen_seconds});
    std::fs::write(
        std::env::var_os("STRIKEBRAIN_BENCH_RESULT").unwrap(),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[gpui_kit::test]
#[ignore = "downloads real Google weights and runs local semantic inference"]
fn fresh_download_finds_semantic_matches_and_previews_them(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let folder = crate::folder(&[
        (
            "one.txt",
            "A violinist performs a concerto on stage with the symphony orchestra.",
        ),
        (
            "two.txt",
            "Waves crash on a rocky coast near a lighthouse. A salty breeze blows across the beach.",
        ),
    ]);
    for (name, rgb) in [("three.png", [0, 0, 255]), ("four.png", [255, 0, 0])] {
        image::RgbImage::from_pixel(128, 128, image::Rgb(rgb))
            .save(folder.path().join(name))
            .unwrap();
    }
    let storage = tempfile::tempdir().unwrap();
    let (window, view) = open_app_with_session(
        cx,
        Launch::Folders(vec![folder.path().to_owned()]),
        Session {
            search_dir: Some(storage.path().to_owned()),
            ..Session::default()
        },
    );
    crate::search_scope(cx, window, "Contents");
    type_query(cx, window, "where can I hear the ocean surf?");
    press(cx, window, "enter");
    assert!(storage.path().read_dir().unwrap().next().is_none());
    click(cx, window, "search-start");
    let started = Instant::now();
    while status(cx, window, "search-status").as_deref() != Some("4 indexed") {
        assert_eq!(status(cx, window, "search-interrupted"), None);
        assert!(
            started.elapsed() < Duration::from_secs(180),
            "model setup or indexing stalled"
        );
        std::thread::park_timeout(Duration::from_millis(20));
    }
    click(cx, window, ("search-hit", 0usize));
    assert_eq!(
        read(cx, &view, |s| s
            .current()
            .and_then(|id| s.name(id))
            .map(str::to_owned))
        .as_deref(),
        Some("two.txt")
    );
    let first = current(cx, &view);
    press(cx, window, "down");
    assert_ne!(current(cx, &view), first);
    cx.update_window(window, |_, w, cx| {
        w.click("filter", cx);
        w.press("secondary-a", cx);
        w.input("a solid blue square", cx);
    })
    .unwrap();
    let start = Instant::now();
    loop {
        cx.run_until_parked();
        if cx
            .update_window(window, |_, w, cx| {
                w.render_frame(cx);
                w.try_find(("search-hit", 0usize)).is_some()
            })
            .unwrap()
        {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "query did not finish"
        );
    }
    click(cx, window, ("search-hit", 0usize));
    assert_eq!(
        read(cx, &view, |s| s
            .current()
            .and_then(|id| s.name(id))
            .map(str::to_owned))
        .as_deref(),
        Some("three.png")
    );
}
