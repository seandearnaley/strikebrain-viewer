//! Bringing a saved view back, and taking the view out for saving.
//!
//! A saved view applies in two steps. Its filter and Show ignored apply
//! before the folder is scanned, so the first tree on screen is already
//! filtered and listed by the saved rules. The rest applies once that
//! scan lands: the sort, the open folders, the checks, the file on the
//! stage, and the row at the top of the tree's scroll. Under a Size sort
//! the rows reorder once sizes are read, so the top row waits for them.

use std::path::{Path, PathBuf};

use explorer::SortKey;
use gpui_kit::Context;

use super::{AppState, StateEvent};
use crate::persist::{FolderView, SnapshotView};

impl AppState {
    /// The filter and the rules of `view`: they apply from the first scan.
    pub(super) fn take_filter(&mut self, view: &FolderView) {
        self.query.clone_from(&view.query);
        self.kind = view.kind;
        self.show_ignored = view.show_ignored;
    }

    /// Applies the view waiting to be restored to the tree just put on
    /// screen. Paths that no longer exist are skipped; a file on the stage
    /// that is gone gives way to the first checked file.
    pub(super) fn restore_view(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.restore.take() else {
            return;
        };
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        explorer.set_sort(view.sort);
        explorer.restore_open(&view.expanded);
        explorer.restore_saved_selection(&view.selected);
        let tree = explorer.tree();
        self.current = view
            .current
            .as_deref()
            .and_then(|path| tree.find(path))
            .filter(|&id| tree.is_file(id));
        let waits_for_sizes = view.sort.key == SortKey::Size && !tree.has_sizes();
        self.selection_changed();
        if waits_for_sizes {
            self.restore_top = view.top_row;
            self.rows_replaced(cx);
        } else {
            self.show_saved_top(view.top_row.as_deref(), cx);
        }
    }

    /// Scrolls the saved top row to the top of the tree, with keyboard
    /// focus on it, and tells the views the rows were rebuilt.
    pub(super) fn show_saved_top(&mut self, top: Option<&str>, cx: &mut Context<Self>) {
        let found = self.explorer.as_ref().and_then(|explorer| {
            let id = explorer.tree().find(top?)?;
            Some((id, explorer.row_of(id)?))
        });
        if let Some((id, row)) = found {
            self.set_focus(id, row);
        }
        self.rows_replaced(cx);
        if let Some((_, row)) = found {
            cx.emit(StateEvent::TopRow(row));
        }
    }

    /// The folder to save: the scanned folder, or the one asked for while
    /// its first scan runs. `None` for a tree built in memory.
    pub(crate) fn saved_root(&self) -> Option<PathBuf> {
        let scanned = self
            .explorer
            .as_ref()
            .map(|explorer| explorer.tree().root_path())
            .filter(|root| !root.as_os_str().is_empty());
        scanned.map(Path::to_path_buf).or_else(|| self.root.clone())
    }

    /// The view to save, given the row at the top of the tree's scroll:
    /// the tree on screen, or the saved view still waiting for its scan.
    /// Cheap: the open folders and the checks are copied as bit sets and
    /// named later, off the UI thread.
    pub(crate) fn saved_view(&self, top_row: Option<usize>) -> Option<SnapshotView> {
        let Some(explorer) = &self.explorer else {
            return self.restore.clone().map(SnapshotView::Waiting);
        };
        let tree = explorer.tree();
        let top_row = self.restore_top.clone().or_else(|| {
            let id = *explorer.rows().get(top_row?)?;
            Some(tree.path(id))
        });
        let rest = FolderView {
            show_ignored: self.show_ignored,
            current: self.current.map(|id| tree.path(id)),
            query: self.query.clone(),
            kind: self.kind,
            sort: explorer.sort(),
            top_row,
            ..FolderView::default()
        };
        Some(SnapshotView::Live(explorer.snapshot(), rest))
    }
}
