//! The menu bar's commands, dispatched by name as macOS does when an item
//! is chosen, and the window title as folders open and files show.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Action, AnyWindowHandle, AppContext as _, OwnedMenuItem, TestAppContext};
use viewer::{AppState, Launch};

use crate::{
    click, fixture_folder, folder, node, node_at, press, read, row, rows_now, state, under, write,
};

/// The action a menu item named `name` sends.
fn action(cx: &mut TestAppContext, name: &str) -> Box<dyn Action> {
    cx.update(|cx| cx.build_action(name, None))
        .expect("the menu's action is registered")
}

/// Chooses the menu item that sends `name`, as the platform does: on the
/// window, from the focused element.
fn choose(cx: &mut TestAppContext, handle: AnyWindowHandle, name: &str) {
    let action = action(cx, name);
    cx.update_window(handle, |_, window, cx| window.dispatch_action(action, cx))
        .expect("dispatch the menu's action");
    cx.run_until_parked();
}

/// Whether macOS would show the item that sends `name` enabled.
fn enabled(cx: &mut TestAppContext, handle: AnyWindowHandle, name: &str) -> bool {
    let action = action(cx, name);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.is_action_available(&*action, cx)
    })
    .expect("ask the window")
}

/// Whether the View menu's Show Ignored is checked.
fn show_ignored_checked(cx: &mut TestAppContext) -> bool {
    let menus = cx.update(|cx| cx.get_menus().unwrap_or_default());
    menus
        .iter()
        .flat_map(|menu| &menu.items)
        .find_map(|item| match item {
            OwnedMenuItem::Action { name, checked, .. } if name == "Show Ignored" => Some(*checked),
            _ => None,
        })
        .expect("the View menu has Show Ignored")
}

#[gpui_kit::test]
fn the_view_menu_shows_ignored_files_refreshes_and_presents(cx: &mut TestAppContext) {
    let dir = folder(&[
        (".gitignore", "secret.txt\n"),
        ("secret.txt", "s"),
        ("a.txt", "a"),
    ]);
    let (handle, view) = crate::open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    assert!(!show_ignored_checked(cx));

    choose(cx, handle, "viewer::ToggleShowIgnored");
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &[".gitignore", "a.txt", "secret.txt"])
    );
    assert!(show_ignored_checked(cx));
    choose(cx, handle, "viewer::ToggleShowIgnored");
    assert!(!show_ignored_checked(cx));

    write(dir.path(), "b.txt", "b");
    choose(cx, handle, "viewer::Refresh");
    assert_eq!(
        rows_now(cx, handle),
        under(dir.path(), &[".gitignore", "a.txt", "b.txt"])
    );

    // Present needs a file on the stage; from the tree it starts the
    // presentation with the stage focused.
    choose(cx, handle, "viewer::StartPresenting");
    assert!(!read(cx, &view, AppState::is_presenting));
    let a = node(cx, &view, "a.txt");
    click(cx, handle, row(a));
    assert!(enabled(cx, handle, "viewer::AddFolder"));
    choose(cx, handle, "viewer::StartPresenting");
    assert!(read(cx, &view, AppState::is_presenting));
    // While presenting, Add Folder is off in the menu bar as on ⌘O.
    assert!(!enabled(cx, handle, "viewer::AddFolder"));
    assert!(enabled(cx, handle, "viewer::Refresh"));
}

#[gpui_kit::test]
fn about_shows_the_name_and_version(cx: &mut TestAppContext) {
    let (handle, _) = crate::open_app(cx, Launch::Empty);
    assert!(!cx.has_pending_prompt());
    choose(cx, handle, "viewer::About");
    assert_eq!(
        cx.pending_prompt(),
        Some((
            "Strikebrain Viewer".to_owned(),
            format!("Version {}", env!("CARGO_PKG_VERSION"))
        ))
    );
    cx.simulate_prompt_answer("OK");
    assert!(!cx.has_pending_prompt());
}

