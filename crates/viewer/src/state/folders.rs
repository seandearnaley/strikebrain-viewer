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
//! A file asked for opens its folder by the same rule, then shows on the
//! stage once a tree lists it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use explorer::{Explorer, NodeId, SavedFolder};
use gpui_kit::{Context, PathPromptOptions, Result, SharedString, Task, TaskExt as _};

use super::{AppState, ScanStatus};
use crate::menus::APP_NAME;
use crate::persist::resolve;

/// One folder asked for: where it is, its latest scan, its watcher, and the
/// saved view waiting for it.
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
    /// One inside a folder whose first scan runs waits for that scan. A
    /// file opens its folder so, then shows on the stage with its row the
    /// focus row; of several files the last one shows. `waiting` holds
    /// saved views, each applied when its folder's first scan lands.
    pub fn open_paths(
        &mut self,
        paths: Vec<PathBuf>,
        mut waiting: Vec<SavedFolder>,
        cx: &mut Context<Self>,
    ) {
        for path in paths {
            let mut path = resolve(&path);
            if path.is_file() {
                let folder = path.parent().map(Path::to_path_buf);
                self.asked_files.push(path.clone());
                path = folder.unwrap_or(path);
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
        self.settle_asked_files(cx);
        cx.notify();
    }

    /// Shows each file asked for that a tree now lists, in the file view
    /// with its row the focus row. A file whose folder is still to land
    /// waits, as does every file while a launch settles, so the saved view
    /// does not replace it. One that no tree lists once its folder landed
    /// (deleted meanwhile, or its folder could not be read) is forgotten.
    pub(super) fn settle_asked_files(&mut self, cx: &mut Context<Self>) {
        if self.asked_files.is_empty() || self.restore.is_some() {
            return;
        }
        for path in std::mem::take(&mut self.asked_files) {
            if let Some(id) = self.listed(&path) {
                self.show(id, cx);
                self.reveal(id, cx);
            } else if self
                .folders
                .iter()
                .any(|folder| !folder.shown && folder.holds(&path))
            {
                self.asked_files.push(path);
            }
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
        let below = path.strip_prefix(folder).ok()?.to_str()?;
        explorer.tree().find_from(top, below)
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
            self.open_paths(vec![path], saved.into_iter().collect(), cx);
        }
    }

    /// Removes the top-level folder `top` with its checks, and stops its
    /// scan and its watcher: its row's remove button, and ⌘⌫. Removing the
    /// last folder shows the empty state. Does nothing for a row that is
    /// not a top-level folder.
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
        self.asked_files.retain(|file| !file.starts_with(&path));
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
    }

    /// ⌘⌫ on the focus row: removes it if it is a top-level folder.
    pub fn remove_focus_folder(&mut self, cx: &mut Context<Self>) {
        if let Some(focus) = self.focus {
            self.remove_folder(focus, cx);
        }
    }

    /// Whether `id` is the row of a top-level folder, which can be removed.
    #[must_use]
    pub fn is_folder_row(&self, id: NodeId) -> bool {
        self.explorer
            .as_ref()
            .is_some_and(|explorer| folder_row(explorer, id))
    }

    /// What the window title says: the folder's name with one folder open,
    /// the application's name otherwise.
    #[must_use]
    pub fn title(&self) -> SharedString {
        let [folder] = self.folders.as_slice() else {
            return APP_NAME.into();
        };
        let name = folder.path.file_name().unwrap_or(folder.path.as_os_str());
        name.to_string_lossy().into_owned().into()
    }

    /// A top-level folder row's tooltip: its full path, which tells two
    /// folders of one name apart, and the operating system's message when
    /// it could not be read. `None` for any other row.
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

/// Whether `id` is a top-level folder of a tree of folders.
fn folder_row(explorer: &Explorer, id: NodeId) -> bool {
    let tree = explorer.tree();
    tree.is_joined() && tree.parent(id) == Some(NodeId::ROOT)
}
