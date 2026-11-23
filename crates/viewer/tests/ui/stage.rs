//! The stage, against real files: which viewer each file gets, moving
//! through the gallery, the position, keys, loading and caches.

use std::fmt::Write as _;
use std::io::Cursor;
use std::path::Path;

use explorer::NodeId;
use gpui_kit::test::ElementSnapshot;
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, InputEvent as _, Role, ScrollDelta, ScrollEvent,
    TestAppContext, Window, point, px,
};
use viewer::{AppState, AppView, Launch, StageProbe};

use crate::{click, current, focus, is_focused, node, open_app, press, read, row, write};

/// What the stage shows: the kind of view and the file's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    Image(String),
    Markdown(String),
    Code(String),
    Unsupported(String),
}

/// What the stage shows once every task has run, or `None` when blank.
pub(crate) fn shown(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<Shown> {
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let label = |id: &'static str| {
            window
                .try_find(id)
                .map(|view| view.label().expect("each view names its file").to_owned())
        };
        // An image that cannot be decoded shows as unsupported inside the
        // image's place, so unsupported is asked first.
        label("stage-unsupported")
            .map(Shown::Unsupported)
            .or_else(|| label("stage-image").map(Shown::Image))
            .or_else(|| label("stage-markdown").map(Shown::Markdown))
            .or_else(|| {
                window.try_find("stage-code")?;
                let editor = code_view(window);
                Some(Shown::Code(editor.label()?.to_owned()))
            })
    })
    .expect("read the stage")
}

/// The code view's editor: the only multi-line text input in the window.
pub(crate) fn code_view(window: &Window) -> ElementSnapshot {
    let mut editors = window.find_all_by_role(Role::MultilineTextInput);
    assert_eq!(editors.len(), 1, "one code view");
    editors.remove(0)
}

/// The position under the file, if it shows.
pub(crate) fn position(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find("stage-position")
            .map(|label| label.label().expect("the position is labelled").to_owned())
    })
    .expect("read the position")
}

pub(crate) fn probe(cx: &mut TestAppContext, view: &Entity<AppView>) -> StageProbe {
    cx.update(|cx| view.read(cx).stage_probe(cx))
}

/// A PNG of `width` x `height` pixels: a gradient.
pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([shade(x, width), shade(y, height), 160, 255])
    });
    encode(
        &image::DynamicImage::ImageRgba8(image),
        image::ImageFormat::Png,
    )
}

/// A JPEG of `width` x `height` pixels: a gradient.
pub(crate) fn jpeg(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        image::Rgb([shade(x, width), shade(y, height), 90])
    });
    encode(
        &image::DynamicImage::ImageRgb8(image),
        image::ImageFormat::Jpeg,
    )
}

/// A channel from 0 to 255 across `of` pixels.
fn shade(at: u32, of: u32) -> u8 {
    u8::try_from(at * 255 / of.max(1)).expect("at < of, so at most 255")
}

/// A two-frame animated GIF, 16 x 16.
fn gif() -> Vec<u8> {
    let frames = [[200, 60, 60, 255], [60, 60, 200, 255]]
        .map(|pixel| image::Frame::new(image::RgbaImage::from_pixel(16, 16, image::Rgba(pixel))));
    let mut bytes = Vec::new();
    image::codecs::gif::GifEncoder::new(&mut bytes)
        .encode_frames(frames)
        .expect("encode the GIF");
    bytes
}

fn encode(image: &image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), format)
        .expect("encode the image");
    bytes
}

/// A folder of one file of each kind the stage tells apart. Display order:
/// big.txt, code.rs, data.bin, latin1.txt, moving.gif, notes.md, photo.jpg,
/// photo.png.
fn mixed(dir: &Path) {
    write(dir, "photo.png", png(64, 48));
    write(dir, "photo.jpg", jpeg(40, 30));
    write(dir, "moving.gif", gif());
    write(dir, "notes.md", "# Notes\n\n- one\n- two\n");
    write(dir, "code.rs", "fn main() {\n    println!(\"hi\");\n}\n");
    write(dir, "data.bin", b"\x00\x01\x02");
    write(dir, "latin1.txt", b"na\xefve\n");
    // Sparse: past the 10 MB limit without writing it.
    write(dir, "big.txt", "text head\n");
    std::fs::File::options()
        .write(true)
        .open(dir.join("big.txt"))
        .and_then(|file| file.set_len(10 * 1024 * 1024 + 1))
        .expect("grow big.txt");
}

