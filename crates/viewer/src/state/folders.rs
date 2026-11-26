//! The folders asked for: adding them, removing one, and what the window
//! shows of them.
//!
//! The folders keep the order they were added in. Each is known by its
//! canonical path, which is also where its tree on screen says it is. A
//! file is never in the tree twice, and that is the whole rule: a folder
//! already open, or one an open folder's tree lists, is not added again;
//! its row opens and becomes the focus row instead. A folder inside an open
//! folder that its tree does not list (its ignore rules hide it, or it
//! could not be read) is added as a folder of its own. One inside a folder
//! whose first scan runs waits for that scan to decide. A folder whose scan
//! lists open folders takes them in when it lands (see `state/scan.rs`).
//!
//! A file asked for shows where a tree lists it. One that no tree lists,
//! and that no folder being read may list, is a top-level entry of its own,
//! its folder neither read nor watched. Files asked for together are
//! checked, so they show as the gallery.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use explorer::{Explorer, NodeId, SavedFolder};
use gpui_kit::{Context, PathPromptOptions, Result, SharedString, Task, TaskExt as _};

use super::history::Record;
use super::{AppState, ScanStatus, StateEvent};
use crate::menus::APP_NAME;
use crate::persist::resolve;

/// One folder asked for, or a file opened on its own: where it is, its
/// latest scan, its watcher, and the saved view waiting for it.
pub(super) struct Folder {
    /// Tells this folder's tasks it apart as others come and go.
    pub(super) key: u64,
    /// Where the folder is: canonical, or absolute when it cannot be
    /// resolved. Its tree on screen is at this path.
    pub(super) path: PathBuf,
    /// Whether its tree is on screen: its first scan landed, read or not.
    pub(super) shown: bool,
    /// Why it could not be read at its last scan: the operating system's
    /// message. It shows as one unreadable row.
    pub(super) error: Option<SharedString>,
    /// Its part of a saved view, waiting for a scan that can read it: from
    /// the saved state, or kept from when it could last be read.
    pub(super) waiting: Option<SavedFolder>,
    /// The latest scan's number; a result from an older one is dropped.
    pub(super) generation: u64,
    /// Whether the latest scan runs, and whether it is slow.
    pub(super) scanning: Option<bool>,
    /// Stops the latest scan's worker threads.
    pub(super) cancel: Arc<AtomicBool>,
    /// The latest scan, kept until replaced: it reads sizes after the tree
    /// is on screen.
    pub(super) scan_task: Option<Task<Result<()>>>,
    /// Marks the latest scan slow.
    pub(super) slow_task: Option<Task<Result<()>>>,
    /// Follows the folder: its watcher and the task batching its events.
    pub(super) watch_task: Option<Task<Result<()>>>,
    /// Why the folder is not followed, when its watcher failed to start.
    pub(super) watch_error: Option<SharedString>,
    /// Something changed on disk while a scan was running: rescan once it
    /// lands.
    pub(super) rescan_pending: bool,
    /// Folders asked for inside this one while its first scan ran, with
    /// their saved views: shown in its tree when it lands if it lists
    /// them, else added as folders of their own.
    pub(super) asked_inside: Vec<(PathBuf, Option<SavedFolder>)>,
    /// A file opened on its own: its tree is the file alone, it is not
    /// followed on disk (Refresh reads it again), and it is never saved.
    pub(super) file: bool,
}

/// The files one call asked for, in the order asked. They show together,
/// once a tree lists each one that is not forgotten.
#[derive(Debug)]
pub(super) struct AskedFiles {
    paths: Vec<PathBuf>,
    /// Asked for with others: checked, so they show as the gallery.
    check: bool,
}

/// Where a file asked for stands.
#[derive(Clone, Copy, Debug)]
enum Standing {
    /// A tree on screen lists it.
    Listed,
    /// A folder being read may list it, or its own row is being read again.
    Waits,
    /// No tree lists it or may: it is to be a row of its own.
    Unlisted,
    /// Gone from disk, or a row of its own already that could not be read,
    /// which is not read again.
    Forgotten,
}

impl std::fmt::Debug for Folder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Folder")
            .field("path", &self.path)
            .field("shown", &self.shown)
            .field("scanning", &self.scanning)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Folder {
    fn new(key: u64, path: PathBuf, waiting: Option<SavedFolder>) -> Self {
        Self {
            key,
            path,
            shown: false,
            error: None,
            waiting,
            generation: 0,
            scanning: None,
            cancel: Arc::new(AtomicBool::new(false)),
            scan_task: None,
            slow_task: None,
            watch_task: None,
            watch_error: None,
            rescan_pending: false,
            asked_inside: Vec::new(),
            file: false,
        }
    }

    /// Where its latest scan stands.
    pub(super) fn status(&self) -> ScanStatus {
        match self.scanning {
            None => ScanStatus::Idle,
            Some(slow) => ScanStatus::Scanning { slow },
        }
    }

    /// Whether `path` is inside this folder, or is it.
    pub(super) fn holds(&self, path: &Path) -> bool {
        path.starts_with(&self.path)
    }
}

