//! The saved state: which folders were open and how they were seen, the
//! explorer's width and the window's bounds. One JSON file in the
//! platform's config folder (on macOS `~/Library/Application
//! Support/ai.brainstrike.viewer/state.json`), read once at launch and
//! written in the background as the view changes.
//!
//! Each folder is saved by its canonical path, with its open folders and
//! checks relative to it, with `/` separators, so they outlive a tree's
//! ids; the file on the stage and the top row are saved by their paths on
//! disk. Checks are stored compactly: a folder whose every file is checked
//! is one entry. The file carries a schema version: version 1, from before
//! several folders, still loads, as one folder; a file that is missing,
//! unreadable, malformed or of another version is ignored and the app
//! starts fresh. Writes go to a temporary file, synced to disk, that is
//! renamed over the old one, so a reader never sees half a file.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash as _, Hasher as _};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use explorer::{KindFilter, SavedFolder, Sort, ViewSnapshot};
use gpui_kit::{Bounds, Pixels, WindowBounds, point, size};

use crate::state::Launch;

/// The bundle identifier. It also names the config folder on macOS.
pub const APP_ID: &str = "ai.brainstrike.viewer";

/// Overrides where the state file lives: a path to the file itself. For
/// running a build without touching the saved state of the installed app.
pub const STATE_FILE_VAR: &str = "STRIKEBRAIN_VIEWER_STATE";

/// How the explorer was seen: everything that is restored, by path. Each
/// folder's part is restored only when that folder opens again.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedView {
    /// Each folder, in the order added: its canonical path, open folders
    /// and checks.
    pub folders: Vec<SavedFolder>,
    /// Whether the files the ignore rules hide were listed.
    pub show_ignored: bool,
    /// The file on the stage, by its path on disk.
    pub current: Option<PathBuf>,
    /// The filter text.
    pub query: String,
    /// The filter kind.
    pub kind: KindFilter,
    /// The tree's sort.
    pub sort: Sort,
    /// The row at the top of the tree's scroll, by its path on disk.
    pub top_row: Option<PathBuf>,
}

/// Where the window was, relative to the display it was on, and that
/// display's persistent identifier. Its bounds are what a window opens
/// with: the top-left corner of the frame and the size of the content,
/// title bar excluded.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedWindow {
    /// Windowed, maximized or fullscreen, with the bounds to restore to.
    pub bounds: WindowBounds,
    /// The display's UUID, when the platform gives one.
    pub display: Option<String>,
}

/// Everything the state file holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saved {
    /// How the explorer was seen.
    pub view: SavedView,
    /// The explorer's width, in pixels.
    pub explorer_width: Option<f32>,
    /// Where the window was.
    pub window: Option<SavedWindow>,
}

/// What a launch opens and what of the saved state applies to it.
#[derive(Debug)]
pub struct Plan {
    /// The folders to open, or nothing.
    pub launch: Launch,
    /// The saved view, when a folder opened was saved: with only the
    /// folders that open.
    pub view: Option<SavedView>,
    /// The explorer's saved width.
    pub explorer_width: Option<f32>,
    /// The window's saved place.
    pub window: Option<SavedWindow>,
}

/// Decides what a launch opens. Folders from the command line open,
/// exactly those, each taking its saved view if it was saved. With none,
/// the saved folders reopen with their views; one that no longer exists is
/// dropped. The filter, the sort, Show ignored, the file on the stage and
/// the top row come back with any saved folder that opens. The window and
/// the explorer's width are restored either way. Touches the disk:
/// canonicalizing paths and asking whether folders exist.
#[must_use]
pub fn plan_launch(folders: Vec<PathBuf>, saved: Option<Saved>) -> Plan {
    let saved = saved.unwrap_or_default();
    let mut view = saved.view;
    let open = if folders.is_empty() {
        view.folders.retain(|folder| folder.path.is_dir());
        view.folders
            .iter()
            .map(|folder| folder.path.clone())
            .collect()
    } else {
        let asked: Vec<PathBuf> = folders.iter().map(|folder| resolve(folder)).collect();
        view.folders.retain(|folder| asked.contains(&folder.path));
        folders
    };
    Plan {
        launch: if open.is_empty() {
            Launch::Empty
        } else {
            Launch::Folders(open)
        },
        view: (!view.folders.is_empty()).then_some(view),
        explorer_width: saved.explorer_width,
        window: saved.window,
    }
}

