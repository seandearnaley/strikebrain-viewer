//! `just shot`: renders key states of the real app with GPUI's offscreen
//! renderer (Metal on macOS) and writes them to `target/shots/*.png`, so a
//! person or an agent can look at the interface without a window.
//!
//! Not a test suite: `cargo test` and nextest skip it (`test = false`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use explorer::{NodeId, Tree};
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, HeadlessAppContext, Pixels,
    ScrollDelta, Size, WindowBounds, WindowOptions, point, px, size,
};
use viewer::{AppAssets, AppView, Launch, ScanStatus, Session};

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
    open_sized(cx, launch, size(px(900.), px(560.)))
}

fn open_sized(
    cx: &mut HeadlessAppContext,
    launch: Launch,
    window_size: Size<Pixels>,
) -> (AnyWindowHandle, Entity<AppView>) {
    let opened = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: Default::default(),
                        size: window_size,
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

/// The row of the node at `path` in the tree on screen: from the root of a
/// tree built in memory, or inside the first folder of a tree of folders.
fn node(cx: &mut HeadlessAppContext, view: &Entity<AppView>, path: &str) -> ElementId {
    let id = cx.update(|cx| {
        let state = view.read(cx).state().read(cx);
        let tree = state.explorer().expect("a tree is on screen").tree();
        let from = if tree.is_joined() {
            tree.folders().next().expect("a folder is open")
        } else {
            NodeId::ROOT
        };
        tree.find_from(from, path).expect("the path is in the tree")
    });
    ("row", id.index()).into()
}

/// The row of the top-level folder at `path`.
fn folder_row(cx: &mut HeadlessAppContext, view: &Entity<AppView>, path: &Path) -> ElementId {
    let id = cx.update(|cx| {
        let state = view.read(cx).state().read(cx);
        let explorer = state.explorer().expect("a tree is on screen");
        let path = explorer::canonical(&path).expect("the folder exists");
        explorer.folder_at(&path).expect("the folder is open")
    });
    ("row", id.index()).into()
}

/// A folder named `name` in a temporary folder, holding `files`, each with
/// its text. The folder goes when the returned handle drops.
fn folder(name: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let root = dir.path().join(name);
    for (path, text) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a file has a folder"))
            .expect("make the file's folder");
        std::fs::write(path, text).expect("write the file");
    }
    (dir, root)
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
    cx.update(|cx| {
        state.update(cx, |state, cx| {
            state.open_paths(vec![root], Vec::new(), cx);
        });
    });
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
        cx.update(|cx| state.read(cx).status() == ScanStatus::Scanning { slow: true })
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

/// One file of each kind the stage shows, in display order: a landscape
/// photo, a portrait photo, an icon smaller than the stage, Markdown, Rust,
/// a file of long lines and a binary file; in a folder named `gallery`.
fn stage_folder() -> (tempfile::TempDir, PathBuf) {
    let (dir, root) = folder("gallery", &[]);
    std::fs::create_dir_all(&root).expect("make the gallery folder");
    let write = |name: &str, bytes: &[u8]| {
        std::fs::write(root.join(name), bytes).expect("write a stage file");
    };
    write("a-landscape.png", &photo(1600, 1000));
    write("b-portrait.jpg", &photo_jpeg(900, 1400));
    write("c-icon.png", &icon(32));
    write("d-notes.md", MARKDOWN.as_bytes());
    write("e-lib.rs", RUST.as_bytes());
    let long: String = (0..60)
        .map(|line| {
            let mut text = format!("{line:03} ");
            text.push_str(&"a long line that runs past the edge of the stage ".repeat(4));
            text.push('\n');
            text
        })
        .collect();
    write("f-long-lines.txt", long.as_bytes());
    write("g-archive.bin", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    (dir, root)
}

/// A calm test picture: a dusk gradient with soft bands and a sun.
fn picture(width: u32, height: u32) -> image::RgbImage {
    image::RgbImage::from_fn(width, height, |x, y| {
        let (fx, fy) = (x as f32 / width as f32, y as f32 / height as f32);
        let sky = [
            (0.10 + 0.55 * fy, 0.18 + 0.35 * fy, 0.42 - 0.12 * fy),
            (0.98, 0.72, 0.45),
        ];
        let (cx, cy) = (0.68_f32, 0.58_f32);
        let sun = ((fx - cx).powi(2) + ((fy - cy) * height as f32 / width as f32).powi(2)).sqrt();
        let glow = (1. - (sun / 0.12)).clamp(0., 1.);
        let hill = fy > 0.72 + 0.06 * (fx * 7.).sin();
        let (r, g, b) = if hill {
            (0.08 + 0.05 * fx, 0.13 + 0.05 * fy, 0.16)
        } else {
            let t = glow.powf(0.6);
            (
                sky[0].0 * (1. - t) + sky[1].0 * t,
                sky[0].1 * (1. - t) + sky[1].1 * t,
                sky[0].2 * (1. - t) + sky[1].2 * t,
            )
        };
        let channel = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
        image::Rgb([channel(r), channel(g), channel(b)])
    })
}

fn photo(width: u32, height: u32) -> Vec<u8> {
    encode(
        image::DynamicImage::ImageRgb8(picture(width, height)),
        image::ImageFormat::Png,
    )
}

fn photo_jpeg(width: u32, height: u32) -> Vec<u8> {
    encode(
        image::DynamicImage::ImageRgb8(picture(width, height)),
        image::ImageFormat::Jpeg,
    )
}

/// A small app icon: a rounded square with a mark in it.
fn icon(side: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(side, side, |x, y| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let half = side as f32 / 2.;
        let corner = side as f32 * 0.22;
        let (dx, dy) = (
            (fx - half).abs() - (half - corner),
            (fy - half).abs() - (half - corner),
        );
        let outside = dx.max(0.).hypot(dy.max(0.)) > corner;
        if outside {
            return image::Rgba([0, 0, 0, 0]);
        }
        let ring = ((fx - half).hypot(fy - half) - side as f32 * 0.24).abs() < side as f32 * 0.07;
        if ring {
            image::Rgba([255, 255, 255, 255])
        } else {
            image::Rgba([52, 110, 190, 255])
        }
    });
    encode(
        image::DynamicImage::ImageRgba8(image),
        image::ImageFormat::Png,
    )
}

fn encode(image: image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut bytes), format)
        .expect("encode the picture");
    bytes
}