impl AppState {
    /// Shows the system folder picker and adds the folders chosen.
    pub fn prompt_for_folders(cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            // A cancelled picker or a dropped channel adds nothing; a
            // picker that failed to open is logged.
            let Ok(paths) = chosen.await else {
                return Ok(());
            };
            let Some(folders) = paths? else {
                return Ok(());
            };
            this.update(cx, |this, cx| this.open_paths(folders, Vec::new(), cx))
        })
        .detach_and_log_err(cx);
    }

    /// Opens each of `paths`: from the picker, the command line, "Open
    /// With" or the Dock. A folder is added as a top-level folder, after
    /// those open, and scanned. One already open, or one an open folder's
    /// tree lists, is not added: its row opens and becomes the focus row.
    /// One inside a folder whose first scan runs waits for that scan. Each
    /// folder asked for emits `StateEvent::FolderAdded`, and the window
    /// shows the explorer; a launch's folders emit before the window
    /// listens. A file shows where a tree lists it, or as a top-level row
    /// of its own, emitting nothing, on the stage with its row the focus
    /// row. Several files in one call are checked as well, so they show as
    /// the gallery, with the files checked before. `waiting` holds saved
    /// views, each applied when its folder's first scan lands.
    pub fn open_paths(
        &mut self,
        paths: Vec<PathBuf>,
        waiting: Vec<SavedFolder>,
        cx: &mut Context<Self>,
    ) {
        self.open_asked(paths, waiting, true, cx);
    }

    /// Opens `paths` as [`AppState::open_paths`] does, each folder among
    /// them emitting `StateEvent::FolderAdded` only with `announce`.
    fn open_asked(
        &mut self,
        paths: Vec<PathBuf>,
        mut waiting: Vec<SavedFolder>,
        announce: bool,
        cx: &mut Context<Self>,
    ) {
        let mut files = Vec::new();
        for path in paths {
            let path = resolve(&path);
            // A file shows on the stage; a folder, as its row.
            if path.is_file() {
                self.open_file(path.clone(), cx);
                files.push(path);
                continue;
            }
            if announce {
                cx.emit(StateEvent::FolderAdded);
            }
            let saved = waiting
                .iter()
                .position(|view| view.path == path)
                .map(|at| waiting.swap_remove(at));
            // Open already, read or not, or in a tree on screen.
            let open = self.folders.iter().any(|folder| folder.path == path);
            if let Some(id) = self.listed(&path).or_else(|| self.folder_node(&path)) {
                self.reveal(id, cx);
                continue;
            }
            if open {
                continue;
            }
            if let Some(outer) = self
                .folders
                .iter_mut()
                .find(|folder| !folder.shown && folder.holds(&path))
            {
                outer.asked_inside.push((path, saved));
                continue;
            }
            let key = self.next_key;
            self.next_key += 1;
            self.folders.push(Folder::new(key, path, saved));
            self.follow(key, cx);
            self.start_scan(key, cx);
        }
        if !files.is_empty() {
            let check = files.len() > 1;
            self.asked_files.push(AskedFiles {
                paths: files,
                check,
            });
        }
        self.settle_asked_files(cx);
        cx.notify();
    }

    /// A file asked for that no tree lists, and that no folder being read
    /// may list: a top-level row of its own. One already a row that could
    /// not be read is read again.
    fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let waits = self.listed(&path).is_some()
            || self
                .folders
                .iter()
                .any(|folder| !folder.shown && folder.holds(&path));
        if waits {
            return;
        }
        if let Some(key) = self
            .folders
            .iter()
            .find(|folder| folder.path == path)
            .map(|folder| folder.key)
        {
            return self.refresh_folder(key, cx);
        }
        let key = self.next_key;
        self.next_key += 1;
        let mut entry = Folder::new(key, path, None);
        entry.file = true;
        self.folders.push(entry);
        self.follow(key, cx);
        self.start_scan(key, cx);
    }

    /// Shows the files each call asked for once a tree lists every one of
    /// them that is not forgotten, in the order asked, each row the focus
    /// row in turn, so the last asked for ends it: in the file view, or
    /// checked into the gallery when they were several. A file whose folder
    /// is still to land waits, as does one whose own row is being read
    /// again, and with it the rest of its call; every file waits while a
    /// launch settles, so the saved view does not replace it. One that no
    /// tree lists once its folder landed becomes a row of its own if it is
    /// a file on disk, and is forgotten otherwise. The files shown are one
    /// step in the history, what the stage shows once they all are: it
    /// never shows them one by one.
    pub(super) fn settle_asked_files(&mut self, cx: &mut Context<Self>) {
        if self.asked_files.is_empty() || self.restore.is_some() {
            return;
        }
        let mut shown = false;
        for mut asked in std::mem::take(&mut self.asked_files) {
            let mut ready = true;
            asked.paths.retain(|path| match self.standing(path) {
                Standing::Listed => true,
                Standing::Waits => {
                    ready = false;
                    true
                }
                Standing::Unlisted => {
                    // Its folder landed and does not list it (its ignore
                    // rules hide it): a row of its own, shown once read.
                    self.open_file(path.clone(), cx);
                    ready = false;
                    true
                }
                Standing::Forgotten => false,
            });
            if !ready {
                self.asked_files.push(asked);
                continue;
            }
            for path in &asked.paths {
                let Some(id) = self.listed(path) else {
                    continue;
                };
                if asked.check {
                    self.check_and_show(id, cx);
                } else {
                    self.show_file(id, cx);
                }
                self.reveal(id, cx);
                shown = true;
            }
        }
        if shown {
            self.record(Record::Add);
        }
    }

    /// Whether the files `asked` for show at the next settle: a tree lists
    /// each one that is not forgotten, and one at least.
    pub(super) fn shows_now(&self, asked: &AskedFiles) -> bool {
        let mut listed = false;
        for path in &asked.paths {
            match self.standing(path) {
                Standing::Listed => listed = true,
                Standing::Forgotten => {}
                Standing::Waits | Standing::Unlisted => return false,
            }
        }
        listed
    }

    /// Where the file asked for at `path` stands.
    fn standing(&self, path: &Path) -> Standing {
        if self.listed(path).is_some() {
            return Standing::Listed;
        }
        let waits = self.folders.iter().any(|folder| {
            (!folder.shown && folder.holds(path))
                || (folder.path == path && folder.scanning.is_some())
        });
        if waits {
            Standing::Waits
        } else if path.is_file() && !self.folders.iter().any(|folder| folder.path == path) {
            Standing::Unlisted
        } else {
            Standing::Forgotten
        }
    }

    /// The node at `path` in the tree on screen, if a folder there lists
    /// it: that folder itself, or a node its scan found. A folder that
    /// could not be read lists nothing.
    pub(super) fn listed(&self, path: &Path) -> Option<NodeId> {
        self.folders
            .iter()
            .filter(|folder| folder.shown && folder.error.is_none() && folder.holds(path))
            .find_map(|folder| self.node_in(&folder.path, path))
    }

    /// The node at `path` below the top-level folder at `folder`.
    fn node_in(&self, folder: &Path, path: &Path) -> Option<NodeId> {
        let explorer = self.explorer.as_ref()?;
        let top = explorer.folder_at(folder)?;
        // The tree's own separator is `/`, whatever the platform's is.
        let names = path
            .strip_prefix(folder)
            .ok()?
            .components()
            .map(|part| match part {
                Component::Normal(name) => name.to_str(),
                _ => None,
            })
            .collect::<Option<Vec<&str>>>()?;
        explorer.tree().find_from(top, &names.join("/"))
    }

    /// Opens the folders above `id` and makes it the focus row.
    pub(super) fn reveal(&mut self, id: NodeId, cx: &mut Context<Self>) {
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        if let Some(row) = explorer.reveal(id) {
            self.set_focus(id, row);
            self.rows_replaced(cx);
        }
    }

    /// After the folder at `path` landed: each folder asked for inside it
    /// meanwhile shows in its tree if the tree lists it, or is added as a
    /// folder of its own.
    pub(super) fn settle_asked_inside(
        &mut self,
        asked: Vec<(PathBuf, Option<SavedFolder>)>,
        cx: &mut Context<Self>,
    ) {
        for (path, saved) in asked {
            // Announced when it was asked for, unless the launch asked.
            self.open_asked(vec![path], saved.into_iter().collect(), false, cx);
        }
    }

    /// Removes the top-level folder `top` with its checks, or the file
    /// opened on its own there, and stops its scan and its watcher: its
    /// row's remove button, and ⌘⌫. Removing the last shows the empty
    /// state. Does nothing for a row that is not a top-level row.
    pub fn remove_folder(&mut self, top: NodeId, cx: &mut Context<Self>) {
        if !self.is_folder_row(top) {
            return;
        }
        let (row, current, before) = (self.focus_index(), self.current, self.before());
        let Some(explorer) = self.explorer.as_mut() else {
            return;
        };
        let path = explorer.tree().abs_path(top);
        let (map, current) = explorer.remove_folder(top, current);
        // Focus on the removed folder moves to the row now in its place.
        let rows = explorer.rows();
        let in_place = row
            .map(|row| row.min(rows.len().saturating_sub(1)))
            .and_then(|at| Some((*rows.get(at)?, at)));
        let none_left = explorer.tree().folders().len() == 0;
        self.folders.retain(|folder| folder.path != path);
        for asked in &mut self.asked_files {
            asked.paths.retain(|file| !file.starts_with(&path));
        }
        self.current = current;
        self.focus = self.focus.and_then(|focus| map.get(focus));
        self.tile = self.tile.and_then(|tile| map.get(tile));
        if self.focus.is_none()
            && let Some((id, at)) = in_place
        {
            self.set_focus(id, at);
        }
        self.tree_replaced();
        if none_left {
            self.explorer = None;
            self.focus = None;
        }
        if self.folders.is_empty() {
            self.restore = None;
            self.restore_top = None;
        }
        self.selection_changed();
        self.landed_keeping(before, &map, cx);
        // Files asked for together with one the row held may wait no more.
        self.settle_asked_files(cx);
    }

    /// ⌘⌫ on the focus row: removes it if it is a top-level row.
    pub fn remove_focus_folder(&mut self, cx: &mut Context<Self>) {
        if let Some(focus) = self.focus {
            self.remove_folder(focus, cx);
        }
    }

    /// Whether `id` is a top-level row, which can be removed: a folder's,
    /// or a file's opened on its own.
    #[must_use]
    pub fn is_folder_row(&self, id: NodeId) -> bool {
        self.explorer
            .as_ref()
            .is_some_and(|explorer| folder_row(explorer, id))
    }

    /// The tree's accessible name: the folder's name with one folder open,
    /// or the file's with one file opened on its own; the application's
    /// name otherwise.
    #[must_use]
    pub fn title(&self) -> SharedString {
        let [folder] = self.folders.as_slice() else {
            return APP_NAME.into();
        };
        let name = folder.path.file_name().unwrap_or(folder.path.as_os_str());
        name.to_string_lossy().into_owned().into()
    }

    /// A top-level row's tooltip, a folder's or a file's opened on its own:
    /// its full path, which tells two of one name apart, and the operating
    /// system's message when it could not be read. `None` for any other
    /// row.
    #[must_use]
    pub fn folder_tooltip(&self, id: NodeId) -> Option<SharedString> {
        let explorer = self.explorer.as_ref()?;
        if !folder_row(explorer, id) {
            return None;
        }
        let path = explorer.tree().abs_path(id);
        let error = self
            .folders
            .iter()
            .find(|folder| folder.path == path)
            .and_then(|folder| folder.error.clone());
        let path = path.display();
        Some(match error {
            Some(error) => format!("{path}\n{error}").into(),
            None => path.to_string().into(),
        })
    }

    /// The folder with `key`, if it is still asked for.
    pub(super) fn folder(&self, key: u64) -> Option<&Folder> {
        self.folders.iter().find(|folder| folder.key == key)
    }

    /// The folder with `key`, to change, if it is still asked for.
    pub(super) fn folder_mut(&mut self, key: u64) -> Option<&mut Folder> {
        self.folders.iter_mut().find(|folder| folder.key == key)
    }

    /// The top-level node of the folder at `path` in the tree on screen.
    pub(super) fn folder_node(&self, path: &Path) -> Option<NodeId> {
        self.explorer.as_ref()?.folder_at(path)
    }
}

