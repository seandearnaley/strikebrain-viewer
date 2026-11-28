//! The window's title bar: one row above both panes, its part over the
//! explorer as wide as the explorer, its buttons, its More menu, and the
//! stage's title.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext, px};
use viewer::keys::{SelectAllFiles, ToggleShowIgnored};
use viewer::{AppState, Launch};

use crate::{
    PROJECT, click, dispatch, drag_divider, folder, node, open_app, press, read, row, rows_now,
    tree,
};

/// The stage's title in the bar after a fresh frame; `None` when it shows
/// none.
fn bar_title(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find("stage-title")
            .map(|title| title.label().expect("the title is a label").to_owned())
    })
    .expect("read the bar's title")
}

#[gpui_kit::test]
fn the_bar_is_one_row_above_both_panes(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let bar = window.find("title-bar").bounds();
        assert_eq!(bar.top(), px(0.));
        assert_eq!(bar.size.height, px(34.));
        for pane in ["explorer-pane", "stage-pane"] {
            assert_eq!(window.find(pane).bounds().top(), bar.bottom(), "{pane}");
        }
        // The explorer's own toolbar is gone.
        assert!(window.try_find("toolbar").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_bars_explorer_part_ends_where_the_explorer_does(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    let edges = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            (
                window.find("bar-explorer").bounds().right(),
                window.find("explorer-pane").bounds().right(),
            )
        })
        .unwrap()
    };
    let (bar, pane) = edges(cx);
    assert_eq!(bar, pane);
    drag_divider(cx, handle, 420.);
    let (bar, pane) = edges(cx);
    assert_eq!(bar, pane);
}

#[gpui_kit::test]
fn every_bar_button_is_a_labelled_icon(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for (id, label) in [
            ("sidebar", "Hide Sidebar"),
            ("add-folder", "Add Folder"),
            ("more", "More"),
            ("back", "Back"),
            ("forward", "Forward"),
            ("present", "Present"),
        ] {
            let button = window.find(id);
            assert_eq!(button.label(), Some(label), "{id}");
            assert_eq!(button.bounds().size.height, px(24.), "{id}");
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_sidebar_button_starts_after_the_lights_room(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    let left = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.find("sidebar").bounds().left()
        })
        .unwrap()
    };
    // The traffic lights' room: 80 px on macOS, the Kit's 12 elsewhere.
    let room = if cfg!(target_os = "macos") {
        px(80.)
    } else {
        px(12.)
    };
    assert_eq!(left(cx), room);
    // In fullscreen the room closes; the Kit's own inset is left.
    cx.update_window(handle, |_, window, _| window.toggle_fullscreen())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(left(cx), px(12.));
}

#[gpui_kit::test]
fn more_holds_the_explorers_commands(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    click(cx, handle, "more");
    let items = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // The Kit draws its separators as menu items with no name.
            window
                .within("popup-menu")
                .find_all_by_role(Role::MenuItem)
                .iter()
                .filter_map(|item| item.label().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(
        items,
        [
            "Content Search…",
            "Expand All",
            "Collapse All",
            "Show Ignored",
            "Refresh",
            "Select All",
            "Deselect All"
        ]
    );
}

/// Whether More's Show Ignored shows its check, in a menu opened afresh.
fn show_ignored_checked(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Option<bool> {
    click(cx, handle, "more");
    let checked = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .within("popup-menu")
                .find_by_label("Show Ignored")
                .checked()
        })
        .expect("read the menu");
    // Esc closes the menu; the next click on More builds it again.
    press(cx, handle, "escape");
    checked
}

#[gpui_kit::test]
fn more_checks_show_ignored_while_it_is_on(cx: &mut TestAppContext) {
    let dir = folder(&[
        (".gitignore", "secret.txt\n"),
        ("secret.txt", "s"),
        ("keep.txt", "k"),
    ]);
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    assert_eq!(show_ignored_checked(cx, handle), Some(false));

    dispatch(cx, handle, ToggleShowIgnored);
    assert!(read(cx, &view, AppState::shows_ignored));
    assert_eq!(show_ignored_checked(cx, handle), Some(true));
}

#[gpui_kit::test]
fn an_item_chosen_in_more_runs_its_command(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));
    click(cx, handle, "more");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let mut menu = window.within("popup-menu");
        // An item's id is its place in the menu; find it by its name.
        let id = menu
            .find_by_label("Expand All")
            .path()
            .last()
            .cloned()
            .expect("an item has an id");
        menu.click(id, cx);
    })
    .expect("choose Expand All");
    cx.run_until_parked();

    assert_eq!(
        rows_now(cx, handle),
        [
            "docs",
            "a.md",
            "b.md",
            "src",
            "app",
            "view.rs",
            "lib.rs",
            "main.rs",
            "file2.txt",
            "file10.txt",
            "README.md"
        ]
    );
    // Choosing closes the menu.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_title_names_what_the_stage_shows(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let readme = node(cx, &view, "README.md");

    // A blank stage: no title in the bar; the window takes the app's name.
    assert_eq!(bar_title(cx, handle), None);
    let window_title =
        |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).title().map(ToString::to_string));
    assert_eq!(window_title(cx).as_deref(), Some("Strikebrain Viewer"));

    click(cx, handle, row(readme));
    assert_eq!(bar_title(cx, handle).as_deref(), Some("README.md"));
    assert_eq!(window_title(cx).as_deref(), Some("README.md"));

    dispatch(cx, handle, SelectAllFiles);
    assert!(read(cx, &view, AppState::shows_gallery));
    assert_eq!(bar_title(cx, handle).as_deref(), Some("Gallery"));
    assert_eq!(window_title(cx).as_deref(), Some("Gallery"));
}

#[gpui_kit::test]
fn a_long_title_truncates_and_present_stays_in_the_window(cx: &mut TestAppContext) {
    // Some 1,900 px of name over a stage part 700 px wide.
    let name = format!("{}.txt", "a-very-long-name-for-a-file-".repeat(8));
    let (handle, view) = open_app(cx, tree(&[name.as_str(), "short.txt"]));
    let file = node(cx, &view, &name);
    click(cx, handle, row(file));
    assert_eq!(bar_title(cx, handle).as_deref(), Some(name.as_str()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let width = window.viewport_size().width;
        let title = window.find("stage-title").bounds();
        let present = window.find("present").bounds();
        assert!(
            present.right() <= width,
            "Present ends at {:?}, past the window's {width:?}",
            present.right()
        );
        assert!(
            title.right() <= present.left(),
            "the title ends at {:?}, past Present at {:?}",
            title.right(),
            present.left()
        );
        assert!(title.left() < title.right(), "the title keeps some room");
    })
    .unwrap();
}

#[gpui_kit::test]
fn present_starts_the_presentation_of_the_file_on_the_stage(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let readme = node(cx, &view, "README.md");
    click(cx, handle, row(readme));
    click(cx, handle, "present");
    assert!(read(cx, &view, AppState::is_presenting));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // No bar while presenting.
        assert!(window.try_find("title-bar").is_none());
    })
    .unwrap();
}
