//! The gallery: the stage's grid of the checked files. Which view shows,
//! opening a tile and going back, the keys over the tiles, presenting from
//! either view, refreshes, what a tile shows, its cache and its scale.

use std::path::Path;

use explorer::NodeId;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Pixels, Role, ScrollDelta, TestAppContext, point, px,
};
use viewer::{AppState, AppView, GalleryProbe, Launch, StageView};

use crate::stage::{Shown, numbered, png, position, shown, turned_jpeg};
use crate::{
    click, current, drag_divider, is_focused, node, open_app, press, read, row, state, tree, write,
};

/// The tiles on screen, by the names they carry, top left to bottom right.
fn tiles(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<String> {
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all_by_role(Role::GridCell)
            .iter()
            .filter(|tile| tile.visible())
            .map(|tile| tile.label().expect("a tile names its file").to_owned())
            .collect()
    })
    .expect("read the tiles")
}

/// What the stage shows: the gallery, a file, or nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
enum View {
    Blank,
    Gallery,
    File(Shown),
}

fn view_of(cx: &mut TestAppContext, handle: AnyWindowHandle) -> View {
    let file = shown(cx, handle);
    let gallery = cx
        .update_window(handle, |_, window, _| window.try_find("gallery").is_some())
        .expect("read the stage");
    assert!(
        !(gallery && file.is_some()),
        "the gallery and {file:?} at once"
    );
    match file {
        Some(file) => View::File(file),
        None if gallery => View::Gallery,
        None => View::Blank,
    }
}

fn code(name: &str) -> View {
    View::File(Shown::Code(name.into()))
}

/// The number in a tile's name: 7 for f007.rs.
fn number(name: &str) -> usize {
    name.trim_start_matches('f')
        .trim_end_matches(".rs")
        .parse()
        .expect("tiles are numbered")
}

/// The name of the tile with keyboard focus.
fn focused_tile(cx: &mut TestAppContext, view: &Entity<AppView>) -> Option<String> {
    read(cx, view, |state| {
        let tree = state.explorer()?.tree();
        Some(tree.name(state.tile()?).to_owned())
    })
}

fn probe(cx: &mut TestAppContext, view: &Entity<AppView>) -> GalleryProbe {
    cx.update(|cx| view.read(cx).gallery_probe(cx))
}

fn tile(id: NodeId) -> (&'static str, usize) {
    ("tile", id.index())
}

/// Clicks the checkbox of the row of `path`.
fn check(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<AppView>, path: &str) {
    let id = node(cx, view, path);
    cx.update_window(handle, |_, window, cx| {
        window.within(row(id)).click("check", cx);
    })
    .expect("click the checkbox");
    cx.run_until_parked();
}

/// A folder of `count` small Rust files, f000.rs on.
fn files(count: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary folder");
    for ix in 0..count {
        write(
            dir.path(),
            &format!("f{ix:03}.rs"),
            format!("fn f{ix}() {{}}\n"),
        );
    }
    dir
}

/// Opens `dir`, checks every file and moves keyboard focus to the stage,
/// which shows the gallery.
fn gallery_of(cx: &mut TestAppContext, dir: &Path) -> (AnyWindowHandle, Entity<AppView>) {
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.to_owned()]));
    click(cx, handle, "select-all");
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));
    assert_eq!(read(cx, &view, AppState::stage_view), StageView::Gallery);
    (handle, view)
}

