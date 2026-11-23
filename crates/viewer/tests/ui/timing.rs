//! Timing probe over a synthetic million-node tree: from a keystroke in the
//! filter to the updated rows on screen, on the app's side (the event, the
//! filter job, applying it, rebuilding the rows and drawing a frame).
//! Ignored by default; the numbers only mean something in release mode:
//!
//! `cargo test --release -p viewer --test ui -- --ignored --nocapture timing`
//!
//! The test executor runs the background job on this thread, so each time
//! is the whole cost of one keystroke, not the latency of a thread hop.

use std::sync::Arc;
use std::time::{Duration, Instant};

use explorer::Tree;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use viewer::{AppView, Launch};

use crate::{click, open_app, read};

/// 10 top folders, 100 folders in each, 1,000 files in each of those:
/// 1,001,011 nodes.
fn million() -> Launch {
    let mut owned = Vec::with_capacity(1_000_000);
    for top in 0..10 {
        for mid in 0..100 {
            for file in 0..1000_u64 {
                owned.push((format!("Top {top}/dir{mid}/file{file}.txt"), file));
            }
        }
    }
    let paths: Vec<(&str, u64)> = owned.iter().map(|(p, s)| (p.as_str(), *s)).collect();
    Launch::Tree(Arc::new(
        Tree::from_paths_with_sizes(&paths).expect("the paths are valid and distinct"),
    ))
}

fn rows(cx: &mut TestAppContext, view: &Entity<AppView>) -> usize {
    read(cx, view, |state| {
        state.explorer().map_or(0, |e| e.rows().len())
    })
}

/// Runs `act`, lets every task finish and draws a frame: the time until
/// the result is on screen.
fn timed(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    act: impl FnOnce(&mut TestAppContext),
) -> Duration {
    let start = Instant::now();
    act(cx);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .expect("draw a frame");
    start.elapsed()
}

#[gpui_kit::test]
#[ignore = "timing probe; run in release with --ignored --nocapture"]
#[expect(clippy::print_stderr, reason = "the probe reports its timings")]
fn timing_a_keystroke_over_a_million_names(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, million());
    cx.update_window(handle, |_, window, cx| window.click("filter", cx))
        .unwrap();

    let mut worst = Duration::ZERO;
    let mut type_key = |cx: &mut TestAppContext, key: &str| {
        let took = timed(cx, handle, |cx| {
            cx.update_window(handle, |_, window, cx| {
                if key == "backspace" {
                    window.press(key, cx);
                } else {
                    window.input(key, cx);
                }
            })
            .unwrap();
        });
        worst = worst.max(took);
        let shown = rows(cx, &view);
        eprintln!("keystroke {key:>9}: {took:>10.2?}  ({shown} rows)");
    };
    for key in ["f", "i", "l", "e", "1", "2", "3"] {
        type_key(cx, key);
    }
    for _ in 0..7 {
        type_key(cx, "backspace");
    }
    eprintln!("worst keystroke: {worst:.2?} (budget 16 ms)");

    let took = timed(cx, handle, |cx| click(cx, handle, "expand-all"));
    eprintln!("expand all: {took:.2?} ({} rows)", rows(cx, &view));
    let took = timed(cx, handle, |cx| click(cx, handle, "header-size"));
    eprintln!("sort by size over every row: {took:.2?}");
    let took = timed(cx, handle, |cx| click(cx, handle, "header-size"));
    eprintln!("flip to largest first: {took:.2?}");
    let took = timed(cx, handle, |cx| click(cx, handle, "select-all"));
    eprintln!("select all: {took:.2?}");
    let took = timed(cx, handle, |cx| click(cx, handle, "collapse-all"));
    eprintln!("collapse all: {took:.2?} ({} rows)", rows(cx, &view));
}
