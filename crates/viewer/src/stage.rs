//! The stage: the right pane. It shows a directory listing, the gallery of
//! checked files (`stage/gallery.rs`), and the file view, which shows one
//! file at a time and is described here. The state says which shows.
//!
//! The file is the state's `current`. The stage reads its first 8 KB off
//! the UI thread and shows it in the viewer the engine picks: an image
//! (`stage/images.rs`), Markdown or code (`stage/text.rs`), or, for
//! anything else, its name and size. Below the file, the file's place in
//! the gallery ("2 / 3") while it is in the gallery, and with two or more
//! files checked a grid button left of it that goes back to the gallery.
//! With no file the stage draws nothing. While the gallery shows, the file
//! view rests: nothing is read for it.
//!
//! Loading: every read carries the tree and the file it was started for,
//! and its result is dropped unless that is still the file asked for.
//! Starting a read replaces the task of the one before, which cancels it,
//! so flicking through files never queues work. Until a read lands the
//! stage keeps showing what it showed. Images decode in GPUI's image
//! loader, through a cache of 8 images and 512 MB (`stage/images.rs`); the
//! gallery files either side of the current one are decoded ahead, up to
//! 8 MB on disk. Text is kept in a
//! 32 MB cache. Both caches remember each file's size and modification
//! time, so after a refresh a changed file is read again and an unchanged
//! one is not.
//!
//! The root view embeds the stage cached: it redraws only when it
//! notifies, which it does when the file, its place in the gallery or
//! what it shows changes, not on every change to the state.
//!
//! The stage takes keyboard focus in the [`STAGE`] key context: ← and →
//! move through the gallery, the vertical keys scroll text, Space and
//! Enter present, Esc returns to the gallery, or to the explorer with
//! fewer than two files checked. While the gallery shows it declares
//! [`GALLERY`](crate::keys::GALLERY) and hands the gallery its keys.
//!
//! Loading lives in `stage/loading.rs` and drawing in `stage/render.rs`.
//!
//! The presentation is this same stage, which the root view then lays out
//! alone: it keeps its file, caches and scroll positions. While presenting
//! it declares the [`PRESENTATION`] key context, where Space moves on and
//! Esc leaves, shows no position, and gives an image the whole window.

mod cache;
mod folder;
mod gallery;
mod images;
mod links;
mod load;
mod loading;
mod render;
mod text;

use std::path::Path;
use std::sync::Arc;

use explorer::{NodeId, Viewer};
use gpui_kit::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, Result, SharedString,
    Subscription, Task, Window, px,
};

use crate::keys::{LeavePresentation, NextFile, Present, PreviousFile};
use crate::state::AppState;

use cache::Lru;
use gallery::Gallery;
pub use gallery::GalleryProbe;
use images::StageImages;
use load::Stamp;
use text::{Scroll, TextViews};

/// How much loaded text the stage keeps: 32 MB.
const TEXT_CACHE_BYTES: usize = 32 * 1024 * 1024;

/// What the state asks the stage to show. The stage redraws when it
/// changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Asked {
    /// The tree generation and `current`.
    staged: (u64, Option<NodeId>),
    /// "2 / 3": `current`'s place in the gallery, and the gallery's size.
    /// Shown in the file view only.
    position: Option<(usize, usize)>,
    presenting: bool,
    /// The gallery shows, not the file view.
    gallery: bool,
    folder: Option<folder::Options>,
    /// The file view offers the way back to the gallery: two or more
    /// files are checked.
    back: bool,
}

impl Asked {
    fn of(state: &AppState) -> Self {
        let presenting = state.is_presenting();
        let gallery = state.shows_gallery();
        let folder = state
            .explorer()
            .zip(state.current())
            .filter(|_| state.stage_view() == crate::StageView::Folder)
            .map(|(e, id)| folder::Options::new(e, id));
        let file_view = !presenting && !gallery && folder.is_none();
        Self {
            staged: state.staged(),
            position: state.position().filter(|_| file_view),
            presenting,
            gallery,
            folder,
            back: file_view && state.selected_count() >= 2,
        }
    }
}

