//! Bringing a saved view back, and taking the view out for saving.
//!
//! A saved view applies in two steps. Its filter and Show ignored apply
//! before the folder is scanned, so the first tree on screen is already
//! filtered and listed by the saved rules. The rest applies once that
//! scan lands: the sort, the open folders, the checks, the file on the
//! stage, and the row at the top of the tree's scroll. Under a Size sort
//! the rows reorder once sizes are read, so the top row waits for them.

use std::path::PathBuf;

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

    /// The folder to save: the scanned folder; with no tree on screen, the
    /// one asked for while its saved view waits for it, else the one kept
    /// (see `kept`), else the one asked for. `None` for a tree built in
    /// memory.
    pub(super) fn saved_root(&self) -> Option<PathBuf> {
        if let Some(explorer) = &self.explorer {
            let root = explorer.tree().root_path();
            return (!root.as_os_str().is_empty()).then(|| root.to_path_buf());
        }
        match &self.kept {
            Some((root, _)) if self.restore.is_none() => Some(root.clone()),
            _ => self.root.clone(),
        }
    }

    /// The folder and its view to save, given the row at the top of the
    /// tree's scroll. Remembers that row, for a view kept once the tree
    /// is gone (see `keep_view`).
    pub(crate) fn take_snapshot(
        &mut self,
        top_row: Option<usize>,
    ) -> (Option<PathBuf>, Option<SnapshotView>) {
        let top = self.restore_top.clone().or_else(|| {
            let explorer = self.explorer.as_ref()?;
            let id = *explorer.rows().get(top_row?)?;
            Some(explorer.tree().path(id))
        });
        if self.explorer.is_some() {
            self.saved_top.clone_from(&top);
        }
        (self.saved_root(), self.saved_view(top))
    }

    /// The view to save, given the path of the row at the top of the
    /// tree's scroll: the tree on screen; else the saved view still
    /// waiting for its scan, with the filter and the rules as they are
    /// now; else the view kept. Cheap: the open folders and the checks are
    /// copied as bit sets and named later, off the UI thread.
    pub(super) fn saved_view(&self, top_row: Option<String>) -> Option<SnapshotView> {
        let Some(explorer) = &self.explorer else {
            if let Some(waiting) = &self.restore {
                return Some(SnapshotView::Waiting(FolderView {
                    show_ignored: self.show_ignored,
                    query: self.query.clone(),
                    kind: self.kind,
                    ..waiting.clone()
                }));
            }
            return self.kept.as_ref().map(|(_, view)| view.clone());
        };
        let tree = explorer.tree();
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
