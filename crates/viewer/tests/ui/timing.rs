//! Timing probes. Over a synthetic million-node tree: from a keystroke in
//! the filter to the updated rows on screen, on the app's side (the event,
//! the filter job, applying it, rebuilding the rows and drawing a frame).
//! Over the spec's 1.65 M-entry reference corpus as one flat list: how far
//! small scroll deltas move the list at the top, the middle and the end,
//! and how long a scrolling frame takes there. On the stage: the code view
//! and the Markdown view on large files, and an 8000 x 6000 photo.
//! Ignored by default; the numbers only mean something in release mode,
//! one at a time, since several report the process's memory:
//!
//! `cargo test --release -p viewer --test ui -- --ignored --nocapture --test-threads=1 timing`
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

/// `bytes` in MB, for reports.
#[expect(
    clippy::cast_precision_loss,
    reason = "a size in a report; exact to the byte below 2^52"
)]
pub(crate) fn megabytes(bytes: u64) -> f64 {
    bytes as f64 / 1_048_576.
}

/// The process's resident memory in MB, from `ps`; `None` where it cannot
/// be read.
pub(crate) fn resident_mb() -> Option<f64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let kb: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(kb / 1024.)
}

/// `lines` lines of Rust-like code, about 27 bytes each; a negative count
/// is one line of that many thousand bytes, as a minified file has.
fn generated_code(lines: i64) -> String {
    use std::fmt::Write as _;
    if lines < 0 {
        return "var a=1;".repeat(usize::try_from(-lines).expect("positive") * 125) + "\n";
    }
    let lines = usize::try_from(lines).expect("positive");
    let mut text = String::with_capacity(lines * 52);
    for line in 0..lines {
        match line % 4 {
            0 => writeln!(text, "/// Item {line}, documented."),
            1 => writeln!(text, "pub fn item_{line}(value: u64) -> u64 {{"),
            2 => writeln!(text, "    value.wrapping_mul({line}) + 0x2a // step"),
            _ => writeln!(text, "}}"),
        }
        .expect("a String takes any text");
    }
    text
}

/// Markdown of `sections` sections: a heading, two paragraphs, a list and
/// a code block each, about 600 bytes.
fn generated_markdown(sections: usize) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(sections * 620);
    for section in 0..sections {
        write!(
            text,
            "## Section {section}\n\nA paragraph of plain prose that wraps over a line or two in the \
             reading column, with *emphasis* and `code`.\n\nAnother paragraph, shorter.\n\n\
             - one\n- two\n- three\n\n```rust\nfn section_{section}() -> u32 {{ {section} }}\n```\n\n"
        )
        .expect("a String takes any text");
    }
    text
}

/// One frame per scroll event of `dy` over the element `id`.
fn time_scroll_frames(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    id: &'static str,
    dy: f32,
    frames: usize,
) -> Vec<Duration> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let position = window.find(id).bounds().center();
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
    .expect("scroll")
}

/// The code view on large files: from a click on the row to the code on
/// screen, the highlighter's background parse, scrolling frames at the top
/// and the end, and the process's memory. The test build copies the whole
/// text into an accessibility value on every frame (the Kit's editor does
/// so whenever `test-support` is compiled in or an assistive technology is
/// connected), so frame time and memory here are upper bounds.
#[gpui_kit::test]
#[ignore = "probe; run in release with --ignored --nocapture"]
#[expect(clippy::print_stderr, reason = "the probe reports its measurements")]
fn timing_the_code_view_on_large_files(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().expect("a temporary folder");
    // 1 KB first, which also loads the highlighter's syntax set; then 8 KB,
    // 27 KB, 62 KB (the largest the highlighter parses at once, on the UI
    // thread), 68 KB, then about 0.5, 5.5 and 10 MB of 27-byte lines, and
    // one 4.8 MB line.
    let sizes = [
        40_i64, 300, 1_000, 2_300, 2_500, 20_000, 200_000, 360_000, -5_000,
    ];
    for lines in sizes {
        crate::write(dir.path(), &format!("{lines:06}.rs"), generated_code(lines));
    }
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));
    for lines in sizes {
        let name = format!("{lines:06}.rs");
        let id = crate::node(cx, &view, &name);
        let bytes = std::fs::metadata(dir.path().join(&name)).map_or(0, |m| m.len());
        let before = resident_mb().unwrap_or(f64::NAN);
        // The read (on this thread under the test executor), the editor
        // taking the text and the first frame.
        let shown = timed(cx, handle, |cx| {
            cx.update_window(handle, |_, window, cx| window.click(crate::row(id), cx))
                .expect("click the row");
        });
        let on_screen = resident_mb().unwrap_or(f64::NAN);
        // The highlighter parses after a 150 ms pause, on the background
        // executor: its whole cost, here on this thread.
        let start = Instant::now();
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        let highlighted = start.elapsed();
        let after_highlight = resident_mb().unwrap_or(f64::NAN);
        let top = time_scroll_frames(cx, handle, "stage-code", -40., 120);
        press(cx, handle, "tab");
        press(cx, handle, "end");
        let end = time_scroll_frames(cx, handle, "stage-code", 40., 120);
        press(cx, handle, "escape");
        eprintln!(
            "{name}: {:.1} MB: click to first frame {shown:.2?}; highlight {highlighted:.2?}; \
             frames at the top {}; at the end {}; RSS {before:.0} -> {on_screen:.0} on screen \
             -> {after_highlight:.0} highlighted -> {:.0} MB after scrolling",
            megabytes(bytes),
            summary(top),
            summary(end),
            resident_mb().unwrap_or(f64::NAN),
        );
    }
}

