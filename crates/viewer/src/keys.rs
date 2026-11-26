//! Keyboard commands: GPUI actions and the keys bound to them.
//!
//! Each pane declares a key context while it holds focus: the explorer's
//! tree `EXPLORER`, its filter input `FILTER` and the stage `STAGE`,
//! or `GALLERY` while the stage shows its grid of checked files.
//! Before a tree is on screen (no folder, or the first scans still
//! running) the explorer's body holds focus in the tree's place, in
//! `EXPLORER_BODY`; a folder that cannot be read is a row of the tree.
//! Tab moves between the panes from each of them. Adding a folder,
//! quitting and the other window commands bind everywhere.
//!
//! A command's modifier is ⌘ on macOS and Ctrl elsewhere: `secondary!`
//! writes the key for the platform the app is built for.
//!
//! The stage's code view is the Kit's editor, which takes focus on a click
//! so its text can be selected and copied. It declares the Kit's `Input`
//! context, whose arrows move a caret; `STAGE_EDITOR` binds the stage's
//! keys over them. Both match at the same depth, and these bindings are
//! added after the Kit's, so they win.
//!
//! While presenting, the stage declares `PRESENTATION` instead (and
//! `PRESENTATION_EDITOR` over its editor). Keys the presentation leaves
//! out are bound there to `NoAction`, which hides every binding they have
//! elsewhere: ⌘O, ⌘F, the sidebar's key, Back, Forward, Tab and Enter do
//! nothing until Esc. ⌘Q still quits.

use gpui_kit::{KeyBinding, NoAction, actions};

/// A key with the platform's command modifier: `secondary!("o")` is
/// `cmd-o` on macOS and `ctrl-o` elsewhere. `KeyBinding::new` takes the
/// modifier spelled out; GPUI's own `secondary-` needs an `App` to resolve.
#[cfg(target_os = "macos")]
macro_rules! secondary {
    ($key:literal) => {
        concat!("cmd-", $key)
    };
}
#[cfg(not(target_os = "macos"))]
macro_rules! secondary {
    ($key:literal) => {
        concat!("ctrl-", $key)
    };
}

/// The key context the explorer's tree declares.
pub(crate) const EXPLORER: &str = "Explorer";

/// The key context around the explorer's filter input.
pub(crate) const FILTER: &str = "Filter";

/// The key context the stage declares while it shows one file.
pub(crate) const STAGE: &str = "Stage";

/// The key context the stage declares while it shows the gallery.
pub(crate) const GALLERY: &str = "Gallery";

/// The key context of the explorer's body while no tree is on screen.
pub(crate) const EXPLORER_BODY: &str = "ExplorerBody";

/// The Kit's editor on the stage, while it holds focus.
pub(crate) const STAGE_EDITOR: &str = "Stage > Input";

/// The key context the stage declares while presenting.
pub(crate) const PRESENTATION: &str = "Presentation";

/// The Kit's editor on the stage while presenting, while it holds focus.
pub(crate) const PRESENTATION_EDITOR: &str = "Presentation > Input";

actions!(
    explorer,
    [
        /// Moves keyboard focus one row up.
        FocusPrevious,
        /// Moves keyboard focus one row down.
        FocusNext,
        /// Moves keyboard focus to the first row.
        FocusFirst,
        /// Moves keyboard focus to the last row.
        FocusLast,
        /// Moves keyboard focus one screen of rows up.
        FocusPreviousPage,
        /// Moves keyboard focus one screen of rows down.
        FocusNextPage,
        /// Opens the focused folder; on an open folder, focuses its first child.
        Expand,
        /// Closes the focused folder; otherwise focuses the parent folder.
        Collapse,
        /// Toggles the focused row's checkbox.
        ToggleCheck,
        /// Shows the focused file on the stage. Does nothing on a folder.
        ShowFile,
        /// Clears the filter query, from the tree.
        ClearFilter,
        /// Esc in the filter input: clears a query, or with none moves focus
        /// to the tree.
        CancelFilter,
        /// Removes the focused top-level row: a folder, or a file opened on
        /// its own.
        RemoveFolder,
    ]
);

