//! The milestone's exit condition: flicking through 1,000 mixed files
//! keeps the stage's caches within their budgets and the process's memory
//! under a fixed ceiling.

use gpui_kit::{
    AnyWindowHandle, AppContext as _, InputEvent as _, KeyDownEvent, KeyUpEvent, Keystroke,
    TestAppContext,
};
use viewer::Launch;

use crate::stage::{jpeg, png};
use crate::timing::{megabytes, resident_mb};
use crate::{click, is_focused, open_app, press, write};

/// How much the process may grow while flicking through the 1,000 files:
/// 64 MB. The stage keeps 8 decoded images of 192 KB and at most 32 MB of
/// text (here about 4 MB); holding every image it showed would take some
/// 105 MB, and every text another 4 MB.
const CEILING_MB: f64 = 64.;

/// → on the stage, as the platform delivers it, and every task it starts.
fn next(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    let keystroke = Keystroke::parse("right").expect("a valid keystroke");
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_event(
            KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    })
    .expect("press a key");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
        .expect("draw a frame");
}

#[gpui_kit::test]
#[expect(
    clippy::print_stderr,
    reason = "the test reports the memory it measured"
)]
fn flicking_through_a_thousand_mixed_files_stays_under_a_memory_ceiling(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().expect("a temporary folder");
    // Each picture is encoded once and written many times: every file is
    // decoded on its own all the same.
    let (picture, photo) = (png(256, 192), jpeg(256, 192));
    let text: String = "A line of text in a file the stage reads whole.\n".repeat(400);
    // Short: the Kit highlights code under 64 KB on the UI thread, about
    // 2.6 ms a KB, which would time syntect rather than the stage.
    let code: String = "fn step(value: u64) -> u64 { value + 1 }\n".repeat(20);
    for ix in 0..1000 {
        let name = format!("{ix:04}");
        match ix % 10 {
            0..=3 => write(dir.path(), &format!("{name}.png"), &picture),
            4 | 5 => write(dir.path(), &format!("{name}.jpg"), &photo),
            6 => write(dir.path(), &format!("{name}.md"), &text),
            7 => write(dir.path(), &format!("{name}.rs"), &code),
            8 => write(dir.path(), &format!("{name}.txt"), &text),
            _ => write(dir.path(), &format!("{name}.bin"), b"\0\x01\x02"),
        }
    }
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    click(cx, handle, "select-all");
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));
    // Warm up: the first files of each kind make the views and caches.
    for _ in 0..10 {
        next(cx, handle);
    }
    let before = resident_mb();
    let started = std::time::Instant::now();

    let mut most_images = 0;
    let mut most_text = 0;
    for _ in 10..999 {
        next(cx, handle);
        let probe = cx.update(|cx| view.read(cx).stage_probe(cx));
        assert!(probe.images() <= 8, "{probe:?}");
        assert!(probe.text_bytes() <= 32 * 1024 * 1024, "{probe:?}");
        most_images = most_images.max(probe.images());
        most_text = most_text.max(probe.text_bytes());
    }
    eprintln!("989 presses in {:.2?}", started.elapsed());
    let position = cx.update(|cx| view.read(cx).state().read(cx).position());
    assert_eq!(position, Some((1000, 1000)));
    assert_eq!(most_images, 8, "the image cache fills and stays full");
    // 200 texts of about 19 KB and 100 of 1 KB fit in 32 MB: all stay.
    assert!(most_text > 3 * 1024 * 1024);

    if let (Some(before), Some(after)) = (before, resident_mb()) {
        eprintln!(
            "1,000 files: {before:.0} MB before, {after:.0} MB after; \
             at most {most_images} images and {:.1} MB of text cached",
            megabytes(most_text as u64)
        );
        assert!(
            after - before < CEILING_MB,
            "the process grew {:.0} MB, from {before:.0} to {after:.0} MB",
            after - before
        );
    }
}