/// A JPEG of `width` x `height` with grain in every pixel, so it decodes
/// as slowly as a photograph rather than a smooth gradient.
fn noisy_jpeg(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        let hash =
            (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663)).wrapping_mul(83_492_791);
        let [a, b, c, _] = hash.to_le_bytes();
        image::Rgb([a, b / 2 + 64, c / 3 + 100])
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Jpeg,
        )
        .expect("encode the photo");
    bytes
}

/// An 8000 x 6000 photo, 48 megapixels, as PNG and as JPEG: from a click
/// on its row to the decoded picture drawn, again from the cache, and the
/// process's memory. GPUI decodes at full resolution.
/// The Markdown view on large documents: from a click to the parsed
/// document on screen, and scrolling frames. The reading column lays out
/// every block on each frame, so frame time grows with the document.
#[gpui_kit::test]
#[ignore = "probe; run in release with --ignored --nocapture"]
#[expect(clippy::print_stderr, reason = "the probe reports its measurements")]
fn timing_the_markdown_view_on_large_documents(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let sizes = [20_usize, 100, 400, 1_600];
    for sections in sizes {
        crate::write(
            dir.path(),
            &format!("{sections:05}.md"),
            generated_markdown(sections),
        );
    }
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));
    for sections in sizes {
        let name = format!("{sections:05}.md");
        let id = crate::node(cx, &view, &name);
        let bytes = std::fs::metadata(dir.path().join(&name)).map_or(0, |m| m.len());
        let shown = timed(cx, handle, |cx| {
            cx.update_window(handle, |_, window, cx| window.click(crate::row(id), cx))
                .expect("click the row");
        });
        let frames = time_scroll_frames(cx, handle, "stage-markdown", -40., 60);
        eprintln!(
            "{name}: {:.0} KB, {sections} sections: click to first frame {shown:.2?}; scroll frames {}",
            megabytes(bytes) * 1024.,
            summary(frames),
        );
    }
}

#[gpui_kit::test]
#[ignore = "probe; run in release with --ignored --nocapture"]
#[expect(clippy::print_stderr, reason = "the probe reports its measurements")]
fn timing_a_large_photo(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let start = Instant::now();
    crate::write(dir.path(), "large.png", crate::stage::png(8000, 6000));
    crate::write(dir.path(), "large.jpg", noisy_jpeg(8000, 6000));
    crate::write(dir.path(), "small.txt", "x");
    eprintln!("wrote the photos in {:.2?}", start.elapsed());
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));
    let small = crate::node(cx, &view, "small.txt");
    for name in ["large.png", "large.jpg"] {
        let id = crate::node(cx, &view, name);
        let before = resident_mb();
        let shown = timed(cx, handle, |cx| {
            cx.update_window(handle, |_, window, cx| window.click(crate::row(id), cx))
                .expect("click the row");
        });
        let after = resident_mb();
        // Away and back: the decoded image comes from the cache.
        timed(cx, handle, |cx| {
            cx.update_window(handle, |_, window, cx| window.click(crate::row(small), cx))
                .expect("click the row");
        });
        let again = timed(cx, handle, |cx| {
            cx.update_window(handle, |_, window, cx| window.click(crate::row(id), cx))
                .expect("click the row");
        });
        eprintln!(
            "{name} 8000 x 6000, {:.1} MB: click to decoded and drawn {shown:.2?} \
             (again, cached: {again:.2?}); RSS {:.0} -> {:.0} MB",
            megabytes(std::fs::metadata(dir.path().join(name)).map_or(0, |m| m.len())),
            before.unwrap_or(f64::NAN),
            after.unwrap_or(f64::NAN),
        );
    }
}
