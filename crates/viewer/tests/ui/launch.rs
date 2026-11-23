//! Opening a folder: from the command line, the picker, and failures.

use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use viewer::Launch;

use crate::{fixture_folder, labels, open_app, stage};

#[gpui_kit::test]
fn a_folder_argument_scans_the_folder_and_names_the_window(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, Launch::Folder(fixture_folder()));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(labels(window), ["nested", "alpha.txt", "beta.md"]);
        assert_eq!(window.find("tree").label(), Some("fixture"));
        assert_eq!(window.find_all("size").len(), 3);
    })
    .unwrap();
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("fixture"));
}

#[gpui_kit::test]
fn an_unreadable_folder_shows_the_os_error(cx: &mut TestAppContext) {
    let missing = fixture_folder().join("no-such-folder");
    let (handle, _) = open_app(cx, Launch::Folder(missing));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let error = window.find("scan-error");
        assert!(error.visible());
        // The operating system's message alone, without Rust's error code.
        assert_eq!(error.label(), Some("No such file or directory"));
        assert!(window.try_find("tree").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn open_folder_and_command_o_show_the_folder_picker(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, Launch::Empty);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("tree").is_none());
        assert_eq!(stage(window), None);
        window.click("open-folder", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files && !options.multiple);
        Some(vec![fixture_folder()])
    });
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(labels(window), ["nested", "alpha.txt", "beta.md"]);
        window.press("cmd-o", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    // Cancelling the picker keeps the folder.
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    let title = cx.update(|cx| view.read(cx).title().cloned());
    assert_eq!(title.as_deref(), Some("fixture"));
}