actions!(
    viewer,
    [
        /// Opens the system folder picker and adds the folders chosen.
        AddFolder,
        /// Quits the application.
        Quit,
        /// Moves keyboard focus to the next pane: filter, tree, stage.
        NextPane,
        /// Moves keyboard focus to the previous pane.
        PreviousPane,
        /// Moves keyboard focus to the filter input.
        FocusFilter,
        /// Moves keyboard focus to the explorer's tree.
        FocusExplorer,
        /// Shows the application's name and version.
        About,
        /// Starts the presentation, from the menu bar. Unbound: `Present`
        /// is Space, and a menu item shows its action's key as a shortcut
        /// that the menu would then take from the filter input.
        StartPresenting,
        /// Shows or hides the files the ignore rules leave out.
        ToggleShowIgnored,
        /// Rescans every folder.
        Refresh,
        /// Minimizes the window.
        Minimize,
        /// Zooms the window to fill the screen, or back.
        Zoom,
        /// Closes the window, which quits.
        CloseWindow,
        /// Makes the window fullscreen, or windowed again.
        ToggleFullScreen,
        /// Hides the application. macOS.
        Hide,
        /// Hides every other application. macOS.
        HideOthers,
        /// Shows every hidden application. macOS.
        ShowAll,
        /// Shows the file in the system's file manager: the file on the
        /// stage from the stage, the focus row from the explorer.
        Reveal,
        /// Copies that file's or folder's path.
        CopyPath,
        /// Opens every folder: the bar's More menu.
        ExpandAll,
        /// Closes every folder.
        CollapseAll,
        /// Checks every file the filter shows.
        SelectAllFiles,
        /// Unchecks every file, hidden ones included.
        DeselectAll,
        /// Hides the explorer, or shows it again.
        ToggleSidebar,
        /// Shows what the stage showed before.
        GoBack,
        /// Shows again what Back left.
        GoForward,
    ]
);

actions!(
    stage,
    [
        /// Shows the next file in the gallery.
        NextFile,
        /// Shows the previous file in the gallery.
        PreviousFile,
        /// Scrolls the text on the stage up a step.
        ScrollUp,
        /// Scrolls the text on the stage down a step.
        ScrollDown,
        /// Scrolls the text on the stage up a screen.
        ScrollPageUp,
        /// Scrolls the text on the stage down a screen.
        ScrollPageDown,
        /// Scrolls the text on the stage to its start.
        ScrollToTop,
        /// Scrolls the text on the stage to its end.
        ScrollToBottom,
        /// Starts the presentation of the file on the stage.
        Present,
        /// Ends the presentation.
        LeavePresentation,
        /// Esc in the file view: back to the gallery with two or more files
        /// checked, otherwise to the explorer.
        ShowGallery,
    ]
);

actions!(
    gallery,
    [
        /// Moves tile focus one tile back: from a row's first tile to the
        /// end of the row above.
        FocusPreviousTile,
        /// Moves tile focus one tile on: from a row's last tile to the
        /// start of the row below.
        FocusNextTile,
        /// Moves tile focus one row up.
        FocusTileAbove,
        /// Moves tile focus one row down, or to the last tile when the row
        /// below is too short to have one under it.
        FocusTileBelow,
        /// Moves tile focus to the first tile.
        FocusFirstTile,
        /// Moves tile focus to the last tile.
        FocusLastTile,
        /// Moves tile focus a screen of rows up.
        FocusTilePageUp,
        /// Moves tile focus a screen of rows down.
        FocusTilePageDown,
        /// Opens the focused tile's file in the file view.
        OpenTile,
    ]
);

