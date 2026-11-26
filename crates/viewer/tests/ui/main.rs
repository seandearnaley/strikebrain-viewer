//! UI integration tests: the real root view in a headless window, driven by
//! clicks and key presses, checked through accessibility facts, geometry
//! and the application state.
//!
//! One module per area of the interface; the helpers they share are here.

mod bar;
mod checks;
mod commands;
mod filter;
mod flick;
mod folders;
mod gallery;
mod header;
mod history;
mod keyboard;
mod launch;
mod layout;
mod menus;
mod persist;
mod presentation;
mod races;
mod redraw;
mod refresh;
mod rows;
mod scale;
mod sidebar;
mod stage;
mod timing;
mod watch;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use explorer::{NodeId, Tree};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, ElementId, Entity, InputEvent as _, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Role, TestAppContext,
    Window, WindowBounds, WindowOptions, point, px, size,
};
use viewer::{AppState, AppView, Launch, Session};

/// A small project. Display order: docs, src, file2.txt, file10.txt,
/// README.md; inside src: app, lib.rs, main.rs.
const PROJECT: &[&str] = &[
    "README.md",
    "file10.txt",
    "file2.txt",
    "docs/a.md",
    "docs/b.md",
    "src/main.rs",
    "src/lib.rs",
    "src/app/view.rs",
];

fn tree(paths: &[&str]) -> Launch {
    Launch::Tree(Arc::new(
        Tree::from_paths(paths).expect("the fixture paths form a tree"),
    ))
}

fn sized_tree(paths: &[(&str, u64)]) -> Launch {
    Launch::Tree(Arc::new(
        Tree::from_paths_with_sizes(paths).expect("the fixture paths form a tree"),
    ))
}

/// The folder `tests/fixture`, scanned from disk.
fn fixture_folder() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixture")
}

/// What the app says of a path that does not exist: the operating system's
/// message, without Rust's error code. Each system words it its own way, so
/// it is asked rather than written here.
fn not_found() -> String {
    let error = std::fs::metadata(fixture_folder().join("no-such-folder"))
        .expect_err("the folder does not exist");
    let code = error.raw_os_error().expect("the system's own error");
    let text = error.to_string();
    text.strip_suffix(&format!(" (os error {code})"))
        .expect("Rust ends the message with its code")
        .to_owned()
}

/// A temporary folder holding `files`, each with its text.
fn folder(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary folder");
    for (path, text) in files {
        write(dir.path(), path, text);
    }
    dir
}

/// Takes every permission from a folder and gives them back on drop, so
/// the temporary folder can be deleted even if the test fails.
struct Locked(PathBuf);

impl Locked {
    /// `None` when the lock does not stop reads, as for root.
    fn new(path: &Path) -> Option<Self> {
        Self::with_mode(path, 0o000)
    }

    /// A folder that can be passed through but not listed: what is inside
    /// it can still be opened by its path. `None` as for [`Locked::new`].
    fn unlisted(path: &Path) -> Option<Self> {
        Self::with_mode(path, 0o311)
    }

    fn with_mode(path: &Path, mode: u32) -> Option<Self> {
        set_mode(path, mode);
        let locked = Self(path.to_path_buf());
        std::fs::read_dir(path).is_err().then_some(locked)
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        set_mode(&self.0, 0o755);
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .expect("set the folder's permissions");
}

/// Windows has no permission bits: the folder stays readable, so
/// [`Locked::new`] gives `None` and its tests end early.
#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) {}

/// Writes `bytes` to `root/path`, making its folders.
fn write(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().expect("a file has a folder"))
        .expect("make the file's folder");
    std::fs::write(path, bytes).expect("write the file");
}

/// Opens the app in a 1000 x 700 window with the app's key bindings.
fn open_app(cx: &mut TestAppContext, launch: Launch) -> (AnyWindowHandle, Entity<AppView>) {
    open_app_with_session(cx, launch, Session::default())
}

/// Opens the app as `open_app` does, with `session`.
fn open_app_with_session(
    cx: &mut TestAppContext,
    launch: Launch,
    session: Session,
) -> (AnyWindowHandle, Entity<AppView>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        viewer::init(cx);
    });
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
            |window, cx| cx.new(|cx| AppView::with_session(launch, session, window, cx)),
        )
        .expect("open test window")
    });
    cx.run_until_parked();
    opened
}

/// The application state entity.
fn state(cx: &mut TestAppContext, view: &Entity<AppView>) -> Entity<AppState> {
    cx.update(|cx| view.read(cx).state().clone())
}

/// Reads the application state.
fn read<R>(cx: &mut TestAppContext, view: &Entity<AppView>, f: impl FnOnce(&AppState) -> R) -> R {
    cx.update(|cx| f(view.read(cx).state().read(cx)))
}

/// The node at `path` in the tree on screen: from the root of a tree
/// built in memory, or inside the first folder of a tree of folders, where
/// `""` is that folder.
fn node(cx: &mut TestAppContext, view: &Entity<AppView>, path: &str) -> NodeId {
    read(cx, view, |state| {
        let tree = state.explorer().expect("a tree is on screen").tree();
        let from = if tree.is_joined() {
            tree.folders().next().expect("a folder is open")
        } else {
            NodeId::ROOT
        };
        tree.find_from(from, path)
            .expect("the path is in the fixture")
    })
}