/// Opens `dir`, checks every file and moves keyboard focus to the stage.
pub(crate) fn open_checked(
    cx: &mut TestAppContext,
    dir: &Path,
) -> (AnyWindowHandle, Entity<AppView>) {
    let (handle, view) = open_app(cx, Launch::Folder(dir.to_owned()));
    click(cx, handle, "select-all");
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));
    (handle, view)
}

pub(crate) fn show(cx: &mut TestAppContext, handle: AnyWindowHandle, id: NodeId) {
    cx.update_window(handle, |_, window, cx| window.click(row(id), cx))
        .expect("click the row");
}

/// `count` numbered lines: `before`, the number, `after`.
pub(crate) fn numbered(count: usize, before: &str, after: &str) -> String {
    let mut text = String::new();
    for line in 0..count {
        write!(text, "{before}{line}{after}").expect("a String takes any text");
    }
    text
}

#[gpui_kit::test]
fn each_file_gets_its_viewer(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    mixed(dir.path());
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));

    let expect = [
        ("photo.png", Shown::Image("photo.png".into())),
        ("photo.jpg", Shown::Image("photo.jpg".into())),
        ("moving.gif", Shown::Image("moving.gif".into())),
        ("notes.md", Shown::Markdown("notes.md".into())),
        ("code.rs", Shown::Code("code.rs".into())),
        ("latin1.txt", Shown::Code("latin1.txt".into())),
        ("data.bin", Shown::Unsupported("data.bin".into())),
        ("big.txt", Shown::Unsupported("big.txt".into())),
    ];
    for (path, kind) in expect {
        let id = node(cx, &view, path);
        show(cx, handle, id);
        assert_eq!(shown(cx, handle), Some(kind), "{path}");
    }
}

#[gpui_kit::test]
fn the_code_view_holds_the_text_and_decodes_invalid_utf8_lossily(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    mixed(dir.path());
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));

    for (path, text) in [
        ("code.rs", "fn main() {\n    println!(\"hi\");\n}\n"),
        ("latin1.txt", "na\u{fffd}ve\n"),
    ] {
        let id = node(cx, &view, path);
        show(cx, handle, id);
        assert_eq!(shown(cx, handle), Some(Shown::Code(path.into())));
        cx.update_window(handle, |_, window, _| {
            assert_eq!(code_view(window).value(), Some(text), "{path}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn an_unsupported_file_shows_its_name_and_size(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    mixed(dir.path());
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));

    for (path, size) in [("data.bin", "3 B"), ("big.txt", "10.0 MB")] {
        let id = node(cx, &view, path);
        show(cx, handle, id);
        assert_eq!(shown(cx, handle), Some(Shown::Unsupported(path.into())));
        cx.update_window(handle, |_, window, _| {
            assert_eq!(window.find("stage-file").label(), Some(path));
            assert_eq!(window.find("stage-size").label(), Some(size), "{path}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn an_image_that_cannot_be_decoded_is_unsupported(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "broken.png", "not a png at all");
    write(dir.path(), "fine.png", png(8, 8));
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));

    let broken = node(cx, &view, "broken.png");
    show(cx, handle, broken);
    assert_eq!(
        shown(cx, handle),
        Some(Shown::Unsupported("broken.png".into()))
    );
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find("stage-size").label(), Some("16 B"));
    })
    .unwrap();

    let fine = node(cx, &view, "fine.png");
    show(cx, handle, fine);
    assert_eq!(shown(cx, handle), Some(Shown::Image("fine.png".into())));
}