/// Rescans the folders, as a refresh does, and lets it land.
fn refresh(cx: &mut TestAppContext, view: &Entity<AppView>) {
    let state = state(cx, view);
    cx.update(|cx| state.update(cx, AppState::refresh));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn two_checked_files_show_the_gallery_one_shows_the_file_none_a_blank_stage(
    cx: &mut TestAppContext,
) {
    let dir = files(3);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    assert_eq!(view_of(cx, handle), View::Blank);

    check(cx, handle, &view, "f000.rs");
    assert_eq!(view_of(cx, handle), code("f000.rs"));
    check(cx, handle, &view, "f001.rs");
    assert_eq!(view_of(cx, handle), View::Gallery);
    assert_eq!(tiles(cx, handle), ["f000.rs", "f001.rs"]);
    // Unchecking down to one shows that one; to none, nothing.
    check(cx, handle, &view, "f000.rs");
    assert_eq!(view_of(cx, handle), code("f001.rs"));
    check(cx, handle, &view, "f001.rs");
    assert_eq!(view_of(cx, handle), View::Blank);
    assert_eq!(current(cx, &view), None);

    // Select all, then Clear.
    click(cx, handle, "select-all");
    assert_eq!(view_of(cx, handle), View::Gallery);
    assert_eq!(tiles(cx, handle), ["f000.rs", "f001.rs", "f002.rs"]);
    click(cx, handle, "clear");
    assert_eq!(view_of(cx, handle), View::Blank);

    // A folder's checkbox checks its files: the gallery.
    let top = node(cx, &view, "");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(top)).click("check", cx);
    })
    .unwrap();
    assert_eq!(view_of(cx, handle), View::Gallery);
}

#[gpui_kit::test]
fn a_file_row_opens_its_file_in_the_file_view_checked_or_not(cx: &mut TestAppContext) {
    let dir = files(4);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    for path in ["f000.rs", "f001.rs", "f002.rs"] {
        check(cx, handle, &view, path);
    }
    assert_eq!(view_of(cx, handle), View::Gallery);

    let b = node(cx, &view, "f001.rs");
    click(cx, handle, row(b));
    assert_eq!(view_of(cx, handle), code("f001.rs"));
    assert_eq!(position(cx, handle).as_deref(), Some("2 / 3"));

    // Enter on a row that is not checked.
    let c = node(cx, &view, "f002.rs");
    click(cx, handle, row(c));
    assert_eq!(view_of(cx, handle), code("f002.rs"));
    press(cx, handle, "down");
    press(cx, handle, "enter");
    assert_eq!(view_of(cx, handle), code("f003.rs"));
    assert_eq!(position(cx, handle), None);
}

#[gpui_kit::test]
fn a_click_or_enter_on_a_tile_opens_it_and_escape_goes_back_to_its_tile(cx: &mut TestAppContext) {
    let dir = files(3);
    let (handle, view) = gallery_of(cx, dir.path());
    let b = node(cx, &view, "f001.rs");
    click(cx, handle, tile(b));
    assert_eq!(view_of(cx, handle), code("f001.rs"));
    assert_eq!(current(cx, &view), Some(b));
    assert!(is_focused(cx, handle, "stage"));

    press(cx, handle, "escape");
    assert_eq!(view_of(cx, handle), View::Gallery);
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f001.rs"));
    assert!(is_focused(cx, handle, "stage"));
    // The tile with focus shows its ring after a key.
    cx.update_window(handle, |_, window, _| {
        assert!(window.within(tile(b)).try_find("tile-ring").is_some());
    })
    .unwrap();

    press(cx, handle, "right");
    press(cx, handle, "enter");
    assert_eq!(view_of(cx, handle), code("f002.rs"));
    // ← and → move through the gallery as before.
    press(cx, handle, "left");
    assert_eq!(view_of(cx, handle), code("f001.rs"));
}

#[gpui_kit::test]
fn escape_after_flicking_goes_back_to_that_files_tile_scrolled_into_view(cx: &mut TestAppContext) {
    let dir = files(80);
    let (handle, view) = gallery_of(cx, dir.path());
    assert!(tiles(cx, handle).contains(&"f000.rs".to_owned()));
    press(cx, handle, "enter");
    assert_eq!(view_of(cx, handle), code("f000.rs"));
    for _ in 0..70 {
        press(cx, handle, "right");
    }
    assert_eq!(view_of(cx, handle), code("f070.rs"));

    press(cx, handle, "escape");
    assert_eq!(view_of(cx, handle), View::Gallery);
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f070.rs"));
    let on_screen = tiles(cx, handle);
    assert!(on_screen.contains(&"f070.rs".to_owned()), "{on_screen:?}");
    assert!(!on_screen.contains(&"f000.rs".to_owned()));
}

