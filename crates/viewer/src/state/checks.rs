//! Checks and the stage: toggling, Select all and Clear, the file on the
//! stage and moving through the gallery, and the presentation.

use explorer::NodeId;
use gpui_kit::Context;

use super::history::Record;
use super::{AppState, StateEvent};

impl AppState {
    /// Toggles a checkbox and applies the rules for `current`: checking a
    /// single file shows it; unchecking the file on the stage moves the stage
    /// to the next checked file, else the previous one, else nothing; an
    /// empty stage takes the first checked file.
    pub(super) fn toggle_check(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let before = self.current;
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
        self.checks_changed(before, cx);
    }

    /// Checks the file `id`, if it is not checked, and puts it on the
    /// stage as a check by hand does: a file opened with others.
    pub(super) fn check_and_show(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let before = self.current;
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        if !explorer.is_selected(id) {
            explorer.toggle_check(id);
        }
        self.current = Some(id);
        self.checks_changed(before, cx);
    }

    /// Checks every file the filter shows: Select All.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let before = self.current;
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        explorer.select_all();
        self.checks_changed(before, cx);
    }

    /// Unchecks every file, hidden ones included: Clear.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        let before = self.current;
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        let current_was_checked = self.current.is_some_and(|c| explorer.is_selected(c));
        explorer.clear_selection();
        self.leave_unchecked_current(current_was_checked);
        self.checks_changed(before, cx);
    }

    /// After the person changed the checks, the file on the stage having
    /// been `before`: the stage shows what they hold (see
    /// `state/gallery.rs`).
    fn checks_changed(&mut self, before: Option<NodeId>, cx: &mut Context<Self>) {
        self.selection_changed();
        self.show_for_checks(before, cx);
        self.record(Record::Add);
        cx.notify();
    }

    /// Shows `id` in the file view if it is a file, checked or not. Does
    /// nothing on a folder.
    pub(super) fn show(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(explorer) = &self.explorer else {
            return;
        };
        if explorer.tree().is_file(id) {
            self.current = Some(id);
            self.view = super::StageView::File;
            self.record(Record::Add);
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
            self.record(Record::Replace);
            cx.emit(StateEvent::CurrentMoved);
            cx.notify();
        }
    }

    /// Space or Enter on the stage: starts the presentation of the file on
    /// the stage, or from the gallery of its focused tile. Does nothing with
    /// no file there.
    pub fn start_presenting(&mut self, cx: &mut Context<Self>) {
        if self.presenting {
            return;
        }
        if self.shows_gallery() && self.tile.is_some() && self.tile != self.current {
            self.current = self.tile;
            // The tree keeps the presented file's row in view, as it does
            // when the stage's keys move the file.
            cx.emit(StateEvent::CurrentMoved);
        }
        if self.current.is_some() {
            self.presenting = true;
            cx.notify();
        }
    }

    /// Esc in the presentation: back to the explorer and the view it
    /// started from, on the same file; the gallery focused on its tile.
    pub fn stop_presenting(&mut self, cx: &mut Context<Self>) {
        if self.presenting {
            self.presenting = false;
            self.back_from_presenting(cx);
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

    /// After any change to the checks: recount, give an empty stage the
    /// first checked file, and keep the gallery's view and focus on what it
    /// holds now.
    pub(super) fn selection_changed(&mut self) {
        let Some(explorer) = &self.explorer else {
            self.selected = 0;
            self.settle_view();
            return;
        };
        self.selected = explorer.selection().count();
        if self.current.is_none() {
            self.current = explorer.gallery().next();
        }
        self.settle_view();
    }
}
