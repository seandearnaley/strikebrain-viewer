//! The stage's history, for Back and Forward: each file or gallery the
//! stage came to show because of something the person did.
//!
//! A step the person asks for is added and drops the steps ahead of it, as
//! in a browser. Moving with ← and → through the checked files replaces the
//! step, as Photos does, and so does a change the person did not make (a
//! rescan, a folder removed, a launch settling) that moves the file on the
//! stage. A blank stage is not a step. Back and Forward pass over steps
//! that cannot be shown now: a file no longer in the tree, and the gallery
//! while fewer than two files are checked. Kept for the session only.

use std::path::PathBuf;

use gpui_kit::Context;

use super::{AppState, StageView, StateEvent};

/// A step of the stage's history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// A file in the file view, by its path on disk.
    File(PathBuf),
    /// The gallery.
    Gallery,
}

/// How a change to what the stage shows enters the history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Record {
    /// Something the person asked to see: a new step.
    Add,
    /// The arrows, or a change the person did not make: the same step.
    Replace,
}

impl AppState {
    /// What the stage shows as a step; `None` while it is blank.
    fn step_now(&self) -> Option<Step> {
        if self.view == StageView::Gallery && self.gallery_len() >= 2 {
            return Some(Step::Gallery);
        }
        let explorer = self.explorer.as_ref()?;
        Some(Step::File(explorer.tree().abs_path(self.current?)))
    }

    /// Enters what the stage shows now into the history, `how` says how. A
    /// blank stage, or the step it already holds, changes nothing.
    pub(super) fn record(&mut self, how: Record) {
        let Some(step) = self.step_now() else {
            return;
        };
        if self.history.current() == Some(&step) {
            return;
        }
        match how {
            Record::Add => self.history.push(step),
            Record::Replace => self.history.replace_current(step),
        }
    }

    /// Whether `step` can be shown now.
    fn can_show(&self, step: &Step) -> bool {
        match step {
            Step::Gallery => self.gallery_len() >= 2,
            Step::File(path) => self.explorer.as_ref().is_some_and(|explorer| {
                let tree = explorer.tree();
                tree.find_abs(path).is_some_and(|id| tree.is_file(id))
            }),
        }
    }

    /// Whether Back has a step to show: one before the current step, or the
    /// current step itself while the stage is blank.
    #[must_use]
    pub fn can_go_back(&self) -> bool {
        let mut steps = self.history.entries().rev();
        let last = steps.next();
        (self.step_now().is_none() && last.is_some_and(|step| self.can_show(step)))
            || steps.any(|step| self.can_show(step))
    }

    /// Whether Forward has a step to show.
    #[must_use]
    pub fn can_go_forward(&self) -> bool {
        self.history
            .forward_entries()
            .any(|step| self.can_show(step))
    }

    /// Back: the nearest earlier step that can be shown; from a blank stage,
    /// the step last shown. Nothing while presenting.
    pub fn go_back(&mut self, cx: &mut Context<Self>) {
        if self.presenting {
            return;
        }
        if self.step_now().is_none()
            && let Some(step) = self.history.current().cloned()
            && self.can_show(&step)
        {
            return self.show_step(step, cx);
        }
        let mut moved = 0;
        while let Some(step) = self.history.back() {
            moved += 1;
            if self.can_show(&step) {
                return self.show_step(step, cx);
            }
        }
        // Nothing to show that way: the history stays where it was.
        for _ in 0..moved {
            self.history.forward();
        }
    }

    /// Forward: the nearest later step that can be shown. Nothing while
    /// presenting.
    pub fn go_forward(&mut self, cx: &mut Context<Self>) {
        if self.presenting {
            return;
        }
        let mut moved = 0;
        while let Some(step) = self.history.forward() {
            moved += 1;
            if self.can_show(&step) {
                return self.show_step(step, cx);
            }
        }
        for _ in 0..moved {
            self.history.back();
        }
    }

    /// Shows `step`, which can be shown, adding no step: a file in the file
    /// view, checked or not; the gallery with `current`'s tile focused.
    fn show_step(&mut self, step: Step, cx: &mut Context<Self>) {
        match step {
            Step::File(path) => {
                let id = self
                    .explorer
                    .as_ref()
                    .and_then(|explorer| explorer.tree().find_abs(&path));
                self.current = id;
                self.view = StageView::File;
                cx.emit(StateEvent::CurrentMoved);
            }
            Step::Gallery => {
                self.view = StageView::Gallery;
                self.set_tile(self.nearest_tile(self.current));
                cx.emit(StateEvent::RevealTile);
            }
        }
        cx.notify();
    }
}