#[gpui_kit::test]
fn the_grid_button_goes_back_with_two_or_more_files_checked(cx: &mut TestAppContext) {
    let dir = files(3);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    click(cx, handle, "select-all");
    let a = node(cx, &view, "f000.rs");
    click(cx, handle, row(a));
    assert_eq!(view_of(cx, handle), code("f000.rs"));
    let button = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let button = window.find("gallery-button");
            let position = window.find("stage-position");
            assert!(button.bounds().right() <= position.bounds().left());
            button
        })
        .unwrap();
    assert_eq!(button.label(), Some("Gallery"));
    click(cx, handle, "gallery-button");
    assert_eq!(view_of(cx, handle), View::Gallery);
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f000.rs"));
    assert!(is_focused(cx, handle, "stage"));
    // From the pointer: no ring.
    cx.update_window(handle, |_, window, _| {
        assert!(window.within(tile(a)).try_find("tile-ring").is_none());
    })
    .unwrap();

    // One checked file: no button, and Esc goes to the explorer.
    for path in ["f001.rs", "f002.rs"] {
        check(cx, handle, &view, path);
    }
    assert_eq!(view_of(cx, handle), code("f000.rs"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("gallery-button").is_none());
    })
    .unwrap();
    // From the tree, where the checkboxes left focus.
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));
    press(cx, handle, "escape");
    assert!(is_focused(cx, handle, "tree"));
    assert_eq!(view_of(cx, handle), code("f000.rs"));
}

#[gpui_kit::test]
fn the_arrows_move_tile_focus_across_row_ends_and_onto_a_short_last_row(cx: &mut TestAppContext) {
    // Two full rows and two tiles in the last, whatever the columns.
    let columns = {
        let dir = files(2);
        let (_, view) = gallery_of(cx, dir.path());
        probe(cx, &view).columns()
    };
    assert!(columns >= 3, "{columns} columns");
    let dir = files(2 * columns + 2);
    let (handle, view) = gallery_of(cx, dir.path());
    let at = |cx: &mut TestAppContext| number(&focused_tile(cx, &view).expect("a tile has focus"));
    assert_eq!(at(cx), 0);
    press(cx, handle, "left");
    assert_eq!(at(cx), 0, "the first tile stays");
    for _ in 0..columns {
        press(cx, handle, "right");
    }
    assert_eq!(at(cx), columns, "the row's end leads onto the next row");
    press(cx, handle, "left");
    assert_eq!(at(cx), columns - 1, "and back");
    press(cx, handle, "down");
    assert_eq!(at(cx), 2 * columns - 1);
    // Nothing under the last column in the short row: its last tile.
    press(cx, handle, "down");
    assert_eq!(at(cx), 2 * columns + 1);
    press(cx, handle, "down");
    assert_eq!(at(cx), 2 * columns + 1, "the last row");
    press(cx, handle, "up");
    assert_eq!(at(cx), columns + 1);
    press(cx, handle, "home");
    assert_eq!(at(cx), 0);
    press(cx, handle, "up");
    assert_eq!(at(cx), 0);
    press(cx, handle, "end");
    assert_eq!(at(cx), 2 * columns + 1);
    press(cx, handle, "right");
    assert_eq!(at(cx), 2 * columns + 1, "the last tile stays");
}