const MARKDOWN: &str = "# Field notes

A short document to check the reading view: headings, a list, code and a table.
Lines wrap inside a measure that stays readable on a wide stage.

## Packing list

- A notebook with *squared* paper
- Two pencils, one **soft**
- The `trail-map.pdf` printed at A4

### Steps

1. Check the forecast.
2. Leave a note with the route.
3. Start before sunrise.

```rust
fn main() {
    let distance = 12.5; // kilometres
    println!(\"{distance} km\");
}
```

| Day | Distance | Climb |
| --- | ---: | ---: |
| Monday | 12.5 km | 640 m |
| Tuesday | 8.0 km | 310 m |

> The map is not the territory.
";

const RUST: &str = "//! A small module, to check the code view.

use std::collections::HashMap;

/// Counts how often each word appears.
pub fn word_counts(text: &str) -> HashMap<&str, usize> {
    let mut counts = HashMap::new();
    for word in text.split_whitespace() {
        *counts.entry(word).or_insert(0) += 1;
    }
    counts
}

#[derive(Debug, Clone, PartialEq)]
pub struct Trail {
    pub name: String,
    pub length_km: f32,
    pub climb_m: u32,
}

impl Trail {
    pub fn new(name: impl Into<String>, length_km: f32, climb_m: u32) -> Self {
        Self { name: name.into(), length_km, climb_m }
    }