/// The file asked for, as the stage reads it.
#[derive(Clone, Debug)]
struct File {
    /// The tree generation and the node: what a read was started for.
    staged: (u64, Option<NodeId>),
    path: Arc<Path>,
    name: SharedString,
    /// The size the scan read, if it has.
    size: Option<u64>,
    /// The extension, or the name for a file without one: the code view's
    /// language.
    language: SharedString,
    /// Plain prose rather than code: the code view wraps it and numbers no
    /// lines.
    prose: bool,
    /// A tree built in memory, with no folder behind it: nothing is read.
    in_memory: bool,
}

/// What the stage shows.
#[derive(Clone, Debug)]
enum Content {
    Blank,
    /// An image file and its size in bytes.
    Image(File, u64),
    Markdown(File),
    Code(File),
    Unsupported {
        name: SharedString,
        size: Option<u64>,
    },
}

/// Which file the stage shows, in which viewer, as of which version. While
/// the file and the viewer stay, keyboard focus inside the view stays; while
/// the version stays too, nothing is read or rebuilt.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Showing {
    path: Arc<Path>,
    viewer: Viewer,
    /// `None` when the file could not be read.
    stamp: Option<Stamp>,
}

impl Showing {
    fn new(file: &File, viewer: Viewer, stamp: Option<Stamp>) -> Self {
        Self {
            path: Arc::clone(&file.path),
            viewer,
            stamp,
        }
    }

    /// Whether `other` is the same file in the same viewer.
    fn same_view(&self, other: &Self) -> bool {
        self.path == other.path && self.viewer == other.viewer
    }
}

/// A text the stage has read, and the file version it came from.
#[derive(Clone, Debug)]
struct CachedText {
    stamp: Stamp,
    text: SharedString,
}

/// What the stage holds, for tests and probes: its caches, how often it
/// has rendered and how many reads it has shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageProbe {
    renders: usize,
    loads: usize,
    images: usize,
    image_bytes: usize,
    texts: usize,
    text_bytes: usize,
    scroll: f32,
    wraps: bool,
}

impl StageProbe {
    /// How many times the stage has rendered.
    #[must_use]
    pub fn renders(&self) -> usize {
        self.renders
    }

    /// How many reads have landed and been shown.
    #[must_use]
    pub fn loads(&self) -> usize {
        self.loads
    }

    /// How many images the image cache holds, decoded or decoding.
    #[must_use]
    pub fn images(&self) -> usize {
        self.images
    }

    /// How many bytes of decoded pixels the image cache holds.
    #[must_use]
    pub fn image_bytes(&self) -> usize {
        self.image_bytes
    }

    /// How many texts the text cache holds.
    #[must_use]
    pub fn texts(&self) -> usize {
        self.texts
    }

    /// How many bytes of text the text cache holds.
    #[must_use]
    pub fn text_bytes(&self) -> usize {
        self.text_bytes
    }

    /// Whether the code view shows prose: lines wrapped, none numbered.
    #[must_use]
    pub fn wraps(&self) -> bool {
        self.wraps
    }

    /// How far the text on the stage is scrolled down, in pixels; 0 at
    /// the top and for anything but text.
    #[must_use]
    pub fn scroll(&self) -> f32 {
        self.scroll
    }
}

/// The stage pane. See the module docs.
pub(crate) struct Stage {
    state: Entity<AppState>,
    focus_handle: FocusHandle,
    asked: Asked,
    content: Content,
    /// What `content` shows, when it shows a file read from disk.
    showing: Option<Showing>,
    /// The read of the file asked for. Replacing it cancels the one before.
    load: Option<Task<Result<()>>>,
    /// Stamps the gallery neighbours to decode ahead. Replaced likewise.
    preload: Option<Task<Result<()>>>,
    images: Entity<StageImages>,
    texts: Lru<Arc<Path>, CachedText>,
    text_views: TextViews,
    gallery: Entity<Gallery>,
    renders: usize,
    loads: usize,
    link_task: Option<Task<Result<()>>>,
    heading_task: Option<Task<Result<()>>>,
    section: Option<(Arc<Path>, String)>,
    _markdown_observer: Subscription,
    _observe: Subscription,
}