/// Whether `id` is a top-level entry of a tree of folders: a folder, or a
/// file opened on its own.
fn folder_row(explorer: &Explorer, id: NodeId) -> bool {
    let tree = explorer.tree();
    tree.is_joined() && tree.parent(id) == Some(NodeId::ROOT)
}

#[cfg(test)]
mod tests {
    use explorer::ScanError;
    use gpui_kit::{AppContext as _, TestAppContext};

    use crate::state::{AppState, Launch};

    #[gpui_kit::test]
    fn a_file_row_that_could_not_be_read_is_not_read_again_on_its_own(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "a").unwrap();
        let state = cx.new(|cx| AppState::new(Launch::Empty, cx));
        state.update(cx, |state, cx| {
            state.open_paths(vec![file.clone()], Vec::new(), cx);
            // Its scan never lands; this one fails, though the file is there.
            state.folders[0].scan_task = None;
            let generation = state.folders[0].generation;
            let error = ScanError::Root(std::io::Error::other("cannot be read"));
            state.finish_scan(0, generation, Err(error), cx);
        });
        cx.run_until_parked();
        // An unreadable row, and the file asked for is forgotten: asking
        // for it once more would read it again, and again if it failed.
        state.read_with(cx, |state, _| {
            assert_eq!(state.scans(), 1);
            assert!(state.folders[0].error.is_some());
            assert_eq!(state.current(), None);
        });
    }
}