#[gpui_kit::test]
fn checking_a_file_shows_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    mixed(dir.path());
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));
    assert_eq!(shown(cx, handle), None);

    let notes = node(cx, &view, "notes.md");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(notes)).click("check", cx);
    })
    .unwrap();
    assert_eq!(shown(cx, handle), Some(Shown::Markdown("notes.md".into())));
}

#[gpui_kit::test]
fn left_and_right_walk_the_gallery_in_display_order_and_stop_at_the_ends(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "b/inner.rs", "inner");
    write(dir.path(), "a.md", "# a");
    write(dir.path(), "c.png", png(4, 4));
    let (handle, _) = open_checked(cx, dir.path());

    // Folders first: b/inner.rs, a.md, c.png.
    let walk = [
        Shown::Code("inner.rs".into()),
        Shown::Markdown("a.md".into()),
        Shown::Image("c.png".into()),
    ];
    assert_eq!(shown(cx, handle).as_ref(), Some(&walk[0]));
    assert_eq!(position(cx, handle).as_deref(), Some("1 / 3"));
    for (ix, expected) in walk.iter().enumerate().skip(1) {
        press(cx, handle, "right");
        assert_eq!(shown(cx, handle).as_ref(), Some(expected));
        assert_eq!(position(cx, handle), Some(format!("{} / 3", ix + 1)));
    }
    // The last file: → stays.
    press(cx, handle, "right");
    assert_eq!(shown(cx, handle).as_ref(), Some(&walk[2]));
    assert_eq!(position(cx, handle).as_deref(), Some("3 / 3"));

    for expected in walk.iter().rev().skip(1) {
        press(cx, handle, "left");
        assert_eq!(shown(cx, handle).as_ref(), Some(expected));
    }
    // The first file: ← stays.
    press(cx, handle, "left");
    assert_eq!(shown(cx, handle).as_ref(), Some(&walk[0]));
    assert_eq!(position(cx, handle).as_deref(), Some("1 / 3"));
}

#[gpui_kit::test]
fn a_file_outside_the_gallery_peeks_to_its_neighbours_without_a_position(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.rs", "b.rs", "c.rs", "d.rs"] {
        write(dir.path(), name, name);
    }
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));
    let [a, b, c, d] = ["a.rs", "b.rs", "c.rs", "d.rs"].map(|path| node(cx, &view, path));
    cx.update_window(handle, |_, window, cx| {
        window.within(row(a)).click("check", cx);
        window.within(row(d)).click("check", cx);
    })
    .unwrap();

    // b is not checked: it shows, with no position.
    show(cx, handle, b);
    assert_eq!(shown(cx, handle), Some(Shown::Code("b.rs".into())));
    assert_eq!(position(cx, handle), None);

    // → goes to the first checked file after it, ← to the last before.
    press(cx, handle, "tab");
    press(cx, handle, "right");
    assert_eq!(current(cx, &view), Some(d));
    assert_eq!(position(cx, handle).as_deref(), Some("2 / 2"));
    show(cx, handle, c);
    press(cx, handle, "tab");
    press(cx, handle, "left");
    assert_eq!(current(cx, &view), Some(a));
    assert_eq!(position(cx, handle).as_deref(), Some("1 / 2"));
}

#[gpui_kit::test]
fn a_blank_stage_draws_nothing_and_the_keys_do_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.rs", "a");
    let (handle, view) = open_app(cx, Launch::Folder(dir.path().to_owned()));
    press(cx, handle, "tab");
    for key in ["right", "left", "down", "end", "space", "enter"] {
        press(cx, handle, key);
    }
    assert_eq!(shown(cx, handle), None);
    assert_eq!(position(cx, handle), None);
    assert_eq!(current(cx, &view), None);
    assert!(!read(cx, &view, AppState::is_presenting));
    assert!(is_focused(cx, handle, "stage"));
}