/// `path` made absolute and canonical as far as it exists: a folder that is
/// gone, or on a volume not mounted, keeps the place its nearest existing
/// parent resolves to, so it still matches the path it was saved under.
pub(crate) fn resolve(path: &Path) -> PathBuf {
    if let Ok(found) = fs::canonicalize(path) {
        return found;
    }
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut rest = Vec::new();
    let mut at = absolute.as_path();
    while let (Some(parent), Some(name)) = (at.parent(), at.file_name()) {
        rest.push(name);
        if let Ok(found) = fs::canonicalize(parent) {
            return rest.iter().rev().fold(found, |path, name| path.join(name));
        }
        at = parent;
    }
    absolute
}

/// `bounds` moved and, if need be, shrunk to lie inside `area`.
#[must_use]
pub fn fit(bounds: Bounds<Pixels>, area: Bounds<Pixels>) -> Bounds<Pixels> {
    let width = bounds.size.width.min(area.size.width);
    let height = bounds.size.height.min(area.size.height);
    let x = bounds
        .origin
        .x
        .clamp(area.origin.x, area.origin.x + area.size.width - width);
    let y = bounds
        .origin
        .y
        .clamp(area.origin.y, area.origin.y + area.size.height - height);
    Bounds::new(point(x, y), size(width, height))
}

/// The state of the view, copied out on the UI thread; [`Snapshot::saved`]
/// names its paths, which can take a while, anywhere.
#[derive(Debug)]
pub(crate) struct Snapshot {
    pub(crate) view: SnapshotView,
    pub(crate) explorer_width: Option<f32>,
    pub(crate) window: Option<SavedWindow>,
}

/// The view in a [`Snapshot`].
#[derive(Clone, Debug)]
pub(crate) struct SnapshotView {
    /// The tree on screen, its folders' open folders and checks still to
    /// be named.
    pub(crate) live: Option<ViewSnapshot>,
    /// Every folder, in order.
    pub(crate) folders: Vec<FolderSnapshot>,
    /// The rest of the view, its `folders` left empty.
    pub(crate) rest: SavedView,
}

/// One folder of a [`SnapshotView`].
#[derive(Clone, Debug)]
pub(crate) enum FolderSnapshot {
    /// On screen: its part of the tree, named from the live snapshot.
    Live(PathBuf),
    /// Not on screen as it was seen: the saved view waiting for its scan,
    /// or the view it had when it could last be read.
    Waiting(SavedFolder),
}

impl SnapshotView {
    /// The view with every folder named. Costs a path per entry, so the UI
    /// thread calls it only where that is rare.
    pub(crate) fn named(self) -> SavedView {
        let mut live = self.live.map_or_else(Vec::new, |live| live.folders());
        let folders = self
            .folders
            .into_iter()
            .map(|folder| match folder {
                FolderSnapshot::Live(path) => {
                    match live.iter().position(|named| named.path == path) {
                        Some(at) => live.swap_remove(at),
                        None => SavedFolder {
                            path,
                            ..SavedFolder::default()
                        },
                    }
                }
                FolderSnapshot::Waiting(saved) => saved,
            })
            .collect();
        SavedView {
            folders,
            ..self.rest
        }
    }
}

impl Snapshot {
    /// The state to write, with every folder named.
    pub(crate) fn saved(self) -> Saved {
        Saved {
            view: self.view.named(),
            explorer_width: self.explorer_width,
            window: self.window,
        }
    }
}

