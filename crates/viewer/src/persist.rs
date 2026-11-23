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
//! file that is renamed over the old one, so a reader never sees half a
//! file.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash as _, Hasher as _};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use explorer::{Direction, KindFilter, SavedSelection, Sort, SortKey, ViewSnapshot};
use gpui_kit::{Bounds, Pixels, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

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

/// Where the window was: its bounds as GPUI gives them, relative to the
/// display it was on, and that display's persistent identifier.
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

/// What a launch opens and what of the saved state applies to it.
#[derive(Debug)]
pub struct Plan {
    /// The folder to open, or nothing.
    pub launch: Launch,
    /// The saved view, when it belongs to the folder opened.
    pub view: Option<FolderView>,
    /// The explorer's saved width.
    pub explorer_width: Option<f32>,
    /// The window's saved place.
    pub window: Option<SavedWindow>,
}

/// Decides what a launch opens. A folder from the command line opens, and
/// takes the saved view only if it is the folder that was saved. With no
/// folder the saved one reopens with its view, if it still exists;
/// otherwise nothing opens. The window and the explorer's width are
/// restored either way. Touches the disk: canonicalizing a path and
/// asking whether a folder exists.
#[must_use]
pub fn plan_launch(folder: Option<PathBuf>, saved: Option<Saved>) -> Plan {
    let saved = saved.unwrap_or_default();
    let (launch, view) = match folder {
        Some(folder) => {
            let same = saved
                .root
                .as_ref()
                .is_some_and(|root| fs::canonicalize(&folder).is_ok_and(|asked| &asked == root));
            (Launch::Folder(folder), same.then_some(saved.view))
        }
        None => match saved.root {
            Some(root) if root.is_dir() => (Launch::Folder(root), Some(saved.view)),
            _ => (Launch::Empty, None),
        },
    };
    Plan {
        launch,
        view,
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
#[derive(Debug)]
pub(crate) enum SnapshotView {
    /// A tree is on screen: its open folders and checks still to be named,
    /// the rest of `FolderView` filled in.
    Live(ViewSnapshot, FolderView),
    /// The first scan has not landed: the view waiting to be restored.
    Waiting(FolderView),
}

impl Snapshot {
    /// The state to write. Names the open folders and the checks, and
    /// makes the root canonical, so it reads the disk once.
    pub(crate) fn saved(self) -> Saved {
        let view = match self.view {
            Some(SnapshotView::Live(snapshot, rest)) => FolderView {
                expanded: snapshot.open_paths(),
                selected: snapshot.selection(),
                ..rest
            },
            Some(SnapshotView::Waiting(view)) => view,
            None => FolderView::default(),
        };
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
        let temporary = folder.join(format!(".state.json.{}.tmp", std::process::id()));
        fs::write(&temporary, &bytes)?;
        fs::rename(&temporary, &self.path)?;
        written.hash = Some(hash);
        Ok(true)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Written> {
        self.written.lock().unwrap_or_else(PoisonError::into_inner)
    }
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

// The file format. Its types are private: the rest of the app sees `Saved`.

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct StateFile {
    version: u32,
    root: Option<PathBuf>,
    show_ignored: bool,
    expanded: Vec<String>,
    selected: SelectionFile,
    current: Option<String>,
    query: String,
    kind: KindFile,
    sort: SortFile,
    first_visible_row: Option<String>,
    explorer_width: Option<f32>,
    window: Option<WindowFile>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct SelectionFile {
    folders: Vec<String>,
    files: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum KindFile {
    #[default]
    All,
    Images,
    Text,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct SortFile {
    key: KeyFile,
    direction: DirectionFile,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum KeyFile {
    #[default]
    Name,
    Type,
    Size,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DirectionFile {
    #[default]
    Ascending,
    Descending,
}

#[derive(Debug, Serialize, Deserialize)]
struct WindowFile {
    state: WindowStateFile,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    display: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WindowStateFile {
    Windowed,
    Maximized,
    Fullscreen,
}

/// The JSON for `saved`.
fn encode(saved: &Saved) -> io::Result<Vec<u8>> {
    let view = &saved.view;
    let file = StateFile {
        version: VERSION,
        root: saved.root.clone(),
        show_ignored: view.show_ignored,
        expanded: view.expanded.clone(),
        selected: SelectionFile {
            folders: view.selected.folders.clone(),
            files: view.selected.files.clone(),
        },
        current: view.current.clone(),
        query: view.query.clone(),
        kind: match view.kind {
            KindFilter::All => KindFile::All,
            KindFilter::Images => KindFile::Images,
            KindFilter::Text => KindFile::Text,
        },
        sort: SortFile {
            key: match view.sort.key {
                SortKey::Name => KeyFile::Name,
                SortKey::Type => KeyFile::Type,
                SortKey::Size => KeyFile::Size,
            },
            direction: match view.sort.direction {
                Direction::Ascending => DirectionFile::Ascending,
                Direction::Descending => DirectionFile::Descending,
            },
        },
        first_visible_row: view.top_row.clone(),
        explorer_width: saved.explorer_width.filter(|width| width.is_finite()),
        window: saved.window.as_ref().and_then(window_file),
    };
    serde_json::to_vec_pretty(&file).map_err(io::Error::other)
}

fn window_file(window: &SavedWindow) -> Option<WindowFile> {
    let (state, bounds) = match window.bounds {
        WindowBounds::Windowed(bounds) => (WindowStateFile::Windowed, bounds),
        WindowBounds::Maximized(bounds) => (WindowStateFile::Maximized, bounds),
        WindowBounds::Fullscreen(bounds) => (WindowStateFile::Fullscreen, bounds),
    };
    let numbers = [
        bounds.origin.x,
        bounds.origin.y,
        bounds.size.width,
        bounds.size.height,
    ]
    .map(Pixels::as_f32);
    numbers.iter().all(|n| n.is_finite()).then(|| WindowFile {
        state,
        x: numbers[0],
        y: numbers[1],
        width: numbers[2],
        height: numbers[3],
        display: window.display.clone(),
    })
}

/// The state in `bytes`, or `None` if they are not a state file of this
/// version.
fn decode(bytes: &[u8]) -> Option<Saved> {
    let file: StateFile = serde_json::from_slice(bytes).ok()?;
    if file.version != VERSION {
        return None;
    }
    let view = FolderView {
        show_ignored: file.show_ignored,
        expanded: file.expanded,
        selected: SavedSelection {
            folders: file.selected.folders,
            files: file.selected.files,
        },
        current: file.current,
        query: file.query,
        kind: match file.kind {
            KindFile::All => KindFilter::All,
            KindFile::Images => KindFilter::Images,
            KindFile::Text => KindFilter::Text,
        },
        sort: Sort {
            key: match file.sort.key {
                KeyFile::Name => SortKey::Name,
                KeyFile::Type => SortKey::Type,
                KeyFile::Size => SortKey::Size,
            },
            direction: match file.sort.direction {
                DirectionFile::Ascending => Direction::Ascending,
                DirectionFile::Descending => Direction::Descending,
            },
        },
        top_row: file.first_visible_row,
    };
    Some(Saved {
        root: file.root.filter(|root| root.is_absolute()),
        view,
        explorer_width: file
            .explorer_width
            .filter(|width| width.is_finite() && *width > 0.),
        window: file.window.and_then(saved_window),
    })
}

fn saved_window(file: WindowFile) -> Option<SavedWindow> {
    let usable = [file.x, file.y].iter().all(|n| n.is_finite())
        && [file.width, file.height]
            .iter()
            .all(|n| n.is_finite() && *n > 0.);
    let bounds = Bounds::new(
        point(px(file.x), px(file.y)),
        size(px(file.width), px(file.height)),
    );
    usable.then_some(SavedWindow {
        bounds: match file.state {
            WindowStateFile::Windowed => WindowBounds::Windowed(bounds),
            WindowStateFile::Maximized => WindowBounds::Maximized(bounds),
            WindowStateFile::Fullscreen => WindowBounds::Fullscreen(bounds),
        },
        display: file.display,
    })
}

#[cfg(test)]
mod tests;