#[gpui_kit::test]
fn page_keys_move_by_a_screen_of_rows_and_keep_the_tile_on_screen(cx: &mut TestAppContext) {
    let dir = files(120);
    let (handle, view) = gallery_of(cx, dir.path());
    let (columns, page) = (probe(cx, &view).columns(), probe(cx, &view).page());
    assert!(page >= 1);
    let at = |cx: &mut TestAppContext| number(&focused_tile(cx, &view).expect("a tile has focus"));
    press(cx, handle, "right");
    press(cx, handle, "pagedown");
    assert_eq!(at(cx), 1 + page * columns);
    let name = focused_tile(cx, &view).unwrap();
    assert!(tiles(cx, handle).contains(&name));
    press(cx, handle, "pagedown");
    press(cx, handle, "pageup");
    assert_eq!(at(cx), 1 + page * columns);
    press(cx, handle, "end");
    assert_eq!(at(cx), 119);
    assert!(tiles(cx, handle).contains(&"f119.rs".to_owned()));
    assert!(!tiles(cx, handle).contains(&"f000.rs".to_owned()));
    press(cx, handle, "pagedown");
    assert_eq!(at(cx), 119);
    press(cx, handle, "home");
    assert!(tiles(cx, handle).contains(&"f000.rs".to_owned()));
}

#[gpui_kit::test]
fn escape_in_the_gallery_focuses_the_explorer(cx: &mut TestAppContext) {
    let dir = files(3);
    let (handle, _) = gallery_of(cx, dir.path());
    press(cx, handle, "escape");
    assert!(is_focused(cx, handle, "tree"));
    assert_eq!(view_of(cx, handle), View::Gallery);
    // Tab comes back to the stage, in the gallery.
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));
}

#[gpui_kit::test]
fn space_presents_from_the_focused_tile_and_escape_leaves_to_the_gallery(cx: &mut TestAppContext) {
    let dir = files(3);
    let (handle, view) = gallery_of(cx, dir.path());
    press(cx, handle, "right");
    press(cx, handle, "space");
    assert!(read(cx, &view, AppState::is_presenting));
    assert_eq!(view_of(cx, handle), code("f001.rs"));
    press(cx, handle, "space");
    assert_eq!(view_of(cx, handle), code("f002.rs"));

    press(cx, handle, "escape");
    assert!(!read(cx, &view, AppState::is_presenting));
    assert_eq!(view_of(cx, handle), View::Gallery);
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f002.rs"));
    assert!(is_focused(cx, handle, "stage"));
}

#[gpui_kit::test]
fn a_presentation_from_the_file_view_leaves_to_the_file_view(cx: &mut TestAppContext) {
    let dir = files(3);
    let (handle, view) = gallery_of(cx, dir.path());
    press(cx, handle, "enter");
    press(cx, handle, "space");
    assert!(read(cx, &view, AppState::is_presenting));
    press(cx, handle, "right");
    press(cx, handle, "escape");
    assert!(!read(cx, &view, AppState::is_presenting));
    assert_eq!(view_of(cx, handle), code("f001.rs"));
}

#[gpui_kit::test]
fn a_refresh_that_leaves_one_checked_file_shows_it(cx: &mut TestAppContext) {
    let dir = files(3);
    let (handle, view) = gallery_of(cx, dir.path());
    for name in ["f001.rs", "f002.rs"] {
        std::fs::remove_file(dir.path().join(name)).unwrap();
    }
    refresh(cx, &view);
    assert_eq!(view_of(cx, handle), code("f000.rs"));
}

#[gpui_kit::test]
fn a_refresh_keeps_the_gallery_and_its_focus_on_a_file_still_there(cx: &mut TestAppContext) {
    let dir = files(4);
    let (handle, view) = gallery_of(cx, dir.path());
    press(cx, handle, "right");
    press(cx, handle, "right");
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f002.rs"));
    std::fs::remove_file(dir.path().join("f000.rs")).unwrap();
    refresh(cx, &view);
    assert_eq!(view_of(cx, handle), View::Gallery);
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f002.rs"));
    assert_eq!(tiles(cx, handle), ["f001.rs", "f002.rs", "f003.rs"]);
    // The focused file goes: focus moves to the file now at its place.
    std::fs::remove_file(dir.path().join("f002.rs")).unwrap();
    refresh(cx, &view);
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f003.rs"));
}

