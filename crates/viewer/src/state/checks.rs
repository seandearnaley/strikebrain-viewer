//! Checks and the stage: toggling, Select all and Clear, the file on the
//! stage and moving through the gallery, and the presentation.

use explorer::NodeId;
use gpui_kit::Context;

use super::{AppState, StateEvent};

impl AppState {
    /// Toggles a checkbox and applies the rules for `current`: checking a
    /// single file shows it; unchecking the file on the stage moves the stage
    /// to the next checked file, else the previous one, else nothing; an
    /// empty stage takes the first checked file.
    pub(super) fn toggle_check(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        let current_was_checked = self.current.is_some_and(|c| explorer.is_selected(c));
        explorer.toggle_check(id);

        if explorer.tree().is_file(id) && explorer.is_selected(id) {
            self.current = Some(id);
        } else {
            self.leave_unchecked_current(current_was_checked);
        }
        self.selection_changed();
        cx.notify();
    }

    /// Checks every file the filter shows: the toolbar's Select all.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        explorer.select_all();
        self.selection_changed();
        cx.notify();
    }

    /// Unchecks every file, hidden ones included: Clear.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        let current_was_checked = self.current.is_some_and(|c| explorer.is_selected(c));
        explorer.clear_selection();
        self.leave_unchecked_current(current_was_checked);
        self.selection_changed();
        cx.notify();
    }

    /// Shows `id` on the stage if it is a file, checked or not. Does
    /// nothing on a folder.
    pub(super) fn show(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(explorer) = &self.explorer else {
            return;
        };
        if explorer.tree().is_file(id) {
            self.current = Some(id);
            cx.notify();
        }
    }

    /// → on the stage: the next gallery file. From a file that is not
    /// checked, the first gallery file after it in display order. Stays
    /// put at the end of the gallery, and with nothing on the stage.
    pub fn show_next(&mut self, cx: &mut Context<Self>) {
        self.step(true, cx);
    }

    /// ← on the stage: the previous gallery file. From a file that is not
    /// checked, the last gallery file before it in display order.
    pub fn show_previous(&mut self, cx: &mut Context<Self>) {
        self.step(false, cx);
    }

    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let (Some(explorer), Some(current)) = (&self.explorer, self.current) else {
            return;
        };
        let to = if forward {
            explorer.gallery_next(current)
        } else {
            explorer.gallery_previous(current)
        };
        if let Some(to) = to {
            self.current = Some(to);
            cx.emit(StateEvent::CurrentMoved);
            cx.notify();
        }
    }

    /// Space or Enter on the stage: starts the presentation of the file on
    /// the stage. Does nothing with no file there.
    pub fn start_presenting(&mut self, cx: &mut Context<Self>) {
        if self.current.is_some() && !self.presenting {
            self.presenting = true;
            cx.notify();
        }
    }

    /// Esc in the presentation: back to the explorer and the stage, on the
    /// same file.
    pub fn stop_presenting(&mut self, cx: &mut Context<Self>) {
        if self.presenting {
            self.presenting = false;
            cx.notify();
        }
    }

    /// Space on the focus row.
    pub fn toggle_focus_check(&mut self, cx: &mut Context<Self>) {
        if let Some(focus) = self.focus {
            self.toggle_check(focus, cx);
        }
    }

    /// Enter on the focus row.
    pub fn show_focus(&mut self, cx: &mut Context<Self>) {
        if let Some(focus) = self.focus {
            self.show(focus, cx);
        }
    }

    /// After checks were removed: a `current` that was checked and no
    /// longer is moves to the next checked file, else the previous one,
    /// else nothing.
    fn leave_unchecked_current(&mut self, current_was_checked: bool) {
        let Some(explorer) = &self.explorer else {
            return;
        };
        if let Some(current) = self.current
            && current_was_checked
            && !explorer.is_selected(current)
        {
            self.current = explorer
                .gallery_next(current)
                .or_else(|| explorer.gallery_previous(current));
        }
    }

    /// After any change to the checks: recount, and give an empty stage the
    /// first checked file.
    pub(super) fn selection_changed(&mut self) {
        let Some(explorer) = &self.explorer else {
            self.selected = 0;
            return;
        };
        self.selected = explorer.selection().count();
        if self.current.is_none() {
            self.current = explorer.gallery().next();
        }
    }
}
