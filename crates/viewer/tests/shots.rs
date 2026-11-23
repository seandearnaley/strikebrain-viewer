//! `just shot`: renders key states of the real app with GPUI's offscreen
//! renderer (Metal on macOS) and writes them to `target/shots/*.png`, so a
//! person or an agent can look at the interface without a window.
//!
//! Not a test suite: `cargo test` and nextest skip it (`test = false`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use explorer::Tree;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, HeadlessAppContext, WindowBounds,
    WindowOptions, point, px, size,
};
use viewer::{AppAssets, AppView, Launch};

/// A small project with nested folders, sizes and a long name.
const FIXTURE: &[(&str, u64)] = &[
    ("assets/", 0),
    ("assets/icons/", 0),
    ("assets/icons/check.svg", 512),
    ("assets/icons/folder.svg", 734),
    ("assets/logo.png", 48_211),
    (
        "assets/photo-of-the-team-at-the-offsite-in-the-mountains.jpg",
        3_482_112,
    ),
    ("docs/", 0),
    ("docs/design.md", 18_402),
    ("docs/notes.md", 2_210),
    ("src/", 0),
    ("src/lib.rs", 1_024),
    ("src/main.rs", 733),
    ("Cargo.toml", 412),
    ("README.md", 5_120),
    ("file2.txt", 10),
    ("file10.txt", 10),
];

fn context() -> HeadlessAppContext {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(AppAssets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        viewer::init(cx);
    });
    cx
}

fn open(cx: &mut HeadlessAppContext, launch: Launch) -> (AnyWindowHandle, Entity<AppView>) {
    let opened = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: size(px(900.), px(560.)),
                    })),
                    focus: true,
                    show: false,
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| AppView::new(launch, window, cx)),
            )
        })
        .expect("open the window");
    cx.run_until_parked();
    cx.update_window(opened.0, |_, window, cx| window.render_frame(cx))
        .expect("draw the first frame");
    opened
}

/// Draws a frame in `mode` and writes it to `dir/name.png`.
///
/// The app follows the window's appearance, and the headless window always
/// reports Light, so the mode is applied again just before drawing.
fn save(
    cx: &mut HeadlessAppContext,
    handle: AnyWindowHandle,
    mode: ThemeMode,
    dir: &Path,
    name: &str,
) -> PathBuf {
    cx.run_until_parked();
    cx.update(|cx| Theme::change(mode, None, cx));
    // Let measured layout and springs (the kind control's pill) settle on
    // the simulated clock.
    for _ in 0..3 {
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .expect("draw a frame");
        cx.advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
    }
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .expect("draw a frame");
    let image = cx
        .capture_screenshot(handle)
        .expect("the offscreen renderer must be available");
    let path = dir.join(format!("{name}.png"));
    image.save(&path).expect("write the PNG");
    path
}

fn fixture() -> Launch {
    Launch::Tree(Arc::new(
        Tree::from_paths_with_sizes(FIXTURE).expect("the fixture is a valid tree"),
    ))
}

/// Drags the divider to `x`.
fn drag_divider(cx: &mut HeadlessAppContext, handle: AnyWindowHandle, x: f32) {
    cx.update_window(handle, |_, window, cx| {
        let pane = window.find("explorer-pane").bounds();
        let y = pane.center().y;
        window.drag(point(pane.right(), y), point(px(x), y), cx);
    })
    .expect("drag the divider");
}

/// Clicks into the filter and types `text`.
fn type_query(cx: &mut HeadlessAppContext, handle: AnyWindowHandle, text: &str) {
    cx.update_window(handle, |_, window, cx| {
        window.click("filter", cx);
        window.input(text, cx);
    })
    .expect("type into the filter");
    cx.run_until_parked();
}

/// Clicks the element `id`.
fn click(cx: &mut HeadlessAppContext, handle: AnyWindowHandle, id: &'static str) {
    cx.update_window(handle, |_, window, cx| window.click(id, cx))
        .expect("click");
    cx.run_until_parked();
}

/// Drives the tree with keys so one frame shows every row state: open and
/// closed folders, mixed and full checks, the current row and the focus ring.
fn exercise(cx: &mut HeadlessAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| {
        // Open assets and assets/icons; focus ends on check.svg.
        for key in ["right", "down", "right", "down"] {
            window.press(key, cx);
        }
        // Check folder.svg and the photo: assets and icons turn mixed, and
        // the photo, checked last, is current.
        for key in ["down", "space", "down", "down", "space"] {
            window.press(key, cx);
        }
        // Check src as a folder and open it; the ring stays on src.
        for key in ["down", "down", "space", "right"] {
            window.press(key, cx);
        }
    })
    .expect("drive the tree");
}

#[expect(
    clippy::print_stdout,
    reason = "the harness reports where it wrote each shot"
)]
fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shots");
    std::fs::create_dir_all(&dir).expect("create target/shots");
    let dir = dir.canonicalize().expect("target/shots exists");
    let mut written = Vec::new();

    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        let suffix = if mode.is_dark() { "dark" } else { "light" };

        let mut cx = context();
        let (handle, _) = open(&mut cx, Launch::Empty);
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("empty-{suffix}"),
        ));

        let mut cx = context();
        let (handle, _) = open(&mut cx, fixture());
        written.push(save(&mut cx, handle, mode, &dir, &format!("tree-{suffix}")));
        exercise(&mut cx, handle);
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("tree-states-{suffix}"),
        ));

        // 480 px: the Type column joins Size.
        drag_divider(&mut cx, handle, 480.);
        written.push(save(&mut cx, handle, mode, &dir, &format!("wide-{suffix}")));

        // Sorted by size, largest first: two clicks on Size.
        click(&mut cx, handle, "header-size");
        click(&mut cx, handle, "header-size");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("sorted-size-{suffix}"),
        ));

        // Narrower than 300 px: no Size column, names truncate.
        drag_divider(&mut cx, handle, 230.);
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("narrow-{suffix}"),
        ));

        // A query that finds files inside closed folders: they show open.
        let mut cx = context();
        let (handle, _) = open(&mut cx, fixture());
        type_query(&mut cx, handle, "svg");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("filter-{suffix}"),
        ));

        // A query with no match: the tree is blank.
        type_query(&mut cx, handle, " zzz");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("filter-empty-{suffix}"),
        ));

        let mut cx = context();
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/no-such-folder");
        let (handle, _) = open(&mut cx, Launch::Folder(missing));
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("unreadable-{suffix}"),
        ));
    }

    for path in written {
        println!("wrote {}", path.display());
    }
}