#[gpui_kit::test]
fn a_refresh_that_removes_the_open_file_moves_to_its_neighbour(cx: &mut TestAppContext) {
    let dir = files(3);
    let (handle, view) = gallery_of(cx, dir.path());
    press(cx, handle, "right");
    press(cx, handle, "enter");
    assert_eq!(view_of(cx, handle), code("f001.rs"));
    std::fs::remove_file(dir.path().join("f001.rs")).unwrap();
    refresh(cx, &view);
    assert_eq!(view_of(cx, handle), code("f002.rs"));
    assert_eq!(position(cx, handle).as_deref(), Some("2 / 2"));
    press(cx, handle, "escape");
    assert_eq!(view_of(cx, handle), View::Gallery);
    assert_eq!(tiles(cx, handle), ["f000.rs", "f002.rs"]);
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f002.rs"));
}

#[gpui_kit::test]
fn a_text_tile_shows_the_top_of_its_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notes.md", "# Notes\n\n- one\n\t- two\n");
    write(dir.path(), "long.txt", numbered(200, "line ", "\n"));
    let (handle, view) = gallery_of(cx, dir.path());
    tiles(cx, handle);
    let text = |cx: &mut TestAppContext, path: &str| {
        let id = node(cx, &view, path);
        cx.update_window(handle, |_, window, _| {
            let text = window.within(tile(id)).find("tile-text");
            // A label, so assistive technology reads its value.
            assert_eq!(text.role(), Some(Role::Label));
            text.value().map(str::to_owned)
        })
        .unwrap()
    };
    assert_eq!(
        text(cx, "notes.md").as_deref(),
        Some("# Notes\n\n- one\n    - two")
    );
    let head = text(cx, "long.txt").unwrap();
    assert!(head.starts_with("line 0\nline 1\n"), "{head}");
    assert!(head.lines().count() < 200);
}

#[gpui_kit::test]
fn an_image_tile_holds_its_picture_scaled_to_the_tile(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a-photo.png", png(1600, 1200));
    write(dir.path(), "b-icon.png", png(40, 30));
    let (handle, view) = gallery_of(cx, dir.path());
    tiles(cx, handle);
    for path in ["a-photo.png", "b-icon.png"] {
        let id = node(cx, &view, path);
        cx.update_window(handle, |_, window, _| {
            let image = window.within(tile(id)).find("tile-image");
            assert_eq!(image.role(), Some(Role::Image));
            assert_eq!(image.label(), Some(path));
        })
        .unwrap();
    }
    let probe = probe(cx, &view);
    let (width, height) = probe.largest().expect("a thumbnail");
    let fit = probe.fit();
    assert!(
        width <= fit.0 && height <= fit.1,
        "{width} x {height} in {fit:?}"
    );
    // The photo, larger than its box, meets one side of it.
    assert!(
        width == fit.0 || height == fit.1,
        "{width} x {height} in {fit:?}"
    );
}

