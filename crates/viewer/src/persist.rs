//! The saved state: which folder was open and how it was seen, the
//! explorer's width and the window's bounds. One JSON file in the
//! platform's config folder (on macOS `~/Library/Application
//! Support/ai.brainstrike.viewer/state.json`), read once at launch and
//! written in the background as the view changes.
//!
//! Paths inside the folder are stored relative to it, with `/` separators,
//! so they outlive a tree's ids. Checks are stored compactly: a folder
//! whose every file is checked is one entry. The file carries a schema
//! version; a file that is missing, unreadable, malformed or of another
//! version is ignored and the app starts fresh. Writes go to a temporary
//! file, synced to disk, that is renamed over the old one, so a reader
//! never sees half a file.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash as _, Hasher as _};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use explorer::{KindFilter, SavedSelection, Sort, ViewSnapshot};
use gpui_kit::{Bounds, Pixels, WindowBounds, point, size};

use crate::state::Launch;

/// The bundle identifier. It also names the config folder on macOS.
pub const APP_ID: &str = "ai.brainstrike.viewer";

/// Overrides where the state file lives: a path to the file itself. For
/// running a build without touching the saved state of the installed app.
pub const STATE_FILE_VAR: &str = "STRIKEBRAIN_VIEWER_STATE";

/// The state file's schema version. A file of any other is ignored.
const VERSION: u32 = 1;

/// How the folder on screen was seen: everything about it that is
/// restored, by path. Restored only when the same folder is opened again.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FolderView {
    /// Whether the files the ignore rules hide were listed.
    pub show_ignored: bool,
    /// The folders opened by hand.
    pub expanded: Vec<String>,
    /// The checked files, compactly.
    pub selected: SavedSelection,
    /// The file on the stage.
    pub current: Option<String>,
    /// The filter text.
    pub query: String,
    /// The filter kind.
    pub kind: KindFilter,
    /// The tree's sort.
    pub sort: Sort,
    /// The row at the top of the tree's scroll.
    pub top_row: Option<String>,
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
    /// The folder that was open, absolute and canonical.
    pub root: Option<PathBuf>,
    /// How it was seen.
    pub view: FolderView,
    /// The explorer's width, in pixels.
    pub explorer_width: Option<f32>,
    /// Where the window was.
    pub window: Option<SavedWindow>,
}

/// A saved folder with its view.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedFolder {
    /// The folder, absolute and canonical.
    pub root: PathBuf,
    /// How it was seen.
    pub view: FolderView,
}

/// What a launch opens and what of the saved state applies to it.
#[derive(Debug)]
pub struct Plan {
    /// The folder to open, or nothing.
    pub launch: Launch,
    /// The saved view, when it belongs to the folder opened.
    pub view: Option<FolderView>,
    /// The saved folder, when the launch does not open it: it stays the
    /// saved one, untouched, until another folder opens.
    pub kept: Option<SavedFolder>,
    /// The explorer's saved width.
    pub explorer_width: Option<f32>,
    /// The window's saved place.
    pub window: Option<SavedWindow>,
}

/// Decides what a launch opens. A folder from the command line opens, and
/// takes the saved view only if it is the folder that was saved. With no
/// folder the saved one reopens with its view, if it still exists;
/// otherwise nothing opens. A saved folder the launch does not open is
/// kept, to be saved again as it was until another folder opens: a
/// volume that is not mounted today loses nothing. The window and the
/// explorer's width are restored either way. Touches the disk:
/// canonicalizing a path and asking whether a folder exists.
#[must_use]
pub fn plan_launch(folder: Option<PathBuf>, saved: Option<Saved>) -> Plan {
    let saved = saved.unwrap_or_default();
    let opens_saved = match &folder {
        Some(folder) => saved
            .root
            .as_ref()
            .is_some_and(|root| fs::canonicalize(folder).is_ok_and(|asked| &asked == root)),
        None => saved.root.as_ref().is_some_and(|root| root.is_dir()),
    };
    let launch = match (folder, &saved.root) {
        (Some(folder), _) => Launch::Folder(folder),
        (None, Some(root)) if opens_saved => Launch::Folder(root.clone()),
        (None, _) => Launch::Empty,
    };
    let (view, kept) = if opens_saved {
        (Some(saved.view), None)
    } else {
        let kept = saved.root.map(|root| SavedFolder {
            root,
            view: saved.view,
        });
        (None, kept)
    };
    Plan {
        launch,
        view,
        kept,
        explorer_width: saved.explorer_width,
        window: saved.window,
    }
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
    /// The folder, as asked for or as scanned.
    pub(crate) root: Option<PathBuf>,
    pub(crate) view: Option<SnapshotView>,
    pub(crate) explorer_width: Option<f32>,
    pub(crate) window: Option<SavedWindow>,
}

/// The view of the folder in a [`Snapshot`].
#[derive(Clone, Debug)]
pub(crate) enum SnapshotView {
    /// A tree is on screen: its open folders and checks still to be named,
    /// the rest of `FolderView` filled in.
    Live(ViewSnapshot, FolderView),
    /// No tree is on screen: the view waiting to be restored, or one
    /// kept as it was saved.
    Waiting(FolderView),
}

impl SnapshotView {
    /// The view with its open folders and checks named. Costs a path per
    /// entry, so the UI thread calls it only where that is rare.
    pub(crate) fn named(self) -> FolderView {
        match self {
            Self::Live(snapshot, rest) => FolderView {
                expanded: snapshot.open_paths(),
                selected: snapshot.selection(),
                ..rest
            },
            Self::Waiting(view) => view,
        }
    }
}

impl Snapshot {
    /// The state to write. Names the open folders and the checks, and
    /// makes the root canonical, so it reads the disk once.
    pub(crate) fn saved(self) -> Saved {
        let view = self.view.map_or_default(SnapshotView::named);
        Saved {
            root: self
                .root
                .map(|root| fs::canonicalize(&root).unwrap_or(root)),
            view,
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