impl std::fmt::Debug for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stage")
            .field("asked", &self.asked)
            .field("content", &self.content)
            .finish_non_exhaustive()
    }
}

impl Focusable for Stage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Stage {
    pub(crate) fn new(
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The state changes on every key in the tree; the stage acts only
        // when what it is asked to show does.
        let observe = cx.observe_in(&state, window, |this, _, window, cx| this.sync(window, cx));
        let focus_handle = cx.focus_handle();
        let gallery = {
            let (state, focus) = (state.clone(), focus_handle.clone());
            cx.new(|cx| Gallery::new(state, focus, cx))
        };
        let text_views = TextViews::new(window, cx);
        let markdown_observer =
            cx.observe_in(&text_views.markdown, window, |this, _, window, cx| {
                this.reveal_fragment(window, cx);
            });
        let mut this = Self {
            state,
            focus_handle,
            asked: Asked::default(),
            content: Content::Blank,
            showing: None,
            load: None,
            preload: None,
            images: cx.new(StageImages::new),
            texts: Lru::new(TEXT_CACHE_BYTES),
            text_views,
            link_task: None,
            heading_task: None,
            section: None,
            _markdown_observer: markdown_observer,
            gallery,
            renders: 0,
            loads: 0,
            _observe: observe,
        };
        this.sync(window, cx);
        this
    }

    /// What the gallery holds, for tests and probes.
    pub(crate) fn gallery_probe(&self, cx: &App) -> GalleryProbe {
        self.gallery.read(cx).probe()
    }

    /// What the stage holds, for tests and probes.
    pub(crate) fn probe(&self, cx: &App) -> StageProbe {
        let scroll = match self.content {
            Content::Code(_) => self.text_views.code_scroll(cx),
            Content::Markdown(_) => self.text_views.markdown_scroll(cx),
            Content::Blank | Content::Image(..) | Content::Unsupported { .. } => px(0.),
        };
        StageProbe {
            scroll: scroll.as_f32(),
            wraps: self.text_views.code_wraps(),
            renders: self.renders,
            loads: self.loads,
            images: self.images.read(cx).len(),
            image_bytes: self.images.read(cx).bytes(),
            texts: self.texts.len(),
            text_bytes: self.texts.used(),
        }
    }

    // Keys

    fn next_file(&mut self, _: &NextFile, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, AppState::show_next);
    }

    fn previous_file(&mut self, _: &PreviousFile, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, AppState::show_previous);
    }

    fn present(&mut self, _: &Present, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, AppState::start_presenting);
    }

    /// Esc in the file view, and its grid button: back to the gallery with
    /// keyboard focus on the stage, its ring showing after a key; with
    /// fewer than two files checked, Esc goes to the explorer.
    fn show_gallery(&mut self, key: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.update(cx, AppState::show_gallery) {
            window.focus(&self.focus_handle, cx);
            if key {
                self.gallery.update(cx, Gallery::key_pressed);
            }
        } else if key {
            window.dispatch_action(Box::new(crate::keys::FocusExplorer), cx);
        }
    }

    fn leave_presentation(
        &mut self,
        _: &LeavePresentation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, AppState::stop_presenting);
        // Back in the gallery by a key: its focus ring shows.
        if self.state.read(cx).shows_gallery() {
            self.gallery.update(cx, Gallery::key_pressed);
        }
    }

    /// Tab or shift-Tab brought keyboard focus to the stage: the gallery's
    /// focus ring shows, as the tree's does.
    pub(crate) fn focused_by_key(&mut self, cx: &mut Context<Self>) {
        if self.asked.gallery || self.asked.folder.is_some() {
            self.gallery.update(cx, Gallery::key_pressed);
        }
    }

    fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        match self.content {
            Content::Code(_) => self.text_views.scroll_code(scroll, cx),
            Content::Markdown(_) => self.text_views.scroll_markdown(scroll, cx),
            Content::Blank | Content::Image(..) | Content::Unsupported { .. } => {}
        }
    }
}