#[gpui_kit::test]
fn a_file_the_stage_cannot_show_is_its_kinds_icon(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.rs", "fn a() {}");
    write(dir.path(), "broken.png", "not a png at all");
    write(dir.path(), "data.bin", b"\x00\x01\x02");
    write(dir.path(), "big.txt", "text head\n");
    std::fs::File::options()
        .write(true)
        .open(dir.path().join("big.txt"))
        .and_then(|file| file.set_len(10 * 1024 * 1024 + 1))
        .unwrap();
    let (handle, view) = gallery_of(cx, dir.path());
    tiles(cx, handle);
    for path in ["broken.png", "data.bin", "big.txt"] {
        let id = node(cx, &view, path);
        cx.update_window(handle, |_, window, _| {
            let inside = window.within(tile(id));
            assert!(inside.try_find("tile-icon").is_some(), "{path}");
            assert!(inside.try_find("tile-image").is_none(), "{path}");
            assert!(inside.try_find("tile-text").is_none(), "{path}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_hundred_thousand_checked_files_build_only_the_rows_on_screen(cx: &mut TestAppContext) {
    let names: Vec<String> = (0..100_000).map(|i| format!("f{i:06}")).collect();
    let paths: Vec<&str> = names.iter().map(String::as_str).collect();
    let (handle, view) = open_app(cx, tree(&paths));
    click(cx, handle, "select-all");
    press(cx, handle, "tab");
    let built = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find_all_by_role(Role::GridCell).len()
        })
        .unwrap()
    };
    let laid_out = probe(cx, &view);
    // The rows on screen, one partly, and no more.
    let screenful = (laid_out.page() + 2) * laid_out.columns();
    assert!(built(cx) <= screenful, "{} of {screenful}", built(cx));
    press(cx, handle, "end");
    assert!(built(cx) <= screenful);
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f099999"));
    assert!(tiles(cx, handle).contains(&"f099999".to_owned()));
    // A tree built in memory names files it has not got: their icons.
    assert_eq!(probe(cx, &view).loading(), 0);
}

#[gpui_kit::test]
fn the_tiles_content_stays_within_its_budget_and_tiles_passed_over_are_not_read(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let picture = png(640, 480);
    for ix in 0..300 {
        write(dir.path(), &format!("p{ix:03}.png"), &picture);
    }
    let (handle, view) = gallery_of(cx, dir.path());
    tiles(cx, handle);
    let first = probe(cx, &view);
    assert!(first.thumbnails() > 0);
    assert!(first.bytes() <= first.budget());
    // Straight to the end: the rows between are never read.
    press(cx, handle, "end");
    tiles(cx, handle);
    let after = probe(cx, &view);
    assert!(after.bytes() <= after.budget());
    let screenful = (after.page() + 3) * after.columns();
    assert!(
        after.thumbnails() <= 2 * screenful,
        "{} read, of 300",
        after.thumbnails()
    );
    assert_eq!(after.loading(), 0);
}

/// The tiles on screen with where each stands: its name and its top.
fn placed(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Vec<(String, Pixels)> {
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .find_all_by_role(Role::GridCell)
            .iter()
            .filter(|tile| tile.visible())
            .map(|tile| {
                let name = tile.label().expect("a tile names its file").to_owned();
                (name, tile.bounds().top())
            })
            .collect()
    })
    .expect("read the tiles")
}

/// Where the tile named `name` stands, if it is on screen.
fn top_of(placed: &[(String, Pixels)], name: &str) -> Option<Pixels> {
    placed
        .iter()
        .find(|(each, _)| each == name)
        .map(|(_, top)| *top)
}

#[gpui_kit::test]
fn resizing_far_down_a_long_gallery_keeps_the_focused_tile_where_it_stood(cx: &mut TestAppContext) {
    let dir = files(3000);
    let (handle, view) = gallery_of(cx, dir.path());
    press(cx, handle, "end");
    for _ in 0..80 {
        press(cx, handle, "pageup");
    }
    let focus = focused_tile(cx, &view).expect("a tile has focus");
    assert!(number(&focus) > 1000, "{focus}");
    let before = top_of(&placed(cx, handle), &focus).expect("the focused tile is on screen");
    // Wider and narrower, within three columns: every tile changes size.
    for x in [280., 260., 300.] {
        drag_divider(cx, handle, x);
        assert_eq!(probe(cx, &view).columns(), 3);
        let top = top_of(&placed(cx, handle), &focus);
        let top = top.unwrap_or_else(|| panic!("{focus} left the screen at {x}"));
        assert!(
            (top - before).abs() < px(1.),
            "{focus} moved from {before:?} to {top:?}"
        );
    }
}

