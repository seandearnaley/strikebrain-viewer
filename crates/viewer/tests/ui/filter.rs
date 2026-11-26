//! The filter input and the kind control.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AppContext as _, ElementId, Role, TestAppContext};
use viewer::AppState;
use viewer::keys::SelectAllFiles;

use crate::{
    PROJECT, dispatch, filter_value, is_focused, node, open_app, press, read, row, rows_now, tree,
    type_query,
};

/// Images, text, and a binary file that is neither.
const MIXED: &[&str] = &[
    "photos/beach.png",
    "photos/notes.md",
    "logo.svg",
    "readme.md",
    "tool.exe",
];

#[gpui_kit::test]
fn typing_a_query_filters_the_rows(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));

    type_query(cx, handle, "main");

    assert_eq!(rows_now(cx, handle), ["src", "main.rs"]);
    assert_eq!(filter_value(cx, handle), "main");
}

#[gpui_kit::test]
fn every_include_term_must_match(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));

    type_query(cx, handle, "file 2");

    assert_eq!(rows_now(cx, handle), ["file2.txt"]);
}

#[gpui_kit::test]
fn a_minus_term_excludes(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));

    type_query(cx, handle, "file -10");

    assert_eq!(rows_now(cx, handle), ["file2.txt"]);
}

#[gpui_kit::test]
fn an_exclude_matching_a_folder_removes_its_subtree(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));

    type_query(cx, handle, "-src");

    // src/main.rs and src/app/view.rs do not hold "src", but go with src.
    assert_eq!(
        rows_now(cx, handle),
        [
            "docs",
            "a.md",
            "b.md",
            "file2.txt",
            "file10.txt",
            "README.md"
        ]
    );
}

#[gpui_kit::test]
fn the_filter_finds_a_file_inside_a_closed_folder_and_shows_its_ancestors_open(
    cx: &mut TestAppContext,
) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let (src, app) = (node(cx, &view, "src"), node(cx, &view, "src/app"));

    type_query(cx, handle, "view");

    assert_eq!(rows_now(cx, handle), ["src", "app", "view.rs"]);
    cx.update_window(handle, |_, window, _| {
        assert_eq!(window.find(row(src)).expanded(), Some(true));
        assert_eq!(window.find(row(app)).expanded(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn clearing_the_query_restores_the_previous_expansion(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let docs = node(cx, &view, "docs");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(docs)).click("chevron", cx);
    })
    .unwrap();

    type_query(cx, handle, "view");
    assert_eq!(rows_now(cx, handle), ["src", "app", "view.rs"]);

    // The input's clear button.
    cx.update_window(handle, |_, window, cx| {
        window.within("filter").click("clean", cx);
    })
    .unwrap();
    cx.run_until_parked();

    assert_eq!(filter_value(cx, handle), "");
    assert_eq!(
        rows_now(cx, handle),
        [
            "docs",
            "a.md",
            "b.md",
            "src",
            "file2.txt",
            "file10.txt",
            "README.md"
        ]
    );
}

#[gpui_kit::test]
fn collapsing_a_revealed_folder_lasts_until_the_query_changes(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let app = node(cx, &view, "src/app");

    type_query(cx, handle, "vie");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(app)).click("chevron", cx);
    })
    .unwrap();
    assert_eq!(rows_now(cx, handle), ["src", "app"]);

    type_query(cx, handle, "w");
    assert_eq!(filter_value(cx, handle), "view");
    assert_eq!(rows_now(cx, handle), ["src", "app", "view.rs"]);
}