#[gpui_kit::test]
fn the_title_names_the_file_on_the_stage_and_otherwise_the_app(cx: &mut TestAppContext) {
    let first = folder(&[("a.txt", "a")]);
    let (handle, view) = crate::open_app(cx, Launch::Folders(vec![first.path().to_owned()]));
    // The title the window shows, as the platform holds it, and the one
    // the view says it gave: the same.
    let title = |cx: &mut TestAppContext| {
        let shown = gpui_kit::VisualTestContext::from_window(handle, cx).window_title();
        let given = cx.update(|cx| view.read(cx).title().map(ToString::to_string));
        assert_eq!(shown, given, "the window shows the title the view gave it");
        shown.unwrap_or_default()
    };
    // Nothing on the stage: the app's name, however many folders are open.
    assert_eq!(title(cx), "Strikebrain Viewer");

    // File > Add Folder… adds one: two folders, the app's name.
    choose(cx, handle, "viewer::AddFolder");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![fixture_folder()]));
    cx.run_until_parked();
    assert_eq!(title(cx), "Strikebrain Viewer");

    // A folder the system hands over ("Open With", the Dock) is added too.
    let second = folder(&[("b.txt", "b")]);
    view.update(cx, |view, cx| {
        view.open_paths(vec![second.path().to_owned()], cx);
    });
    cx.run_until_parked();
    let rows = rows_now(cx, handle);
    assert_eq!(rows[..2], under(first.path(), &["a.txt"]));
    assert_eq!(rows[rows.len() - 2..], under(second.path(), &["b.txt"]));
    assert_eq!(title(cx), "Strikebrain Viewer");

    // Down to one folder again: still the app's name, until a file shows.
    for gone in [first.path().to_owned(), fixture_folder()] {
        let top = node_at(cx, &view, &gone);
        state(cx, &view).update(cx, |state, cx| state.remove_folder(top, cx));
    }
    cx.run_until_parked();
    assert_eq!(title(cx), "Strikebrain Viewer");
    let b = node_at(cx, &view, &second.path().join("b.txt"));
    click(cx, handle, row(b));
    assert_eq!(title(cx), "b.txt");
}

/// The text on the clipboard.
fn clipboard(cx: &TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui_kit::test]
fn copy_path_takes_the_focus_row_and_from_the_stage_its_file(cx: &mut TestAppContext) {
    let dir = folder(&[("a.txt", "a"), ("sub/b.txt", "b")]);
    let (handle, view) = crate::open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let root = explorer::canonical(dir.path()).unwrap();

    // From the tree: the focus row, here the folder itself.
    choose(cx, handle, "viewer::CopyPath");
    assert_eq!(clipboard(cx), Some(root.to_string_lossy().into_owned()));

    // A file shown from the tree is the focus row and the stage's file.
    let a = node(cx, &view, "a.txt");
    click(cx, handle, row(a));
    press(cx, handle, "secondary-alt-c");
    let a_path = root.join("a.txt").to_string_lossy().into_owned();
    assert_eq!(clipboard(cx), Some(a_path.clone()));

    // Focus moves up the tree, to the folder above; the stage keeps its
    // file. Each pane copies its own.
    press(cx, handle, "up");
    choose(cx, handle, "viewer::CopyPath");
    let sub = root.join("sub").to_string_lossy().into_owned();
    assert_eq!(clipboard(cx), Some(sub));
    press(cx, handle, "tab");
    choose(cx, handle, "viewer::CopyPath");
    assert_eq!(clipboard(cx), Some(a_path));
    // Reveal goes to the system; here it only has to be on offer.
    assert!(enabled(cx, handle, "viewer::Reveal"));
}

#[gpui_kit::test]
fn with_no_folder_there_is_no_path_to_copy(cx: &mut TestAppContext) {
    let (handle, view) = crate::open_app(cx, Launch::Empty);
    choose(cx, handle, "viewer::CopyPath");
    choose(cx, handle, "viewer::Reveal");
    assert_eq!(clipboard(cx), None);
    assert_eq!(read(cx, &view, |state| state.pointed(true)), None);
}

#[gpui_kit::test]
fn toggle_full_screen_fills_the_screen_and_comes_back(cx: &mut TestAppContext) {
    let (handle, _) = crate::open_app(cx, Launch::Empty);
    let fullscreen = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, _| window.is_fullscreen())
            .unwrap()
    };
    assert!(!fullscreen(cx));
    choose(cx, handle, "viewer::ToggleFullScreen");
    assert!(fullscreen(cx));
    choose(cx, handle, "viewer::ToggleFullScreen");
    assert!(!fullscreen(cx));
}