    /// Minutes at 4 km/h plus 10 minutes per 100 m of climb.
    pub fn minutes(&self) -> u32 {
        let walking = self.length_km / 4.0 * 60.0;
        walking.round() as u32 + self.climb_m / 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_repeated_words() {
        let counts = word_counts(\"a b a\");
        assert_eq!(counts[\"a\"], 2);
    }
}
";

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
    let dir = explorer::canonical(&dir).expect("target/shots exists");
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
        let (_ignoring, ignoring) = folder(
            "notes",
            &[
                (".gitignore", "build.log\n"),
                ("build.log", "x"),
                ("notes.md", "x"),
            ],
        );
        let mut cx = context();
        let (handle, _) = open(&mut cx, Launch::Folders(vec![ignoring]));
        click(&mut cx, handle, "show-ignored");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("show-ignored-{suffix}"),
        ));

        // An unreadable folder and a symlink among ordinary rows.
        let (_special, special) = folder(
            "project",
            &[
                ("locked/secret.txt", "x"),
                ("real/inside.txt", "x"),
                ("a.txt", "x"),
            ],
        );
        std::os::unix::fs::symlink(special.join("real"), special.join("link"))
            .expect("make a symlink");
        chmod(&special.join("locked"), 0o000);
        let mut cx = context();
        let (handle, _) = open(&mut cx, Launch::Folders(vec![special.clone()]));
        chmod(&special.join("locked"), 0o755);
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("special-rows-{suffix}"),
        ));

        // The gallery of seven files, one of each kind: pictures, text and
        // a file the stage cannot show.
        let (_stage, stage) = stage_folder();
        let mut cx = context();
        let (handle, _) = open_sized(
            &mut cx,
            Launch::Folders(vec![stage.clone()]),
            size(px(1200.), px(760.)),
        );
        click(&mut cx, handle, "select-all");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("gallery-{suffix}"),
        ));
        // Keyboard focus on a tile that is not current: its ring.
        cx.update_window(handle, |_, window, cx| {
            for key in ["tab", "right", "down"] {
                window.press(key, cx);
            }
        })
        .expect("move tile focus");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("gallery-focus-{suffix}"),
        ));
        // A narrow stage: one column of tiles.
        drag_divider(&mut cx, handle, 960.);
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("gallery-narrow-{suffix}"),
        ));

        // The stage, one file of each kind, opened from the first tile: the
        // position and the grid button show under each.
        let mut cx = context();
        let (handle, _) = open_sized(
            &mut cx,
            Launch::Folders(vec![stage.clone()]),
            size(px(1200.), px(760.)),
        );
        click(&mut cx, handle, "select-all");
        cx.update_window(handle, |_, window, cx| {
            window.press("tab", cx);
            window.press("enter", cx);
        })
        .expect("open the first tile");
        for name in [
            "stage-landscape",
            "stage-portrait",
            "stage-icon",
            "stage-markdown",
            "stage-code",
            "stage-long-lines",
            "stage-unsupported",
        ] {
            written.push(save(
                &mut cx,
                handle,
                mode,
                &dir,
                &format!("{name}-{suffix}"),
            ));
            cx.update_window(handle, |_, window, cx| window.press("right", cx))
                .expect("next file");
        }

        // The presentation: the stage alone, from the first file.
        let mut cx = context();
        let (handle, _) = open_sized(
            &mut cx,
            Launch::Folders(vec![stage.clone()]),
            size(px(1200.), px(760.)),
        );
        click(&mut cx, handle, "select-all");
        cx.update_window(handle, |_, window, cx| {
            window.press("tab", cx);
            window.press("space", cx);
        })
        .expect("present");
        for name in [
            "present-landscape",
            "present-portrait",
            "present-icon",
            "present-markdown",
            "present-code",
        ] {
            written.push(save(
                &mut cx,
                handle,
                mode,
                &dir,
                &format!("{name}-{suffix}"),
            ));
            cx.update_window(handle, |_, window, cx| window.press("right", cx))
                .expect("next file");
        }

        // A file outside the gallery: no position.
        let mut cx = context();
        let (handle, view) = open_sized(
            &mut cx,
            Launch::Folders(vec![stage.clone()]),
            size(px(1200.), px(760.)),
        );
        let rust = node(&mut cx, &view, "e-lib.rs");
        cx.update_window(handle, |_, window, cx| window.click(rust, cx))
            .expect("show a file");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("stage-peek-{suffix}"),
        ));

        // Two folders, each a top-level row in the order added, the second
        // closed.
        let (_folders, first) = folder(
            "web-app",
            &[
                ("src/app.rs", "fn app() {}"),
                ("src/lib.rs", "pub mod app;"),
                ("Cargo.toml", "[package]"),
                ("README.md", "# Web app"),
            ],
        );
        let second = first.with_file_name("design-notes");
        for (path, text) in [("brief.md", "# Brief"), ("sketch.png", "png")] {
            std::fs::create_dir_all(&second).expect("make the second folder");
            std::fs::write(second.join(path), text).expect("write the file");
        }
        let mut cx = context();
        let (handle, view) = open(
            &mut cx,
            Launch::Folders(vec![first.clone(), second.clone()]),
        );
        let src = node(&mut cx, &view, "src");
        let notes = folder_row(&mut cx, &view, &second);
        cx.update_window(handle, |_, window, cx| {
            window.within(src).click("chevron", cx);
            window.within(notes.clone()).click("chevron", cx);
            // The pointer leaves the tree.
            let stage = window.find("stage-pane").bounds().center();
            window.pointer_move(stage, None, cx);
        })
        .expect("open and close folders");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("folders-{suffix}"),
        ));

        // The pointer over a folder's row: its remove button, over the Size
        // cell.
        cx.update_window(handle, |_, window, cx| window.hover(notes, cx))
            .expect("hover a folder row");
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("remove-folder-{suffix}"),
        ));

        // A folder that cannot be read: one row, with a warning.
        let mut cx = context();
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/no-such-folder");
        let (handle, _) = open(&mut cx, Launch::Folders(vec![missing]));
        written.push(save(
            &mut cx,
            handle,
            mode,
            &dir,
            &format!("unreadable-{suffix}"),
        ));

        // The same folder followed on disk: its watcher cannot start, so
        // Refresh takes the warning colour.
        let mut cx = context();
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/no-such-folder");
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
                    |window, cx| {
                        let session = Session {
                            watch: true,
                            ..Session::default()
                        };
                        cx.new(|cx| {
                            AppView::with_session(
                                Launch::Folders(vec![missing]),
                                session,
                                window,
                                cx,
                            )
                        })
                    },
                )
            })
            .expect("open the window");
        written.push(save(
            &mut cx,
            opened.0,
            mode,
            &dir,
            &format!("unwatched-{suffix}"),
        ));
    }

    for path in written {
        println!("wrote {}", path.display());
    }
}