#[gpui_kit::test]
fn the_kind_control_shows_only_images_or_only_text(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(MIXED));
    let segment = |ix: usize| ElementId::from(ix);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find_by_label("All").selected(), Some(true));
        window.within("kind").click(segment(1), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(rows_now(cx, handle), ["photos", "beach.png", "logo.svg"]);

    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.find_by_label("Images").selected(), Some(true));
        window.within("kind").click(segment(2), cx);
    })
    .unwrap();
    cx.run_until_parked();
    // The executable is neither an image nor text.
    assert_eq!(rows_now(cx, handle), ["photos", "notes.md", "readme.md"]);

    cx.update_window(handle, |_, window, cx| {
        window.within("kind").click(segment(0), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        rows_now(cx, handle),
        ["photos", "logo.svg", "readme.md", "tool.exe"]
    );
}

#[gpui_kit::test]
fn a_click_on_a_kind_keeps_keyboard_focus_in_the_tree(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(MIXED));
    let notes = node(cx, &view, "photos/notes.md");

    cx.update_window(handle, |_, window, cx| {
        window.within("kind").click(ElementId::from(2usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(is_focused(cx, handle, "tree"));

    // Space reaches the tree: the focus row, photos, checks what Text shows.
    press(cx, handle, "space");
    assert_eq!(read(cx, &view, AppState::selected_count), 1);
    assert!(read(cx, &view, |state| state
        .explorer()
        .unwrap()
        .is_selected(notes)));
}

#[gpui_kit::test]
fn select_all_under_a_filter_selects_only_matches(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));

    type_query(cx, handle, "main");
    dispatch(cx, handle, SelectAllFiles);
    press(cx, handle, "escape");

    assert_eq!(read(cx, &view, AppState::selected_count), 1);
    let main = node(cx, &view, "src/main.rs");
    assert!(read(cx, &view, |state| state
        .explorer()
        .unwrap()
        .is_selected(main)));
}

#[gpui_kit::test]
fn unchecking_a_folder_under_a_filter_clears_hidden_files_too(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    let docs = node(cx, &view, "docs");
    cx.update_window(handle, |_, window, cx| {
        window.within(row(docs)).click("check", cx);
    })
    .unwrap();
    assert_eq!(read(cx, &view, AppState::selected_count), 2);

    type_query(cx, handle, "a.md");
    assert_eq!(rows_now(cx, handle), ["docs", "a.md"]);
    cx.update_window(handle, |_, window, cx| {
        // b.md is hidden, so docs counts only a.md: fully checked.
        assert_eq!(window.find(row(docs)).checked(), Some(true));
        window.within(row(docs)).click("check", cx);
    })
    .unwrap();
    press(cx, handle, "escape");

    assert_eq!(read(cx, &view, AppState::selected_count), 0);
}

#[gpui_kit::test]
fn a_stale_query_never_lands(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    // The rows each time the state changes.
    let seen: Rc<RefCell<Vec<Vec<String>>>> = Rc::default();
    let log = Rc::clone(&seen);
    let state = cx.update(|cx| view.read(cx).state().clone());
    let _observe = cx.update(|cx| {
        cx.observe(&state, move |state, cx| {
            let state = state.read(cx);
            let explorer = state.explorer().unwrap();
            let names = explorer
                .rows()
                .iter()
                .map(|&id| explorer.tree().name(id).to_owned())
                .collect();
            log.borrow_mut().push(names);
        })
    });

    // Two keystrokes, two filter jobs, and no job runs before both are
    // typed: "a", then "ap".
    cx.update_window(handle, |_, window, cx| {
        window.click("filter", cx);
        window.input("a", cx);
    })
    .unwrap();
    assert_eq!(read(cx, &view, |state| state.query().to_owned()), "a");
    cx.update_window(handle, |_, window, cx| window.input("p", cx))
        .unwrap();
    assert_eq!(read(cx, &view, |state| state.query().to_owned()), "ap");
    cx.run_until_parked();

    let only_ap = vec!["src".to_owned(), "app".to_owned()];
    assert_eq!(rows_now(cx, handle), only_ap);
    // "a" alone matches docs and a.md as well; no state ever showed it.
    assert!(!seen.borrow().is_empty());
    assert!(seen.borrow().iter().all(|rows| *rows == only_ap));
}

#[gpui_kit::test]
fn no_rows_after_filtering_leaves_the_tree_blank(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, tree(PROJECT));

    type_query(cx, handle, "nothing-matches");

    assert_eq!(rows_now(cx, handle), Vec::<String>::new());
    cx.update_window(handle, |_, window, _| {
        assert!(window.find("tree").visible());
        assert!(window.find_all_by_role(Role::TreeItem).is_empty());
        assert!(window.find_all_by_role(Role::Label).is_empty());
    })
    .unwrap();
}