/// The key that toggles fullscreen: the system's own on each platform.
const FULL_SCREEN: &str = if cfg!(target_os = "macos") {
    "ctrl-cmd-f"
} else {
    "f11"
};

/// The key that hides and shows the sidebar: Finder's and Notes' on macOS,
/// the usual Ctrl+B elsewhere.
pub(crate) const SIDEBAR: &str = if cfg!(target_os = "macos") {
    "ctrl-cmd-s"
} else {
    "ctrl-b"
};

/// Back, as Finder and Safari have it on macOS, and as browsers have it
/// elsewhere. The Kit's text input binds `cmd-[` to Outdent but passes it
/// on when it is one line or read-only, as the filter and the code view
/// are, and GPUI then tries the next binding.
pub(crate) const BACK: &str = if cfg!(target_os = "macos") {
    "cmd-["
} else {
    "alt-left"
};

/// Forward: see `BACK`.
pub(crate) const FORWARD: &str = if cfg!(target_os = "macos") {
    "cmd-]"
} else {
    "alt-right"
};

/// Every key binding of the application.
///
/// Learning exercise: Sean may rewrite this body; the tests below define it.
#[must_use]
pub(crate) fn bindings() -> Vec<KeyBinding> {
    let (explorer, filter, stage) = (Some(EXPLORER), Some(FILTER), Some(STAGE));
    let (body, gallery) = (Some(EXPLORER_BODY), Some(GALLERY));
    let mut bindings = vec![
        KeyBinding::new("up", FocusPrevious, explorer),
        KeyBinding::new("down", FocusNext, explorer),
        KeyBinding::new("home", FocusFirst, explorer),
        KeyBinding::new("end", FocusLast, explorer),
        KeyBinding::new("pageup", FocusPreviousPage, explorer),
        KeyBinding::new("pagedown", FocusNextPage, explorer),
        KeyBinding::new("right", Expand, explorer),
        KeyBinding::new("left", Collapse, explorer),
        KeyBinding::new("space", ToggleCheck, explorer),
        KeyBinding::new("enter", ShowFile, explorer),
        KeyBinding::new("escape", ClearFilter, explorer),
        KeyBinding::new(secondary!("backspace"), RemoveFolder, explorer),
        KeyBinding::new(secondary!("f"), FocusFilter, explorer),
        KeyBinding::new("tab", NextPane, explorer),
        KeyBinding::new("shift-tab", PreviousPane, explorer),
        KeyBinding::new("escape", CancelFilter, filter),
        KeyBinding::new("tab", NextPane, filter),
        KeyBinding::new("shift-tab", PreviousPane, filter),
        KeyBinding::new(secondary!("f"), FocusFilter, body),
        KeyBinding::new("tab", NextPane, body),
        KeyBinding::new("shift-tab", PreviousPane, body),
        KeyBinding::new(secondary!("o"), AddFolder, None),
        KeyBinding::new(secondary!("q"), Quit, None),
        KeyBinding::new(secondary!("w"), CloseWindow, None),
        KeyBinding::new(secondary!("r"), Refresh, None),
        // Finder's key for its hidden files.
        KeyBinding::new(secondary!("shift-."), ToggleShowIgnored, None),
        KeyBinding::new(secondary!("shift-r"), Reveal, None),
        KeyBinding::new(secondary!("alt-c"), CopyPath, None),
        KeyBinding::new(FULL_SCREEN, ToggleFullScreen, None),
        KeyBinding::new(SIDEBAR, ToggleSidebar, None),
        KeyBinding::new(BACK, GoBack, None),
        KeyBinding::new(FORWARD, GoForward, None),
    ];
    // Keys only macOS has a convention for.
    if cfg!(target_os = "macos") {
        bindings.extend([
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
        ]);
    }
    // The stage's keys, on the stage and over the editor inside it.
    for stage in [stage, Some(STAGE_EDITOR)] {
        bindings.extend([
            KeyBinding::new("right", NextFile, stage),
            KeyBinding::new("left", PreviousFile, stage),
            KeyBinding::new("up", ScrollUp, stage),
            KeyBinding::new("down", ScrollDown, stage),
            KeyBinding::new("pageup", ScrollPageUp, stage),
            KeyBinding::new("pagedown", ScrollPageDown, stage),
            KeyBinding::new("home", ScrollToTop, stage),
            KeyBinding::new("end", ScrollToBottom, stage),
            KeyBinding::new("space", Present, stage),
            KeyBinding::new("enter", Present, stage),
            KeyBinding::new("escape", ShowGallery, stage),
            KeyBinding::new(secondary!("f"), FocusFilter, stage),
            KeyBinding::new("tab", NextPane, stage),
            KeyBinding::new("shift-tab", PreviousPane, stage),
        ]);
    }
    // The gallery's keys: focus moves over the tiles.
    bindings.extend([
        KeyBinding::new("left", FocusPreviousTile, gallery),
        KeyBinding::new("right", FocusNextTile, gallery),
        KeyBinding::new("up", FocusTileAbove, gallery),
        KeyBinding::new("down", FocusTileBelow, gallery),
        KeyBinding::new("home", FocusFirstTile, gallery),
        KeyBinding::new("end", FocusLastTile, gallery),
        KeyBinding::new("pageup", FocusTilePageUp, gallery),
        KeyBinding::new("pagedown", FocusTilePageDown, gallery),
        KeyBinding::new("enter", OpenTile, gallery),
        KeyBinding::new("space", Present, gallery),
        KeyBinding::new("escape", FocusExplorer, gallery),
        KeyBinding::new(secondary!("f"), FocusFilter, gallery),
        KeyBinding::new("tab", NextPane, gallery),
        KeyBinding::new("shift-tab", PreviousPane, gallery),
    ]);
    // The presentation's keys, likewise. Space moves on, as in a slideshow.
    for presentation in [Some(PRESENTATION), Some(PRESENTATION_EDITOR)] {
        bindings.extend([
            KeyBinding::new("right", NextFile, presentation),
            KeyBinding::new("space", NextFile, presentation),
            KeyBinding::new("left", PreviousFile, presentation),
            KeyBinding::new("up", ScrollUp, presentation),
            KeyBinding::new("down", ScrollDown, presentation),
            KeyBinding::new("pageup", ScrollPageUp, presentation),
            KeyBinding::new("pagedown", ScrollPageDown, presentation),
            KeyBinding::new("home", ScrollToTop, presentation),
            KeyBinding::new("end", ScrollToBottom, presentation),
            KeyBinding::new("escape", LeavePresentation, presentation),
            KeyBinding::new("enter", NoAction, presentation),
            KeyBinding::new(secondary!("f"), NoAction, presentation),
            KeyBinding::new(secondary!("o"), NoAction, presentation),
            KeyBinding::new("tab", NoAction, presentation),
            KeyBinding::new("shift-tab", NoAction, presentation),
            KeyBinding::new(SIDEBAR, NoAction, presentation),
            KeyBinding::new(BACK, NoAction, presentation),
            KeyBinding::new(FORWARD, NoAction, presentation),
        ]);
    }
    bindings
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Action, Keystroke};

    /// The binding for exactly `key` in `context`, if any.
    fn binding(key: &'static str, context: Option<&str>) -> Option<KeyBinding> {
        let typed = [Keystroke::parse(key).unwrap()];
        bindings().into_iter().find(|binding| {
            binding.match_keystrokes(&typed) == Some(false)
                && binding.predicate().map(|predicate| predicate.to_string())
                    == context.map(str::to_owned)
        })
    }

    fn assert_bound(key: &'static str, context: Option<&str>, action: &dyn Action) {
        let binding =
            binding(key, context).unwrap_or_else(|| panic!("{key} is not bound in {context:?}"));
        assert!(
            binding.action().partial_eq(action),
            "{key} in {context:?} is bound to {}, not {}",
            binding.action().name(),
            action.name()
        );
    }

    #[test]
    fn arrows_move_focus_and_open_or_close_folders_in_the_explorer() {
        assert_bound("up", Some(EXPLORER), &FocusPrevious);
        assert_bound("down", Some(EXPLORER), &FocusNext);
        assert_bound("right", Some(EXPLORER), &Expand);
        assert_bound("left", Some(EXPLORER), &Collapse);
    }

    #[test]
    fn home_end_and_page_keys_jump_in_the_explorer() {
        assert_bound("home", Some(EXPLORER), &FocusFirst);
        assert_bound("end", Some(EXPLORER), &FocusLast);
        assert_bound("pageup", Some(EXPLORER), &FocusPreviousPage);
        assert_bound("pagedown", Some(EXPLORER), &FocusNextPage);
    }

    #[test]
    fn space_checks_and_enter_shows_in_the_explorer() {
        assert_bound("space", Some(EXPLORER), &ToggleCheck);
        assert_bound("enter", Some(EXPLORER), &ShowFile);
    }

    #[test]
    fn escape_clears_the_filter_or_leaves_it_and_command_f_finds_it() {
        assert_bound("escape", Some(EXPLORER), &ClearFilter);
        assert_bound("escape", Some(FILTER), &CancelFilter);
        assert_bound("escape", Some(STAGE), &ShowGallery);
        assert_bound("escape", Some(GALLERY), &FocusExplorer);
        assert_bound(secondary!("f"), Some(EXPLORER), &FocusFilter);
        assert_bound(secondary!("f"), Some(STAGE), &FocusFilter);
        assert_bound(secondary!("f"), Some(EXPLORER_BODY), &FocusFilter);
    }

    #[test]
    fn the_stage_moves_between_files_scrolls_and_presents_over_its_editor_too() {
        for context in [STAGE, STAGE_EDITOR] {
            assert_bound("right", Some(context), &NextFile);
            assert_bound("left", Some(context), &PreviousFile);
            assert_bound("up", Some(context), &ScrollUp);
            assert_bound("down", Some(context), &ScrollDown);
            assert_bound("pageup", Some(context), &ScrollPageUp);
            assert_bound("pagedown", Some(context), &ScrollPageDown);
            assert_bound("home", Some(context), &ScrollToTop);
            assert_bound("end", Some(context), &ScrollToBottom);
            assert_bound("space", Some(context), &Present);
            assert_bound("enter", Some(context), &Present);
            assert_bound("escape", Some(context), &ShowGallery);
            assert_bound(secondary!("f"), Some(context), &FocusFilter);
        }
    }

    #[test]
    fn the_gallery_moves_tile_focus_opens_and_presents() {
        let gallery = Some(GALLERY);
        assert_bound("left", gallery, &FocusPreviousTile);
        assert_bound("right", gallery, &FocusNextTile);
        assert_bound("up", gallery, &FocusTileAbove);
        assert_bound("down", gallery, &FocusTileBelow);
        assert_bound("home", gallery, &FocusFirstTile);
        assert_bound("end", gallery, &FocusLastTile);
        assert_bound("pageup", gallery, &FocusTilePageUp);
        assert_bound("pagedown", gallery, &FocusTilePageDown);
        assert_bound("enter", gallery, &OpenTile);
        assert_bound("space", gallery, &Present);
        assert_bound("escape", gallery, &FocusExplorer);
        assert_bound(secondary!("f"), gallery, &FocusFilter);
    }

    #[test]
    fn the_presentation_moves_scrolls_and_leaves_over_its_editor_too() {
        for context in [PRESENTATION, PRESENTATION_EDITOR] {
            assert_bound("right", Some(context), &NextFile);
            assert_bound("space", Some(context), &NextFile);
            assert_bound("left", Some(context), &PreviousFile);
            assert_bound("up", Some(context), &ScrollUp);
            assert_bound("down", Some(context), &ScrollDown);
            assert_bound("pageup", Some(context), &ScrollPageUp);
            assert_bound("pagedown", Some(context), &ScrollPageDown);
            assert_bound("home", Some(context), &ScrollToTop);
            assert_bound("end", Some(context), &ScrollToBottom);
            assert_bound("escape", Some(context), &LeavePresentation);
        }
    }

    #[test]
    fn the_presentation_hides_the_keys_it_leaves_out_but_not_quit() {
        for context in [PRESENTATION, PRESENTATION_EDITOR] {
            for key in [
                "enter",
                secondary!("f"),
                secondary!("o"),
                "tab",
                "shift-tab",
                SIDEBAR,
                BACK,
                FORWARD,
            ] {
                assert_bound(key, Some(context), &NoAction);
            }
            assert!(binding(secondary!("q"), Some(context)).is_none());
        }
    }

    #[test]
    fn tab_and_shift_tab_move_between_panes_from_each_pane() {
        for context in [
            EXPLORER,
            FILTER,
            STAGE,
            STAGE_EDITOR,
            GALLERY,
            EXPLORER_BODY,
        ] {
            assert_bound("tab", Some(context), &NextPane);
            assert_bound("shift-tab", Some(context), &PreviousPane);
        }
    }

    #[test]
    fn command_backspace_removes_a_folder_in_the_explorer() {
        assert_bound(secondary!("backspace"), Some(EXPLORER), &RemoveFolder);
        assert!(binding(secondary!("backspace"), Some(STAGE)).is_none());
    }

    #[test]
    fn the_window_commands_work_everywhere() {
        assert_bound(secondary!("o"), None, &AddFolder);
        assert_bound(secondary!("q"), None, &Quit);
        assert_bound(secondary!("w"), None, &CloseWindow);
        assert_bound(secondary!("r"), None, &Refresh);
        assert_bound(secondary!("shift-."), None, &ToggleShowIgnored);
        assert_bound(secondary!("shift-r"), None, &Reveal);
        assert_bound(secondary!("alt-c"), None, &CopyPath);
        assert_bound(FULL_SCREEN, None, &ToggleFullScreen);
        assert_bound(SIDEBAR, None, &ToggleSidebar);
        assert_bound(BACK, None, &GoBack);
        assert_bound(FORWARD, None, &GoForward);
    }

    #[test]
    fn a_command_takes_the_platforms_modifier() {
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        assert_eq!(secondary!("o"), format!("{modifier}-o"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn minimize_and_hide_take_their_mac_keys() {
        assert_bound("cmd-m", None, &Minimize);
        assert_bound("cmd-h", None, &Hide);
        assert_bound("alt-cmd-h", None, &HideOthers);
    }

    #[test]
    fn the_menu_only_actions_have_no_key() {
        let actions: [&dyn Action; 8] = [
            &About,
            &StartPresenting,
            &Zoom,
            &ShowAll,
            &ExpandAll,
            &CollapseAll,
            &SelectAllFiles,
            &DeselectAll,
        ];
        for action in actions {
            assert!(
                bindings()
                    .iter()
                    .all(|binding| !binding.action().partial_eq(action)),
                "{} is bound",
                action.name()
            );
        }
    }

    #[test]
    fn no_key_is_bound_twice_in_one_context() {
        let all = bindings();
        for (ix, a) in all.iter().enumerate() {
            for b in &all[ix + 1..] {
                let same_keys = a.keystrokes() == b.keystrokes();
                let same_context =
                    a.predicate().map(|p| p.to_string()) == b.predicate().map(|p| p.to_string());
                assert!(
                    !(same_keys && same_context),
                    "{} and {} share a key and a context",
                    a.action().name(),
                    b.action().name()
                );
            }
        }
    }
}
