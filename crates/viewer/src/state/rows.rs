//! Moving over the rows: pointer presses on drawn rows, keyboard focus,
//! opening and closing folders, and the sort.

use explorer::{NodeId, SortKey};
use gpui_kit::Context;

use super::{AppState, DrawnRow, Motion, RowAction, StateEvent};

impl AppState {
    /// The row at `ix`, showing `id`, as the frame being drawn shows it.
    pub(crate) fn drawn_row(&self, ix: usize, id: NodeId) -> DrawnRow {
        DrawnRow {
            tree: self.tree_generation,
            ix,
            id,
        }
    }

    /// A press on a drawn row: focuses the row, then does `action` on it.
    /// Does nothing and returns false when the row is no longer on screen
    /// as it was drawn: the tree was replaced, or the rows changed under it.
    pub(crate) fn row_action(
        &mut self,
        row: DrawnRow,
        action: RowAction,
        cx: &mut Context<Self>,
    ) -> bool {
        let still_drawn = row.tree == self.tree_generation
            && self
                .explorer
                .as_ref()
                .is_some_and(|explorer| explorer.rows().get(row.ix) == Some(&row.id));
        if !still_drawn {
            return false;
        }
        self.set_focus(row.id, row.ix);
        match action {
            RowAction::Show => self.show(row.id, cx),
            RowAction::ToggleCheck => self.toggle_check(row.id, cx),
            RowAction::ToggleOpen => self.toggle_folder(row.id, cx),
            RowAction::Remove => self.remove_folder(row.id, cx),
        }
        cx.notify();
        true
    }

    /// Moves keyboard focus; returns the new focus row.
    pub(crate) fn move_focus(&mut self, motion: Motion, cx: &mut Context<Self>) -> Option<usize> {
        let explorer = self.explorer.as_ref()?;
        let last = explorer.rows().len().checked_sub(1)?;
        let from = self.focus_index().unwrap_or(0);
        let to = match motion {
            Motion::Up(n) => from.saturating_sub(n),
            Motion::Down(n) => from.saturating_add(n).min(last),
            Motion::First => 0,
            Motion::Last => last,
        };
        let id = explorer.rows()[to];
        self.set_focus(id, to);
        cx.notify();
        Some(to)
    }

    /// → on the focus row: opens a closed folder, or focuses the first
    /// child of an open one. Returns the new focus row.
    pub fn expand(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        let row = self.focus_index()?;
        let explorer = self.explorer.as_mut()?;
        let id = explorer.rows()[row];
        if !explorer.can_open(id) {
            return Some(row);
        }
        if !explorer.is_open(id) {
            explorer.set_open(id, true);
            cx.notify();
            return Some(row);
        }
        let child = explorer.rows().get(row + 1).copied();
        match child {
            Some(child) if explorer.tree().parent(child) == Some(id) => {
                self.set_focus(child, row + 1);
                cx.notify();
                Some(row + 1)
            }
            _ => Some(row),
        }
    }

    /// ← on the focus row: closes an open folder, or focuses the parent
    /// folder. Returns the new focus row.
    pub fn collapse(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        let row = self.focus_index()?;
        let explorer = self.explorer.as_mut()?;
        let id = explorer.rows()[row];
        if explorer.can_open(id) && explorer.is_open(id) {
            explorer.set_open(id, false);
            cx.notify();
            return Some(row);
        }
        let parent = explorer.tree().parent(id).filter(|&p| p != NodeId::ROOT);
        let Some((parent, parent_row)) =
            parent.and_then(|parent| Some((parent, explorer.row_of(parent)?)))
        else {
            return Some(row);
        };
        self.set_focus(parent, parent_row);
        cx.notify();
        Some(parent_row)
    }

    /// Opens or closes a folder: its chevron. Focus is already on it, and
    /// the rows above it do not move.
    fn toggle_folder(&mut self, id: NodeId, cx: &mut Context<Self>) {
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.toggle_folder(id);
            cx.notify();
        }
    }

    /// Opens every folder: the toolbar's Expand all.
    pub fn expand_all(&mut self, cx: &mut Context<Self>) {
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.expand_all();
            self.rows_replaced(cx);
        }
    }

    /// Closes every folder: the toolbar's Collapse all. Focus inside a
    /// folder moves to its top-level folder.
    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.collapse_all();
            self.rows_replaced(cx);
        }
    }

    /// A click on a column header: sorts by `key`, or flips the direction
    /// when the tree is already sorted by it.
    pub fn sort_by(&mut self, key: SortKey, cx: &mut Context<Self>) {
        if let Some(explorer) = self.explorer.as_mut() {
            let sort = explorer.sort().clicked(key);
            explorer.set_sort(sort);
            self.sort = sort;
            self.rows_replaced(cx);
        }
    }

    /// After a command rebuilt the rows: keep focus on a row and tell the
    /// views.
    pub(super) fn rows_replaced(&mut self, cx: &mut Context<Self>) {
        self.settle_focus();
        cx.emit(StateEvent::RowsReplaced);
        cx.notify();
    }

    /// Keeps `focus` on a row on screen after the rows changed. A focus row
    /// that is gone moves to its deepest folder still on screen, else to
    /// the first row. One pass over the rows, stopping at the focus row.
    pub(super) fn settle_focus(&mut self) {
        let Some(explorer) = &self.explorer else {
            return;
        };
        let rows = explorer.rows();
        let Some(focus) = self.focus else {
            self.focus = rows.first().copied();
            self.focus_row = 0;
            return;
        };
        if rows.get(self.focus_row) == Some(&focus) {
            return;
        }
        let tree = explorer.tree();
        let mut nearest = None;
        for (row, &id) in rows.iter().enumerate() {
            if id == focus {
                self.focus_row = row;
                return;
            }
            if tree.subtree(id).contains(&focus.index()) {
                nearest = Some((row, id));
            }
        }
        if let Some((row, id)) = nearest {
            self.set_focus(id, row);
        } else {
            self.focus = rows.first().copied();
            self.focus_row = 0;
        }
    }
}