#[gpui_kit::test]
fn the_vertical_keys_scroll_the_text(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let code = numbered(400, "let line_", " = 0;\n");
    let markdown = numbered(200, "Paragraph ", ", long enough to read.\n\n");
    write(dir.path(), "a.rs", &code);
    write(dir.path(), "b.md", &markdown);
    let (handle, view) = open_checked(cx, dir.path());

    for name in ["a.rs", "b.md"] {
        let scroll = |cx: &mut TestAppContext| probe(cx, &view).scroll();
        assert_eq!(scroll(cx), 0., "{name} starts at the top");
        press(cx, handle, "down");
        assert_eq!(scroll(cx), 40., "{name}: one step");
        press(cx, handle, "down");
        press(cx, handle, "up");
        assert_eq!(scroll(cx), 40., "{name}: back one step");
        press(cx, handle, "pagedown");
        let page = scroll(cx);
        assert!(page > 300., "{name}: a page from 40 px reaches {page}");
        press(cx, handle, "end");
        let end = scroll(cx);
        assert!(end > page, "{name}: the end, {end}, is past a page");
        press(cx, handle, "down");
        assert_eq!(scroll(cx), end, "{name}: nothing past the end");
        press(cx, handle, "pageup");
        assert!(scroll(cx) < end);
        press(cx, handle, "home");
        assert_eq!(scroll(cx), 0., "{name}: home");
        press(cx, handle, "right");
    }
}

#[gpui_kit::test]
fn the_stage_keys_work_while_the_code_view_holds_focus(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let code = numbered(200, "line ", "\n");
    write(dir.path(), "a.rs", &code);
    write(dir.path(), "b.rs", "b");
    let (handle, view) = open_checked(cx, dir.path());
    let (a, b) = (node(cx, &view, "a.rs"), node(cx, &view, "b.rs"));

    // A click in the code view gives it focus, for selecting and copying.
    let editor_focused = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            code_view(window).focused() == Some(true)
        })
        .unwrap()
    };
    assert!(!editor_focused(cx));
    click(cx, handle, "stage-code");
    assert!(editor_focused(cx));
    assert!(is_focused(cx, handle, "stage"));

    press(cx, handle, "down");
    assert_eq!(probe(cx, &view).scroll(), 40.);
    press(cx, handle, "right");
    assert_eq!(current(cx, &view), Some(b));
    assert_eq!(shown(cx, handle), Some(Shown::Code("b.rs".into())));
    // The view it had focus in went away with the file; the stage keeps
    // the keys.
    press(cx, handle, "left");
    assert_eq!(current(cx, &view), Some(a));

    click(cx, handle, "stage-code");
    press(cx, handle, "escape");
    assert!(is_focused(cx, handle, "tree"));
}

#[gpui_kit::test]
fn escape_and_tab_leave_the_stage(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.png", png(4, 4));
    let (handle, _) = open_checked(cx, dir.path());
    press(cx, handle, "escape");
    assert!(is_focused(cx, handle, "tree"));
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "filter"));
}

#[gpui_kit::test]
fn quick_presses_show_only_the_last_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let names: Vec<String> = (0..8).map(|ix| format!("f{ix}.rs")).collect();
    for name in &names {
        write(dir.path(), name, name);
    }
    let (handle, view) = open_checked(cx, dir.path());
    assert_eq!(shown(cx, handle), Some(Shown::Code("f0.rs".into())));
    let loads = probe(cx, &view).loads();

    // Five presses before any read can land: each replaces the read before.
    cx.update_window(handle, |_, window, cx| {
        for _ in 0..5 {
            window.press("right", cx);
        }
    })
    .unwrap();
    assert_eq!(shown(cx, handle), Some(Shown::Code("f5.rs".into())));
    assert_eq!(
        probe(cx, &view).loads(),
        loads + 1,
        "only the last read lands"
    );

    // A read that finished on the background executor but has not landed
    // yet is dropped when the next press replaces it.
    press(cx, handle, "right");
    cx.update_window(handle, |_, window, cx| window.press("right", cx))
        .unwrap();
    // Background work only: f7.rs is read, its result waits to land.
    while cx.dispatcher.tick(true) {}
    cx.update_window(handle, |_, window, cx| window.press("left", cx))
        .unwrap();
    assert_eq!(shown(cx, handle), Some(Shown::Code("f6.rs".into())));
    // f6.rs is still on the stage, unchanged: back on it, nothing is read.
    assert_eq!(probe(cx, &view).loads(), loads + 2);
}

