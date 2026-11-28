//! Bringing a saved view back, and taking the view out for saving.
//!
//! A saved view applies in steps. Its filter and Show ignored apply before
//! any folder is scanned, so the first trees on screen are already filtered
//! and listed by the saved rules. The sort applies to the first tree. Each
//! folder's open folders and checks apply when that folder's first scan
//! lands. The file on the stage and the row at the top of the tree's scroll
//! apply once every folder is on screen; under a Size sort the rows reorder
//! once sizes are read, so the top row waits for them.

use std::path::{Path, PathBuf};

use explorer::{Explorer, SavedFolder, SortKey};
use gpui_kit::Context;

use super::history::Record;
use super::{AppState, StageView, StateEvent};
use crate::persist::{FolderSnapshot, SavedView, SnapshotView};

/// The parts of a saved view that wait for the folders to land; empty for
/// a launch with no saved view, which settles at the top.
#[derive(Clone, Debug, Default)]
pub(super) struct Pending {
    current: Option<PathBuf>,
    top_row: Option<PathBuf>,
    view: StageView,
}

impl AppState {
    /// Takes the filter and the rules of `view`, which apply from the first
    /// scan, and keeps what waits for the folders. Returns each folder's
    /// part, to wait for its folder.
    pub(super) fn take_view(&mut self, view: SavedView) -> Vec<SavedFolder> {
        self.query = view.query;
        self.kind = view.kind;
        self.show_ignored = view.show_ignored;
        self.sort = view.sort;
        self.restore = Some(Pending {
            current: view.current,
            top_row: view.top_row,
            view: view.view,
        });
        view.folders
    }

    /// The explorer the first tree joins: with the sort, saved or last
    /// chosen, and the filter asked for.
    pub(super) fn new_explorer(&self) -> Explorer {
        let mut explorer = Explorer::empty();
        explorer.set_sort(self.sort);
        if self.filter_asked() {
            explorer.set_filter(&self.query, self.kind);
        }
        explorer
    }

    /// One folder's saved part, onto the tree on screen.
    pub(super) fn restore_folder(&mut self, saved: &SavedFolder) {
        if let Some(explorer) = self.explorer.as_mut() {
            explorer.restore_folder(saved);
        }
        self.selection_changed();
    }

    /// After a folder landed or left: once every folder is on screen, what
    /// a saved view waits with applies, and a top row waiting for sizes
    /// shows once they are in. Either way the views hear that the rows
    /// were rebuilt. Once every folder is on screen, what the stage shows
    /// replaces the history's step: a launch's saved file or gallery is
    /// its first, unless files asked for show in its place next.
    pub(super) fn landed(&mut self, cx: &mut Context<Self>) {
        if self.folders.iter().any(|folder| !folder.shown) {
            return self.rows_replaced(cx);
        }
        if let Some(pending) = self.restore.take() {
            self.finish_restore(pending, cx);
        } else {
            let sized = self
                .explorer
                .as_ref()
                .is_some_and(|explorer| explorer.tree().has_sizes());
            match self.restore_top.take_if(|_| sized) {
                Some(top) => self.show_saved_top(Some(&top), cx),
                None => self.rows_replaced(cx),
            }
        }
        // Files asked for record their own step as they show; the saved
        // view they replace is never on screen.
        let asked_show = self.asked_files.iter().any(|asked| self.shows_now(asked));
        if !asked_show {
            self.record(Record::Replace);
        }
    }

    /// Applies the sort, the file on the stage, the stage's view and the
    /// top row of a saved view once every folder is on screen. A file on
    /// the stage that is gone gives way to the first checked file; the
    /// gallery comes back only while two or more files are checked.
    fn finish_restore(&mut self, pending: Pending, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return self.rows_replaced(cx);
        };
        let tree = explorer.tree();
        let current = pending
            .current
            .as_deref()
            .and_then(|path| tree.find_abs(path))
            .filter(|&id| tree.is_file(id));
        let waits_for_sizes = self.sort.key == SortKey::Size && !tree.has_sizes();
        if current.is_some() {
            self.current = current;
        }
        self.selection_changed();
        self.restore_view(pending.view, cx);
        if waits_for_sizes {
            self.restore_top = pending.top_row;
            self.rows_replaced(cx);
        } else {
            self.show_saved_top(pending.top_row.as_deref(), cx);
        }
    }

    /// Scrolls the saved top row to the top of the tree, with keyboard
    /// focus on it, and tells the views the rows were rebuilt. With no
    /// saved top row, or one that is gone, the first row: a launch starts
    /// at the top.
    pub(super) fn show_saved_top(&mut self, top: Option<&Path>, cx: &mut Context<Self>) {
        let found = self.explorer.as_ref().and_then(|explorer| {
            let saved = top.and_then(|top| {
                let id = explorer.tree().find_abs(top)?;
                Some((id, explorer.row_of(id)?))
            });
            saved.or_else(|| Some((*explorer.rows().first()?, 0)))
        });
        if let Some((id, row)) = found {
            self.set_focus(id, row);
        }
        self.rows_replaced(cx);
        if let Some((_, row)) = found {
            cx.emit(StateEvent::TopRow(row));
        }
    }

    /// The view to save, with the row at the top of the tree's scroll.
    /// Cheap: the open folders and the checks of the tree on screen are
    /// copied as bit sets and named later, off the UI thread. A folder not
    /// on screen as it was seen saves the view waiting for it; one still
    /// scanning for the first time, or never read, saves as open, as it
    /// will show. A tree built in memory has no folder to save, and a file
    /// opened on its own is not saved.
    pub(crate) fn take_snapshot(&self) -> SnapshotView {
        let explorer = self.explorer.as_ref();
        let on_disk = |path: PathBuf| path.is_absolute().then_some(path);
        let pending = self.restore.as_ref();
        let top_row = self
            .restore_top
            .clone()
            .or_else(|| pending.and_then(|pending| pending.top_row.clone()))
            .or_else(|| on_disk(explorer?.tree().abs_path(self.top_node()?)));
        let current = match (explorer, self.current) {
            (Some(explorer), Some(id)) => on_disk(explorer.tree().abs_path(id)),
            _ => pending.and_then(|pending| pending.current.clone()),
        };
        let sort = self.sort;
        let view = pending.map_or(self.view, |pending| pending.view);
        let folders: Vec<FolderSnapshot> = self
            .folders
            .iter()
            // Files opened on their own last for the session.
            .filter(|folder| !folder.file)
            .map(|folder| match &folder.waiting {
                Some(waiting) => FolderSnapshot::Waiting(waiting.clone()),
                None if folder.error.is_none()
                    && (folder.shown || self.folder_node(&folder.path).is_some()) =>
                {
                    FolderSnapshot::Live(folder.path.clone())
                }
                // Still to be scanned, or never read: open, as it will show.
                None => FolderSnapshot::Waiting(SavedFolder {
                    path: folder.path.clone(),
                    open: vec![String::new()],
                    ..SavedFolder::default()
                }),
            })
            .collect();
        let live = folders
            .iter()
            .any(|folder| matches!(folder, FolderSnapshot::Live(_)));
        SnapshotView {
            live: explorer.filter(|_| live).map(Explorer::snapshot),
            folders,
            rest: SavedView {
                folders: Vec::new(),
                show_ignored: self.show_ignored,
                current,
                query: self.query.clone(),
                kind: self.kind,
                sort,
                top_row,
                view,
            },
        }
    }
}