#[gpui_kit::test]
fn resizing_keeps_the_first_tile_on_screen_when_the_focused_one_is_not(cx: &mut TestAppContext) {
    let dir = files(3000);
    let (handle, view) = gallery_of(cx, dir.path());
    // Focus stays on the first tile; the wheel goes far down, a row and a
    // half into one.
    cx.update_window(handle, |_, window, cx| {
        let delta = ScrollDelta::Pixels(point(px(0.), px(-120_000.)));
        window.scroll("gallery", delta, cx);
    })
    .unwrap();
    let before = placed(cx, handle);
    let (first, top) = before.first().cloned().expect("tiles on screen");
    assert!(number(&first) > 1000, "{first}");
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f000.rs"));
    drag_divider(cx, handle, 280.);
    let after = placed(cx, handle);
    let (first_after, top_after) = after.first().cloned().expect("tiles on screen");
    assert_eq!(first_after, first);
    // The same share of its row is above the top as before: a row starts
    // 16 px above its tiles.
    let row = |placed: &[(String, Pixels)]| {
        let second_row = placed.iter().find(|(_, each)| *each > placed[0].1).unwrap();
        second_row.1 - placed[0].1
    };
    let share = |top: Pixels, row: Pixels| (px(16.) - top) / row;
    let (was, now) = (share(top, row(&before)), share(top_after, row(&after)));
    assert!(
        (was - now).abs() < 0.01,
        "{was} of a row above the top, then {now}"
    );
}

#[gpui_kit::test]
fn a_change_of_columns_keeps_the_focused_tile_on_screen(cx: &mut TestAppContext) {
    let dir = files(120);
    let (handle, view) = gallery_of(cx, dir.path());
    for _ in 0..20 {
        press(cx, handle, "down");
    }
    let focus = focused_tile(cx, &view).expect("a tile has focus");
    assert_eq!(probe(cx, &view).columns(), 3);
    assert!(top_of(&placed(cx, handle), &focus).is_some());
    // One column, then back to three.
    for (x, columns) in [(760., 1), (300., 3)] {
        drag_divider(cx, handle, x);
        assert_eq!(probe(cx, &view).columns(), columns);
        assert!(
            top_of(&placed(cx, handle), &focus).is_some(),
            "{focus} left the screen at {columns} columns"
        );
    }
}

