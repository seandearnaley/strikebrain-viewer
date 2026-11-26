//! The window's title bar: one row above both panes, its part over the
//! explorer as wide as the explorer, its buttons, and the stage's title.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, AppContext as _, Role, TestAppContext, px};
use viewer::AppState;
use viewer::keys::SelectAllFiles;

use crate::{PROJECT, click, dispatch, drag_divider, node, open_app, read, row, tree};

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
            ("add-folder", "Add Folder"),
            ("more", "More"),
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
            "Expand All",
            "Collapse All",
            "Show Ignored",
            "Refresh",
            "Select All",
            "Deselect All"
        ]
    );
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