#[gpui_kit::test]
fn the_image_cache_holds_at_most_eight_while_flicking(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for ix in 0..20 {
        write(dir.path(), &format!("{ix:02}.png"), png(16, 16));
    }
    let (handle, view) = open_checked(cx, dir.path());
    for _ in 0..19 {
        press(cx, handle, "right");
        assert!(matches!(shown(cx, handle), Some(Shown::Image(_))));
        assert!(probe(cx, &view).images() <= 8);
    }
    // The current image and its neighbour decoded ahead are both kept.
    assert_eq!(probe(cx, &view).images(), 8);
}

#[gpui_kit::test]
fn the_neighbours_of_the_file_on_the_stage_are_decoded_ahead(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.png", png(4, 4));
    write(dir.path(), "b.rs", "b");
    write(dir.path(), "c.png", png(4, 4));
    write(dir.path(), "d.png", png(4, 4));
    let (handle, view) = open_checked(cx, dir.path());
    // a.png on the stage; b.rs is its only neighbour, and not an image.
    assert_eq!(shown(cx, handle), Some(Shown::Image("a.png".into())));
    assert_eq!(probe(cx, &view).images(), 1);
    // b.rs: a.png and c.png either side.
    press(cx, handle, "right");
    assert_eq!(shown(cx, handle), Some(Shown::Code("b.rs".into())));
    assert_eq!(probe(cx, &view).images(), 2);
    // c.png, and d.png ahead.
    press(cx, handle, "right");
    assert_eq!(probe(cx, &view).images(), 3);
}

#[gpui_kit::test]
fn text_stays_in_its_cache_within_32_megabytes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    // Six files of 9 MB: three fit in 32 MB.
    let text = "x".repeat(9 * 1024 * 1024);
    for ix in 0..6 {
        write(dir.path(), &format!("{ix}.txt"), &text);
    }
    let (handle, view) = open_checked(cx, dir.path());
    for step in 0..6 {
        if step > 0 {
            press(cx, handle, "right");
        }
        assert_eq!(shown(cx, handle), Some(Shown::Code(format!("{step}.txt"))));
        let probe = probe(cx, &view);
        assert!(probe.text_bytes() <= 32 * 1024 * 1024, "{probe:?}");
        assert_eq!(probe.texts(), (step + 1).min(3));
    }
}

#[gpui_kit::test]
fn deleting_the_file_on_the_stage_then_refreshing_shows_its_neighbour(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.rs", "b.rs", "c.rs"] {
        write(dir.path(), name, name);
    }
    let (handle, _) = open_checked(cx, dir.path());
    press(cx, handle, "right");
    assert_eq!(shown(cx, handle), Some(Shown::Code("b.rs".into())));

    std::fs::remove_file(dir.path().join("b.rs")).unwrap();
    click(cx, handle, "refresh");
    assert_eq!(shown(cx, handle), Some(Shown::Code("c.rs".into())));
    assert_eq!(position(cx, handle).as_deref(), Some("2 / 2"));
}

