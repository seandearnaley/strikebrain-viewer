//! The menu bar: the application menu, File, Edit, View and Window, each
//! item an action the app already handles. Items whose action nothing on
//! the focused path handles show disabled, as macOS asks GPUI before it
//! draws a menu.
//!
//! An item's shortcut is the first key bound to its action. That is why
//! the View menu's Present is its own unbound action: `Present` is Space,
//! and a menu shortcut of Space would take the key from the filter input.

use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{App, Menu, MenuItem, OsAction, PromptLevel};

use crate::keys::{
    About, AddFolder, Minimize, Quit, Refresh, StartPresenting, ToggleShowIgnored, Zoom,
};

/// The name the menus show.
pub(crate) const APP_NAME: &str = "Strikebrain Viewer";

/// Sets the menu bar, with Show Ignored checked while ignored files show.
pub(crate) fn install(shows_ignored: bool, cx: &App) {
    cx.set_menus([
        Menu::new(APP_NAME).items([
            MenuItem::action(format!("About {APP_NAME}"), About),
            MenuItem::separator(),
            MenuItem::action(format!("Quit {APP_NAME}"), Quit),
        ]),
        Menu::new("File").items([MenuItem::action("Add Folder…", AddFolder)]),
        // The editing commands of the filter input and the code view, which
        // macOS also routes to its own panels, such as the folder picker.
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Present", StartPresenting),
            MenuItem::separator(),
            MenuItem::action("Show Ignored", ToggleShowIgnored).checked(shows_ignored),
            MenuItem::action("Refresh", Refresh),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
        ]),
    ]);
}

/// About: the application's name and version, in the system's alert.
pub(crate) fn about(_: &About, cx: &mut App) {
    // The app's window, even while another app is in front or it is
    // minimized.
    let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
        return;
    };
    let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
    // The action arrives while the window is being updated; the alert
    // waits until that is over.
    cx.defer(move |cx| {
        let shown = window.update(cx, |_, window, cx| {
            // The only answer is OK; nothing waits for it.
            drop(window.prompt(PromptLevel::Info, APP_NAME, Some(&version), &["OK"], cx));
        });
        // A window closed meanwhile shows nothing.
        shown.unwrap_or_default();
    });
}

#[cfg(test)]
mod tests {
    use gpui_kit::{OwnedMenuItem, TestAppContext};

    use super::install;

    /// Each menu's name and its items: name, then whether checked; a
    /// separator is "-".
    fn menus(cx: &mut TestAppContext) -> Vec<(String, Vec<(String, bool)>)> {
        cx.update(|cx| cx.get_menus().unwrap_or_default())
            .into_iter()
            .map(|menu| {
                let items = menu
                    .items
                    .iter()
                    .map(|item| match item {
                        OwnedMenuItem::Action { name, checked, .. } => (name.clone(), *checked),
                        OwnedMenuItem::Separator => ("-".to_owned(), false),
                        OwnedMenuItem::Submenu(menu) => (menu.name.to_string(), false),
                        OwnedMenuItem::SystemMenu(menu) => (menu.name.to_string(), false),
                    })
                    .collect();
                (menu.name.to_string(), items)
            })
            .collect()
    }

    #[gpui_kit::test]
    fn the_menu_bar_holds_the_app_file_edit_view_and_window_menus(cx: &mut TestAppContext) {
        cx.update(|cx| install(false, cx));
        let bar = menus(cx);
        let names: Vec<&str> = bar.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            ["Strikebrain Viewer", "File", "Edit", "View", "Window"]
        );
        let items =
            |ix: usize| -> Vec<&str> { bar[ix].1.iter().map(|(name, _)| name.as_str()).collect() };
        assert_eq!(
            items(0),
            ["About Strikebrain Viewer", "-", "Quit Strikebrain Viewer"]
        );
        assert_eq!(items(1), ["Add Folder…"]);
        assert_eq!(
            items(2),
            ["Undo", "Redo", "-", "Cut", "Copy", "Paste", "Select All"]
        );
        assert_eq!(items(3), ["Present", "-", "Show Ignored", "Refresh"]);
        assert_eq!(items(4), ["Minimize", "Zoom"]);
        assert!(!bar[3].1[2].1);

        cx.update(|cx| install(true, cx));
        assert!(menus(cx)[3].1[2].1, "Show Ignored is checked");
    }
}