/// The state file: reads it, and writes it so that saves started in order
/// land in order and an unchanged state is not written again. Clones share
/// that bookkeeping.
#[derive(Clone, Debug)]
pub struct StateStore {
    path: PathBuf,
    written: Arc<Mutex<Written>>,
}

/// What the store last wrote.
#[derive(Debug, Default)]
struct Written {
    /// The ticket of the last save written or skipped as unchanged.
    ticket: u64,
    /// The next ticket to hand out.
    next: u64,
    /// A hash of the bytes last written or read.
    hash: Option<u64>,
}

impl StateStore {
    /// The state file at `path`. Tests point this at a temporary folder.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            written: Arc::default(),
        }
    }

    /// The state file in the platform's config folder, or where
    /// [`STATE_FILE_VAR`] says. `None` when the platform has no home folder.
    #[must_use]
    pub fn in_config_dir() -> Option<Self> {
        if let Some(path) = std::env::var_os(STATE_FILE_VAR).filter(|path| !path.is_empty()) {
            return Some(Self::at(path));
        }
        let (qualifier, organization, application) = split_app_id();
        directories::ProjectDirs::from(qualifier, organization, application)
            .map(|dirs| Self::at(dirs.config_dir().join("state.json")))
    }

    /// Where the file is.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The temporary file a write goes to before it is renamed over the
    /// state file: beside it, named for this process.
    fn temporary(&self) -> PathBuf {
        let folder = self.path.parent().unwrap_or(Path::new("."));
        folder.join(format!(".state.json.{}.tmp", std::process::id()))
    }

    /// The files this store writes: the state file and its temporary name.
    pub(crate) fn own_files(&self) -> Vec<PathBuf> {
        vec![self.path.clone(), self.temporary()]
    }

    /// The saved state, or `None` if there is none usable: missing,
    /// unreadable, malformed, or of another version.
    #[must_use]
    pub fn load(&self) -> Option<Saved> {
        let bytes = fs::read(&self.path).ok()?;
        let saved = decode(&bytes)?;
        self.lock().hash = Some(hash(&bytes));
        Some(saved)
    }

    /// A ticket for a save about to be taken: saves are written only in
    /// the order their tickets were handed out. Take it on the UI thread
    /// when the snapshot is taken.
    pub(crate) fn ticket(&self) -> u64 {
        let mut written = self.lock();
        written.next += 1;
        written.next
    }

    /// Writes `saved` unless a later save already landed or the file
    /// already holds the same bytes. Returns whether it wrote. Call it off
    /// the UI thread.
    ///
    /// # Errors
    ///
    /// If the folder cannot be made or the file cannot be written.
    pub(crate) fn write(&self, ticket: u64, saved: &Saved) -> io::Result<bool> {
        // Held while writing, so two saves never interleave on disk.
        let mut written = self.lock();
        if ticket <= written.ticket {
            return Ok(false);
        }
        written.ticket = ticket;
        let bytes = encode(saved)?;
        let hash = hash(&bytes);
        if written.hash == Some(hash) {
            return Ok(false);
        }
        let folder = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(folder)?;
        let temporary = self.temporary();
        // On disk before the rename, so a crash leaves the old file or the
        // new one, never an empty one; on failure, no stray temporary file.
        let replaced =
            write_synced(&temporary, &bytes).and_then(|()| fs::rename(&temporary, &self.path));
        if let Err(err) = replaced {
            let _removed = fs::remove_file(&temporary);
            return Err(err);
        }
        written.hash = Some(hash);
        Ok(true)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Written> {
        self.written.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Writes `bytes` to a new file at `path` and waits until they are on disk.
fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// [`APP_ID`] split into the parts `directories` joins again on macOS.
fn split_app_id() -> (&'static str, &'static str, &'static str) {
    let mut parts = APP_ID.splitn(3, '.');
    let mut next = || parts.next().expect("the app id has three parts");
    (next(), next(), next())
}

fn hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

mod format;

use format::{decode, encode};

#[cfg(test)]
mod tests;
