//! The state file's format: private serde types, so the rest of the app
//! sees only `Saved`. Version 2 holds a list of folders; version 1, from
//! before several folders, still loads, as one folder.

use std::io;
use std::path::PathBuf;

use explorer::{Direction, KindFilter, SavedFolder, SavedSelection, Sort, SortKey};
use gpui_kit::{Bounds, Pixels, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

use super::{Saved, SavedView, SavedWindow};

/// The schema version written. Version 1 is still read.
const VERSION: u32 = 2;

/// Version 2: every folder with its own open folders and checks; the file
/// on the stage and the top row by their paths on disk.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct StateFile {
    version: u32,
    folders: Vec<FolderFile>,
    show_ignored: bool,
    current: Option<PathBuf>,
    query: String,
    kind: KindFile,
    sort: SortFile,
    first_visible_row: Option<PathBuf>,
    explorer_width: Option<f32>,
    window: Option<WindowFile>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct FolderFile {
    root: PathBuf,
    expanded: Vec<String>,
    selected: SelectionFile,
}

/// Version 1: one folder, and every path relative to it.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StateFileV1 {
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

/// Just the version, read first to choose the format.
#[derive(Deserialize)]
struct Versioned {
    version: u32,
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
pub(super) fn encode(saved: &Saved) -> io::Result<Vec<u8>> {
    let view = &saved.view;
    let file = StateFile {
        version: VERSION,
        folders: view
            .folders
            .iter()
            .map(|folder| FolderFile {
                root: folder.path.clone(),
                expanded: folder.open.clone(),
                selected: SelectionFile {
                    folders: folder.selection.folders.clone(),
                    files: folder.selection.files.clone(),
                },
            })
            .collect(),
        show_ignored: view.show_ignored,
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

/// The state in `bytes`, or `None` if they are not a state file of a
/// version this app reads. Values that cannot be used (a relative path, a
/// non-positive size) are dropped.
pub(super) fn decode(bytes: &[u8]) -> Option<Saved> {
    let Versioned { version } = serde_json::from_slice(bytes).ok()?;
    let file = match version {
        VERSION => serde_json::from_slice(bytes).ok()?,
        1 => from_version_1(serde_json::from_slice(bytes).ok()?),
        _ => return None,
    };
    let absolute = |path: Option<PathBuf>| path.filter(|path| path.is_absolute());
    let view = SavedView {
        folders: file
            .folders
            .into_iter()
            .filter(|folder| folder.root.is_absolute())
            .map(|folder| SavedFolder {
                path: folder.root,
                open: folder.expanded,
                selection: SavedSelection {
                    folders: folder.selected.folders,
                    files: folder.selected.files,
                },
            })
            .collect(),
        show_ignored: file.show_ignored,
        current: absolute(file.current),
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
        top_row: absolute(file.first_visible_row),
    };
    Some(Saved {
        view,
        explorer_width: file
            .explorer_width
            .filter(|width| width.is_finite() && *width > 0.),
        window: file.window.and_then(saved_window),
    })
}

/// A version 1 file as version 2: its folder, open as a folder added is,
/// and its paths made absolute. A relative root is dropped, with the
/// paths relative to it.
fn from_version_1(file: StateFileV1) -> StateFile {
    let root = file.root.filter(|root| root.is_absolute());
    let under = |path: Option<String>| Some(root.as_ref()?.join(path?));
    StateFile {
        version: VERSION,
        current: under(file.current),
        first_visible_row: under(file.first_visible_row),
        folders: root
            .iter()
            .map(|root| FolderFile {
                root: root.clone(),
                expanded: std::iter::once(String::new())
                    .chain(file.expanded.iter().cloned())
                    .collect(),
                selected: SelectionFile {
                    folders: file.selected.folders.clone(),
                    files: file.selected.files.clone(),
                },
            })
            .collect(),
        show_ignored: file.show_ignored,
        query: file.query,
        kind: file.kind,
        sort: file.sort,
        explorer_width: file.explorer_width,
        window: file.window,
    }
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