#[gpui_kit::test]
fn a_file_changed_on_disk_is_read_again_on_refresh(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let code = numbered(200, "line ", "\n");
    write(dir.path(), "a.rs", &code);
    write(dir.path(), "b.rs", "one");
    let (handle, view) = open_checked(cx, dir.path());

    // An unchanged file keeps its place after a refresh.
    press(cx, handle, "pagedown");
    let scrolled = probe(cx, &view).scroll();
    assert!(scrolled > 0.);
    click(cx, handle, "refresh");
    assert_eq!(shown(cx, handle), Some(Shown::Code("a.rs".into())));
    assert_eq!(probe(cx, &view).scroll(), scrolled);

    // A changed one is read again. The toolbar's buttons leave keyboard
    // focus on the stage.
    assert!(is_focused(cx, handle, "stage"));
    press(cx, handle, "right");
    write(dir.path(), "b.rs", "two, longer");
    click(cx, handle, "refresh");
    assert_eq!(shown(cx, handle), Some(Shown::Code("b.rs".into())));
    cx.update_window(handle, |_, window, _| {
        assert_eq!(code_view(window).value(), Some("two, longer"));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_rescan_leaves_focus_scroll_and_the_editor_alone(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.rs", numbered(200, "line ", "\n"));
    write(dir.path(), "other.txt", "x");
    let (handle, view) = open_checked(cx, dir.path());
    let editor_focused = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            code_view(window).focused() == Some(true)
        })
        .unwrap()
    };
    click(cx, handle, "stage-code");
    assert!(editor_focused(cx), "a click gives the code view focus");
    press(cx, handle, "pagedown");
    let scrolled = probe(cx, &view).scroll();
    assert!(scrolled > 0.);
    let loads = probe(cx, &view).loads();

    // Another file changes on disk, and live refresh rescans.
    write(dir.path(), "other.txt", "changed");
    let state = cx.update(|cx| view.read(cx).state().clone());
    state.update(cx, AppState::disk_changed);
    cx.run_until_parked();
    assert_eq!(read(cx, &view, AppState::scans), 2);
    assert!(
        editor_focused(cx),
        "a rescan took focus out of the code view"
    );
    assert_eq!(probe(cx, &view).scroll(), scrolled);
    // The file on the stage is unchanged: nothing is read or rebuilt.
    assert_eq!(probe(cx, &view).loads(), loads);

    // The file on the stage changes: it is read again, and the code view
    // keeps focus, as the file and its viewer are the same.
    write(dir.path(), "a.rs", numbered(201, "line ", "\n"));
    state.update(cx, AppState::disk_changed);
    cx.run_until_parked();
    assert_eq!(probe(cx, &view).loads(), loads + 1);
    assert!(editor_focused(cx));
}

#[gpui_kit::test]
fn the_tree_follows_the_stage_into_view(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for ix in 0..60 {
        write(dir.path(), &format!("{ix:02}.rs"), "x");
    }
    let (handle, view) = open_checked(cx, dir.path());
    let last = node(cx, &view, "59.rs");
    let focus_before = focus(cx, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(row(last)).is_none_or(|row| !row.visible()));
    })
    .unwrap();

    for _ in 0..59 {
        press(cx, handle, "right");
    }
    assert_eq!(current(cx, &view), Some(last));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let row = window.find(row(last));
        assert!(row.visible());
        assert_eq!(row.selected(), Some(true), "the current row's tint follows");
    })
    .unwrap();
    // Keyboard focus in the tree stays where it was.
    assert_eq!(focus(cx, &view), focus_before);
}

#[gpui_kit::test]
fn the_wheel_scrolls_markdown_over_its_margins_too(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "long.md",
        numbered(200, "Paragraph ", ", long enough to read.\n\n"),
    );
    let (handle, view) = open_checked(cx, dir.path());
    // A stage wider than the reading column: margins either side.
    crate::drag_divider(cx, handle, 220.);
    let scroll = |cx: &mut TestAppContext| probe(cx, &view).scroll();
    let wheel = |cx: &mut TestAppContext, x: f32| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let bounds = window.find("stage-markdown").bounds();
            let at = point(bounds.left() + px(x), bounds.center().y);
            window.pointer_move(at, None, cx);
            window.dispatch_event(
                ScrollEvent {
                    position: at,
                    delta: ScrollDelta::Pixels(point(px(0.), px(-100.))),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
        cx.run_until_parked();
    };
    assert_eq!(scroll(cx), 0.);
    // In the left margin, outside the column (and the divider's grip).
    wheel(cx, 16.);
    assert_eq!(scroll(cx), 100.);
    // Over the text.
    wheel(cx, 300.);
    assert!(scroll(cx) > 100., "the column scrolls itself");
}
