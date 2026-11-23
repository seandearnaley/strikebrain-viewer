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
use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, HeadlessAppContext, ScrollDelta,
    WindowBounds, WindowOptions, point, px, size,
};
use viewer::{AppAssets, AppView, Launch, ScanStatus};

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
    capture(cx, handle, mode, dir, name)
}

/// Draws one frame in `mode` and writes it, running no task first: for a
/// state that the next task would end, such as a scan in progress.
fn capture(
    cx: &mut HeadlessAppContext,
    handle: AnyWindowHandle,
    mode: ThemeMode,
    dir: &Path,
    name: &str,
) -> PathBuf {
    cx.update(|cx| Theme::change(mode, None, cx));
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .expect("draw a frame");
    let image = cx
        .capture_screenshot(handle)
        .expect("the offscreen renderer must be available");
    let path = dir.join(format!("{name}.png"));
    image.save(&path).expect("write the PNG");
    path
}

/// The node at `path` in the tree on screen.
fn node(cx: &mut HeadlessAppContext, view: &Entity<AppView>, path: &str) -> ElementId {
    let id = cx.update(|cx| {
        let state = view.read(cx).state().read(cx);
        let explorer = state.explorer().expect("a tree is on screen");
        explorer.tree().find(path).expect("the path is in the tree")
    });
    ("row", id.index()).into()
}

/// A temporary folder holding `files`, each with its text.
fn folder(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary folder");
    for (path, text) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().expect("a file has a folder"))
            .expect("make the file's folder");
        std::fs::write(path, text).expect("write the file");
    }
    dir
}

/// Sets a folder's permissions: 0o000 makes it unreadable.
fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .expect("set the folder's permissions");
}

/// Starts opening `root` and stops while the scan is still running but has
/// run for 200 ms: the simulated clock moves past 200 ms before any task
/// runs, the expired timer runs alone, then the foreground tasks in spawn
/// order until the slow mark, which comes before the task that starts the
/// scan job.
fn open_slowly(cx: &mut HeadlessAppContext, view: &Entity<AppView>, root: PathBuf) {
    let state = cx.update(|cx| view.read(cx).state().clone());
    cx.update(|cx| state.update(cx, |state, cx| state.open_folder(root, cx)));
    let executor = cx.background_executor.clone();
    let dispatcher = executor
        .dispatcher()
        .as_test()
        .expect("the headless app runs on the test scheduler");
    dispatcher
        .scheduler()
        .clock()
        .advance(Duration::from_millis(200));
    while dispatcher.tick(true) {}
    let slow = |cx: &mut HeadlessAppContext| {
        cx.update(|cx| *state.read(cx).status() == ScanStatus::Scanning { slow: true })
    };
    while !slow(cx) && dispatcher.tick(false) {}
    assert!(slow(cx), "the scan shows its progress bar");
}

fn fixture() -> Launch {
    Launch::Tree(Arc::new(
        Tree::from_paths_with_sizes(FIXTURE).expect("the fixture is a valid tree"),
    ))
}

/// 300 files with sizes from bytes to megabytes.
fn long_list() -> Launch {
    let owned: Vec<(String, u64)> = (0..300_u64)
        .map(|i| (format!("frame-{i:03}.png"), 1 + i * i * i * 97))
        .collect();
    let paths: Vec<(&str, u64)> = owned.iter().map(|(p, s)| (p.as_str(), *s)).collect();
    Launch::Tree(Arc::new(
        Tree::from_paths_with_sizes(&paths).expect("the names are distinct"),
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

        // Keyboard focus on a file row: the ring, on a row that is not
        // current.
        let mut cx = context();
        let (handle, _) = open(&mut cx, fixture());
        cx.update_window(handle, |_, window, cx| {
            for key in ["end", "up", "up"] {
                window.press(key, cx);
            }
        })
        .expect("drive the tree");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("focus-file-{suffix}"),
        ));

        // The pointer over a row that is not current.
        let mut cx = context();
        let (handle, view) = open(&mut cx, fixture());
        let docs = node(&mut cx, &view, "docs");
        cx.update_window(handle, |_, window, cx| window.hover(docs, cx))
            .expect("hover a row");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("hover-{suffix}"),
        ));

        // A long list scrolled to its middle, the pointer on the overlay
        // scrollbar so it shows: it must not cover the sizes.
        let mut cx = context();
        let (handle, _) = open(&mut cx, long_list());
        cx.update_window(handle, |_, window, cx| {
            window.scroll("tree", ScrollDelta::Pixels(point(px(0.), px(-2_900.))), cx);
            let tree = window.find("tree").bounds();
            window.pointer_move(point(tree.right() - px(8.), tree.center().y), None, cx);
        })
        .expect("scroll the tree");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("long-scrolled-{suffix}"),
        ));

        // A folder scanning for more than 200 ms: the progress bar under
        // the toolbar.
        let mut cx = context();
        let (handle, view) = open(&mut cx, Launch::Empty);
        open_slowly(
            &mut cx,
            &view,
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture"),
        );
        written.push(capture(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("scanning-{suffix}"),
        ));

        // Show ignored on: the ignored file listed, the eye pressed.
        let ignoring = folder(&[
            (".gitignore", "build.log\n"),
            ("build.log", "x"),
            ("notes.md", "x"),
        ]);
        let mut cx = context();
        let (handle, _) = open(&mut cx, Launch::Folder(ignoring.path().to_owned()));
        click(&mut cx, handle, "show-ignored");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("show-ignored-{suffix}"),
        ));

        // An unreadable folder and a symlink among ordinary rows.
        let special = folder(&[
            ("locked/secret.txt", "x"),
            ("real/inside.txt", "x"),
            ("a.txt", "x"),
        ]);
        std::os::unix::fs::symlink(special.path().join("real"), special.path().join("link"))
            .expect("make a symlink");
        chmod(&special.path().join("locked"), 0o000);
        let mut cx = context();
        let (handle, _) = open(&mut cx, Launch::Folder(special.path().to_owned()));
        chmod(&special.path().join("locked"), 0o755);
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("special-rows-{suffix}"),
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