#[gpui_kit::test]
fn reads_a_scroll_leaves_behind_hold_their_slots_until_they_end(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let picture = png(64, 48);
    for ix in 0..300 {
        write(dir.path(), &format!("p{ix:03}.png"), &picture);
    }
    let (handle, view) = gallery_of(cx, dir.path());
    tiles(cx, handle);
    // The end starts four reads; five pages up, before any of them runs,
    // leave them behind. They are told to stop, and no fifth read starts
    // while they still hold their slots.
    let keys = |cx: &mut TestAppContext, keys: &[&str]| {
        cx.update_window(handle, |_, window, cx| {
            for key in keys {
                window.press(key, cx);
            }
            window.render_frame(cx);
        })
        .unwrap();
    };
    keys(cx, &["end"]);
    let reading = probe(cx, &view);
    assert_eq!((reading.loading(), reading.stopping()), (4, 0));
    keys(cx, &["pageup"; 5]);
    let left = probe(cx, &view);
    assert_eq!((left.loading(), left.stopping()), (4, 4));
    // Once they end, the tiles now on screen are read.
    let on_screen = tiles(cx, handle);
    assert_eq!(probe(cx, &view).loading(), 0);
    assert!(!on_screen.contains(&"p299.png".to_owned()));
    for name in on_screen {
        let id = node(cx, &view, &name);
        cx.update_window(handle, |_, window, _| {
            assert!(
                window.within(tile(id)).try_find("tile-image").is_some(),
                "{name}"
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_turned_photos_tile_is_turned(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    // 800 x 400, its EXIF orientation a quarter turn clockwise.
    write(dir.path(), "a-turned.jpg", turned_jpeg(800, 400));
    write(dir.path(), "b.rs", "fn b() {}");
    let (handle, view) = gallery_of(cx, dir.path());
    tiles(cx, handle);
    let (width, height) = probe(cx, &view).largest().expect("a thumbnail");
    assert!(height > width, "{width} x {height}");
}

/// Whether the tile of the file at `path` shows the focus ring.
fn ringed(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    view: &Entity<AppView>,
    path: &str,
) -> bool {
    let id = node(cx, view, path);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within(tile(id)).try_find("tile-ring").is_some()
    })
    .expect("read the tile")
}

#[gpui_kit::test]
fn tab_into_the_gallery_and_leaving_a_presentation_show_the_ring(cx: &mut TestAppContext) {
    let dir = files(3);
    let (handle, view) = gallery_of(cx, dir.path());
    // Tab brought focus here: the focused tile shows its ring, as the
    // tree's row does.
    assert!(ringed(cx, handle, &view, "f000.rs"));
    // A press below the tiles hides it.
    click(cx, handle, "gallery");
    assert!(!ringed(cx, handle, &view, "f000.rs"));
    assert!(is_focused(cx, handle, "stage"));
    // Presenting and leaving by key shows it again.
    press(cx, handle, "space");
    press(cx, handle, "escape");
    assert_eq!(view_of(cx, handle), View::Gallery);
    assert!(ringed(cx, handle, &view, "f000.rs"));
    // Shift-Tab away and Tab back.
    click(cx, handle, "gallery");
    press(cx, handle, "shift-tab");
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));
    assert!(ringed(cx, handle, &view, "f000.rs"));
}

#[gpui_kit::test]
fn unchecking_another_file_leaves_the_focused_tile_where_it_is(cx: &mut TestAppContext) {
    let dir = files(100);
    let (handle, view) = gallery_of(cx, dir.path());
    assert_eq!(current(cx, &view), Some(node(cx, &view, "f000.rs")));
    for _ in 0..70 {
        press(cx, handle, "right");
    }
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f070.rs"));
    // A file other than the one on the stage, unchecked in the tree.
    check(cx, handle, &view, "f005.rs");
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f070.rs"));
    assert!(tiles(cx, handle).contains(&"f070.rs".to_owned()));
    // Checking a file puts it on the stage: focus follows it.
    check(cx, handle, &view, "f005.rs");
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f005.rs"));
    assert!(tiles(cx, handle).contains(&"f005.rs".to_owned()));
}

#[gpui_kit::test]
fn a_focused_file_that_goes_gives_its_focus_to_the_file_now_in_its_place(cx: &mut TestAppContext) {
    let dir = files(100);
    let (handle, view) = gallery_of(cx, dir.path());
    for _ in 0..70 {
        press(cx, handle, "right");
    }
    std::fs::remove_file(dir.path().join("f070.rs")).unwrap();
    refresh(cx, &view);
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f071.rs"));
    assert!(tiles(cx, handle).contains(&"f071.rs".to_owned()));
    // The next key goes on from there.
    press(cx, handle, "right");
    assert_eq!(focused_tile(cx, &view).as_deref(), Some("f072.rs"));
}

#[gpui_kit::test]
fn presenting_from_the_gallery_brings_the_files_row_into_view_in_the_tree(cx: &mut TestAppContext) {
    let dir = files(80);
    let (handle, view) = gallery_of(cx, dir.path());
    for _ in 0..70 {
        press(cx, handle, "right");
    }
    let id = node(cx, &view, "f070.rs");
    let row_shown = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(row(id)).is_some_and(|row| row.visible())
        })
        .expect("read the tree")
    };
    assert!(!row_shown(cx));
    press(cx, handle, "space");
    press(cx, handle, "escape");
    assert!(row_shown(cx), "the presented file's row is in view");
}