/// The node at `path` on disk in the tree on screen.
fn node_at(cx: &mut TestAppContext, view: &Entity<AppView>, path: &Path) -> NodeId {
    read(cx, view, |state| {
        let tree = state.explorer().expect("a tree is on screen").tree();
        tree.find_abs(&explorer::canonical(path).expect("the path exists"))
            .expect("the path is on screen")
    })
}

/// The checked files, each by its path inside its folder.
fn checked(cx: &mut TestAppContext, view: &Entity<AppView>) -> Vec<String> {
    read(cx, view, |state| {
        let explorer = state.explorer().expect("a tree is on screen");
        let tree = explorer.tree();
        explorer
            .selection()
            .iter()
            .map(|id| tree.path_from(tree.folder_of(id).expect("a file is in a folder"), id))
            .collect()
    })
}

/// A folder's name: what its row shows.
fn name_of(folder: &Path) -> String {
    folder
        .file_name()
        .expect("a folder has a name")
        .to_string_lossy()
        .into_owned()
}

/// `rows` under the row of `folder`, as a tree of that one folder shows
/// them.
fn under(folder: &Path, rows: &[&str]) -> Vec<String> {
    std::iter::once(name_of(folder))
        .chain(rows.iter().map(|&row| row.to_owned()))
        .collect()
}

fn row(id: NodeId) -> ElementId {
    ("row", id.index()).into()
}

/// The labels of the tree rows on screen, top to bottom.
fn labels(window: &Window) -> Vec<String> {
    window
        .find_all_by_role(Role::TreeItem)
        .iter()
        .filter(|row| row.visible())
        .map(|row| row.label().expect("every row has a label").to_owned())
        .collect()
}

/// The labels of the rows on screen after a fresh frame.
fn rows_now(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        labels(window)
    })
    .expect("read the rows")
}

/// The name on the stage, if any: what an unsupported file shows, which
/// every file of a tree built in memory is.
fn stage(window: &Window) -> Option<String> {
    window
        .try_find("stage-file")
        .map(|file| file.label().expect("the stage names its file").to_owned())
}

/// The name on the stage once every task has run and a frame is drawn.
fn stage_now(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        stage(window)
    })
    .expect("read the stage")
}

fn current(cx: &mut TestAppContext, view: &Entity<AppView>) -> Option<NodeId> {
    read(cx, view, AppState::current)
}

fn focus(cx: &mut TestAppContext, view: &Entity<AppView>) -> Option<NodeId> {
    read(cx, view, AppState::focus)
}

/// Presses `key` in the window and lets every task finish.
fn press(cx: &mut TestAppContext, handle: AnyWindowHandle, key: &str) {
    cx.update_window(handle, |_, window, cx| window.press(key, cx))
        .expect("press a key");
    cx.run_until_parked();
}

/// Clicks the element `id` and lets every task finish.
fn click(cx: &mut TestAppContext, handle: AnyWindowHandle, id: impl Into<ElementId>) {
    let id = id.into();
    cx.update_window(handle, |_, window, cx| window.click(id, cx))
        .expect("click");
    cx.run_until_parked();
}

/// Dispatches `action` from the focused element, as a key, a menu item or
/// a button does, and lets every task finish.
fn dispatch(cx: &mut TestAppContext, handle: AnyWindowHandle, action: impl gpui_kit::Action) {
    cx.update_window(handle, |_, window, cx| {
        // The action travels the dispatch path of the frame on screen.
        window.render_frame(cx);
        window.dispatch_action(Box::new(action), cx);
    })
    .expect("dispatch an action");
    cx.run_until_parked();
}

/// Moves the pointer to `position`, presses and releases the left button,
/// without drawing a frame first or after: the click reaches the listeners
/// of the last frame drawn, as a click does that the platform delivers
/// before the next frame.
fn click_without_frame(window: &mut Window, position: Point<Pixels>, cx: &mut App) {
    let modifiers = Modifiers::default();
    let button = MouseButton::Left;
    let events = [
        MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers,
        }
        .to_platform_input(),
        MouseDownEvent {
            button,
            position,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        MouseUpEvent {
            button,
            position,
            modifiers,
            click_count: 1,
        }
        .to_platform_input(),
    ];
    for event in events {
        window.dispatch_event(event, cx);
    }
}

/// Clicks into the filter input, types `text` there and lets the filter
/// job land.
fn type_query(cx: &mut TestAppContext, handle: AnyWindowHandle, text: &str) {
    cx.update_window(handle, |_, window, cx| {
        window.click("filter", cx);
        window.input(text, cx);
    })
    .expect("type into the filter");
    cx.run_until_parked();
}

/// What the filter input holds.
fn filter_value(cx: &mut TestAppContext, handle: AnyWindowHandle) -> String {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.find("filter").value().unwrap_or_default().to_owned()
    })
    .expect("read the filter")
}

/// Whether the element `id` holds keyboard focus.
fn is_focused(cx: &mut TestAppContext, handle: AnyWindowHandle, id: &'static str) -> bool {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.find(id).focused() == Some(true)
    })
    .expect("read focus")
}

/// Drags the divider from where it is to `to_x`, at the explorer's mid-height.
fn drag_divider(cx: &mut TestAppContext, handle: AnyWindowHandle, to_x: f32) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let explorer = window.find("explorer-pane").bounds();
        let mid_height = explorer.center().y;
        window.drag(
            point(explorer.right(), mid_height),
            point(px(to_x), mid_height),
            cx,
        );
        window.render_frame(cx);
    })
    .expect("drag the divider in the test window");
}
