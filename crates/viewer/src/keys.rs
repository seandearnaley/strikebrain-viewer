//! Keyboard commands: GPUI actions and the keys bound to them.
//!
//! Tree commands bind in the [`EXPLORER`] key context, which the explorer's
//! tree declares while it holds focus. Opening a folder and quitting bind
//! everywhere.

use gpui_kit::{KeyBinding, actions};

/// The key context the explorer's tree declares.
pub const EXPLORER: &str = "Explorer";

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
    ]
);

actions!(
    viewer,
    [
        /// Opens the system folder picker.
        OpenFolder,
        /// Quits the application.
        Quit,
    ]
);

/// Every key binding of the application.
///
/// Learning exercise: Sean may rewrite this body; the tests below define it.
#[must_use]
pub fn bindings() -> Vec<KeyBinding> {
    let explorer = Some(EXPLORER);
    vec![
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
        KeyBinding::new("cmd-o", OpenFolder, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]
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
    fn command_o_and_command_q_work_everywhere() {
        assert_bound("cmd-o", None, &OpenFolder);
        assert_bound("cmd-q", None, &Quit);
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
