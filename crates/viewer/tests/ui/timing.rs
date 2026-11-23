//! Timing probes. Over a synthetic million-node tree: from a keystroke in
//! the filter to the updated rows on screen, on the app's side (the event,
//! the filter job, applying it, rebuilding the rows and drawing a frame).
//! Over the spec's 1.65 M-entry reference corpus as one flat list: how far
//! small scroll deltas move the list at the top, the middle and the end,
//! and how long a scrolling frame takes there.
//! Ignored by default; the numbers only mean something in release mode:
//!
//! `cargo test --release -p viewer --test ui -- --ignored --nocapture timing`
//!
//! The test executor runs the background job on this thread, so each time
//! is the whole cost of one keystroke, not the latency of a thread hop.

use std::sync::Arc;
use std::time::{Duration, Instant};

use explorer::Tree;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, InputEvent as _, Role, ScrollDelta, ScrollEvent,
    TestAppContext, point, px,
};
use viewer::{AppView, Launch};

use crate::{click, open_app, press, read};

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

/// The spec's reference corpus: 1,650,231 entries. A flat folder makes
/// every entry a row: f0000000 to f1650230.
const CORPUS: usize = 1_650_231;
/// The height of one row, rule included.
const ROW: f64 = 29.;

/// Where the list is scrolled, read from the frame: the first row drawn,
/// whose name is its index, and how far its top sits above the list's.
fn scroll_top(cx: &mut TestAppContext, handle: AnyWindowHandle) -> f64 {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let list = window.find("tree").bounds();
        let first = window
            .find_all_by_role(Role::TreeItem)
            .into_iter()
            .find(gpui_kit::test::ElementSnapshot::visible)
            .expect("a row on screen");
        let ix: f64 = first
            .label()
            .and_then(|label| label.strip_prefix('f'))
            .expect("rows are named f and their index")
            .parse::<u32>()
            .expect("rows are named by index")
            .into();
        ix * ROW - f64::from(f32::from(first.bounds().top() - list.top()))
    })
    .expect("read the frame")
}

/// Scrolls the tree by `dy` pixels (positive is toward the top) and draws.
fn scroll_by(cx: &mut TestAppContext, handle: AnyWindowHandle, dy: f32) {
    cx.update_window(handle, |_, window, cx| {
        window.scroll("tree", ScrollDelta::Pixels(point(px(0.), px(dy))), cx);
    })
    .expect("scroll the tree");
}

/// Draws `frames` frames, each after one scroll event of `dy`, as the
/// platform delivers them: the event, then a draw that rebuilds only what
/// it made dirty. Returns each frame's time.
fn time_frames(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    dy: f32,
    frames: usize,
) -> Vec<Duration> {
    cx.update_window(handle, |_, window, cx| {
        let position = window.find("tree").bounds().center();
        (0..frames)
            .map(|_| {
                let start = Instant::now();
                window.dispatch_event(
                    ScrollEvent {
                        position,
                        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                window.draw(cx).clear(cx);
                start.elapsed()
            })
            .collect()
    })
    .expect("scroll the tree")
}

fn summary(mut times: Vec<Duration>) -> String {
    times.sort();
    let at = |q: f64| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "an index into a short list of samples"
        )]
        let ix = ((times.len() - 1) as f64 * q).round() as usize;
        times[ix]
    };
    format!(
        "median {:.2?}, p95 {:.2?}, max {:.2?}",
        at(0.5),
        at(0.95),
        at(1.)
    )
}

/// Where the scroll probe measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Top,
    Middle,
    End,
}

impl Place {
    fn go(self, cx: &mut TestAppContext, handle: AnyWindowHandle) {
        match self {
            Self::Top => press(cx, handle, "home"),
            Self::Middle => {
                press(cx, handle, "home");
                // Half the corpus' height.
                scroll_by(cx, handle, -23_925_000.);
            }
            Self::End => press(cx, handle, "end"),
        }
    }
}

/// The spacing of `f32` values at `y`: GPUI keeps the scroll offset in an
/// `f32`, so a delta smaller than half of it is lost.
fn f32_step(y: f64) -> f64 {
    #[expect(clippy::cast_possible_truncation, reason = "the probe's offsets fit")]
    let y = y as f32;
    f64::from(y.next_up() - y)
}

#[gpui_kit::test]
#[ignore = "probe; run in release with --ignored --nocapture"]
#[expect(clippy::print_stderr, reason = "the probe reports its measurements")]
fn timing_scrolling_the_reference_corpus(cx: &mut TestAppContext) {
    let start = Instant::now();
    let names: Vec<String> = (0..CORPUS).map(|i| format!("f{i:07}")).collect();
    let paths: Vec<&str> = names.iter().map(String::as_str).collect();
    let (handle, view) = open_app(
        cx,
        Launch::Tree(Arc::new(
            Tree::from_paths(&paths).expect("the names are distinct"),
        )),
    );
    eprintln!(
        "{} rows on screen after {:.2?}",
        rows(cx, &view),
        start.elapsed()
    );

    // Where a delta of half a pixel, one pixel and three pixels starts to
    // be lost or rounded: the f32 step reaches 1, 2 and 4 px.
    for step in [1_u32, 2, 4] {
        let from = f64::from(step) * f64::from(1_u32 << 23);
        eprintln!(
            "f32 step {step} px from {from:.0} px: row {:.0}",
            from / ROW
        );
    }

    for place in [Place::Top, Place::Middle, Place::End] {
        place.go(cx, handle);
        let top = scroll_top(cx, handle);
        eprintln!(
            "{place:?}: scrolled {top:.1} px (row {:.0}); f32 step there {:.3} px",
            top / ROW,
            f32_step(top)
        );
        // Small deltas toward the top, then back, as a slow trackpad sends.
        for dy in [0.5_f32, 1., 3., 10.] {
            for sign in [1., -1.] {
                place.go(cx, handle);
                let before = scroll_top(cx, handle);
                scroll_by(cx, handle, sign * dy);
                let moved = before - scroll_top(cx, handle);
                eprintln!("  delta {:+5.1} px: moved {moved:+.1} px", sign * dy);
            }
        }
        place.go(cx, handle);
        // Two seconds at 60 Hz of a 12 px-per-frame scroll toward the end
        // (or the top, from the end).
        let dy = if place == Place::End { 12. } else { -12. };
        eprintln!("  frames: {}", summary(time_frames(cx, handle, dy, 120)));
    }
}
