//! The menu bar's commands, dispatched by name as macOS does when an item
//! is chosen, and the window title as folders open.

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Action, AnyWindowHandle, AppContext as _, OwnedMenuItem, TestAppContext};
use viewer::{AppState, Launch};

use crate::{click, fixture_folder, folder, node, read, row, rows_now, write};

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
    let (handle, view) = crate::open_app(cx, Launch::Folder(dir.path().to_owned()));
    assert!(!show_ignored_checked(cx));

    choose(cx, handle, "viewer::ToggleShowIgnored");
    assert_eq!(rows_now(cx, handle), [".gitignore", "a.txt", "secret.txt"]);
    assert!(show_ignored_checked(cx));
    choose(cx, handle, "viewer::ToggleShowIgnored");
    assert!(!show_ignored_checked(cx));

    write(dir.path(), "b.txt", "b");
    choose(cx, handle, "viewer::Refresh");
    assert_eq!(rows_now(cx, handle), [".gitignore", "a.txt", "b.txt"]);

    // Present needs a file on the stage; from the tree it starts the
    // presentation with the stage focused.
    choose(cx, handle, "viewer::StartPresenting");
    assert!(!read(cx, &view, AppState::is_presenting));
    let a = node(cx, &view, "a.txt");
    click(cx, handle, row(a));
    assert!(enabled(cx, handle, "viewer::OpenFolder"));
    choose(cx, handle, "viewer::StartPresenting");
    assert!(read(cx, &view, AppState::is_presenting));
    // While presenting, Open Folder is off in the menu bar as on ⌘O.
    assert!(!enabled(cx, handle, "viewer::OpenFolder"));
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
fn the_title_follows_the_folder_opened(cx: &mut TestAppContext) {
    let first = folder(&[("a.txt", "a")]);
    let (handle, view) = crate::open_app(cx, Launch::Folder(first.path().to_owned()));
    let title = |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).title().cloned());
    let name = first
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(title(cx).as_deref(), Some(name.as_str()));

    choose(cx, handle, "viewer::OpenFolder");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![fixture_folder()]));
    cx.run_until_parked();
    assert_eq!(title(cx).as_deref(), Some("fixture"));

    // A folder the system hands over ("Open With", the Dock) opens too.
    let second = folder(&[("b.txt", "b")]);
    view.update(cx, |view, cx| {
        view.open_folder(second.path().to_owned(), cx);
    });
    cx.run_until_parked();
    let name = second
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(title(cx).as_deref(), Some(name.as_str()));
    assert_eq!(rows_now(cx, handle), ["b.txt"]);
}
