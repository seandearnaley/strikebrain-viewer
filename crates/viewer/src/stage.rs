//! The stage: the right pane, which shows one file at a time.
//!
//! The file is the state's `current`. The stage reads its first 8 KB off
//! the UI thread and shows it in the viewer the engine picks: an image
//! (`stage/images.rs`), Markdown or code (`stage/text.rs`), or, for
//! anything else, its name and size. Below the file, the file's place in
//! the gallery ("2 / 3") while it is in the gallery. With no file the
//! stage draws nothing.
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
//! Enter present, Esc returns to the explorer.
//!
//! The presentation is this same stage, which the root view then lays out
//! alone: it keeps its file, caches and scroll positions. While presenting
//! it declares the [`PRESENTATION`] key context, where Space moves on and
//! Esc leaves, shows no position, and gives an image the whole window.

mod cache;
mod images;
mod load;
mod text;

use std::path::Path;
use std::sync::Arc;

use explorer::{NodeId, Viewer, format_size, is_image_extension};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, Hsla, IntoElement,
    ObjectFit, ParentElement as _, Render, Resource, Result, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription, Task,
    TestSupportExt as _, Window, div, img, prelude::*, px,
};

use crate::keys::{
    LeavePresentation, NextFile, PRESENTATION, Present, PreviousFile, STAGE, ScrollDown,
    ScrollPageDown, ScrollPageUp, ScrollToBottom, ScrollToTop, ScrollUp,
};
use crate::state::AppState;
use crate::theme::TABULAR_FIGURES;

use cache::Lru;
use images::StageImages;
use load::{Held, Loaded, Stamp};
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
    /// Not shown while presenting.
    position: Option<(usize, usize)>,
    presenting: bool,
}

impl Asked {
    fn of(state: &AppState) -> Self {
        let presenting = state.is_presenting();
        Self {
            staged: state.staged(),
            position: state.position().filter(|_| !presenting),
            presenting,
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
    renders: usize,
    loads: usize,
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
        let mut this = Self {
            state,
            focus_handle: cx.focus_handle(),
            asked: Asked::default(),
            content: Content::Blank,
            showing: None,
            load: None,
            preload: None,
            images: cx.new(StageImages::new),
            texts: Lru::new(TEXT_CACHE_BYTES),
            text_views: TextViews::new(window, cx),
            renders: 0,
            loads: 0,
            _observe: observe,
        };
        this.sync(window, cx);
        this
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
            renders: self.renders,
            loads: self.loads,
            images: self.images.read(cx).len(),
            image_bytes: self.images.read(cx).bytes(),
            texts: self.texts.len(),
            text_bytes: self.texts.used(),
        }
    }

    // Loading

    /// Follows the state: starts reading a new file, and redraws when the
    /// gallery position changes or the presentation starts or ends.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let asked = Asked::of(self.state.read(cx));
        if asked == self.asked {
            return;
        }
        let new_file = asked.staged != self.asked.staged;
        let redraw =
            (asked.position, asked.presenting) != (self.asked.position, self.asked.presenting);
        self.asked = asked;
        if new_file {
            self.start_load(window, cx);
        }
        if redraw {
            cx.notify();
        }
    }

    /// The file the state asks for, if any.
    fn asked_file(&self, cx: &App) -> Option<File> {
        let state = self.state.read(cx);
        let id = self.asked.staged.1?;
        let tree = state.explorer()?.tree();
        let name = tree.name(id);
        let extension = tree.extension(id);
        let language = if extension.is_empty() {
            name
        } else {
            extension
        };
        Some(File {
            staged: self.asked.staged,
            path: tree.abs_path(id).into(),
            name: name.to_owned().into(),
            size: tree.size(id),
            language: language.to_owned().into(),
            in_memory: tree.root_path().as_os_str().is_empty(),
        })
    }

    /// Reads the file asked for on the background executor, replacing any
    /// read still running.
    fn start_load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load = None;
        self.preload = None;
        let Some(file) = self.asked_file(cx) else {
            return self.show(Content::Blank, None, window, cx);
        };
        if file.in_memory {
            // A tree from paths alone: its files are names, not files.
            let content = Content::Unsupported {
                name: file.name,
                size: file.size,
            };
            return self.show(content, None, window, cx);
        }
        let held = Held {
            shown: self
                .showing
                .as_ref()
                .filter(|showing| showing.path == file.path)
                .and_then(|showing| showing.stamp),
            text: self.texts.peek(&file.path).map(|text| text.stamp),
        };
        let (path, name) = (Arc::clone(&file.path), file.name.clone());
        self.load = Some(cx.spawn_in(window, async move |this, cx| {
            let loaded = cx
                .background_spawn(async move { load::load(&path, &name, held) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.finish_load(file, loaded, window, cx);
            })
        }));
    }

    /// Shows a read, if it is for the file still asked for.
    fn finish_load(
        &mut self,
        file: File,
        loaded: Loaded,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if file.staged != self.asked.staged {
            return;
        }
        self.load = None;
        let (content, showing) = match loaded {
            Loaded::Unchanged => {
                // The same file as on the stage, unchanged: its view stays
                // as it is, scrolled, with its selection and focus.
                if let Content::Image(shown, _) | Content::Markdown(shown) | Content::Code(shown) =
                    &mut self.content
                {
                    shown.staged = file.staged;
                }
                return self.preload_neighbours(window, cx);
            }
            Loaded::Image(stamp) => {
                let resource = Resource::Path(Arc::clone(&file.path));
                self.images.update(cx, |images, cx| {
                    images.prepare(&resource, stamp, true, window, cx);
                });
                let showing = Showing::new(&file, Viewer::Image, Some(stamp));
                (Content::Image(file, stamp.len()), showing)
            }
            Loaded::Text {
                viewer,
                stamp,
                text,
            } => {
                let text = if let Some(text) = text {
                    let entry = CachedText {
                        stamp,
                        text: text.clone(),
                    };
                    self.texts.insert(Arc::clone(&file.path), entry, text.len());
                    text
                } else if let Some(cached) = self.texts.get(&file.path) {
                    cached.text.clone()
                } else {
                    // Gone since the read started: read it whole.
                    return self.start_load(window, cx);
                };
                let showing = Showing::new(&file, viewer, Some(stamp));
                if viewer == Viewer::Markdown {
                    self.text_views.show_markdown(&file.path, stamp, &text, cx);
                    (Content::Markdown(file), showing)
                } else {
                    self.text_views
                        .show_code(&file.path, stamp, &text, &file.language, window, cx);
                    (Content::Code(file), showing)
                }
            }
            Loaded::Unsupported { len } => {
                let showing = Showing::new(&file, Viewer::Unsupported, None);
                let content = Content::Unsupported {
                    name: file.name,
                    size: len.or(file.size),
                };
                (content, showing)
            }
        };
        if !matches!(content, Content::Image(..)) {
            self.images.update(cx, |images, _| images.leave_stage());
        }
        self.loads += 1;
        self.show(content, Some(showing), window, cx);
        self.preload_neighbours(window, cx);
    }

    /// Puts `content`, which shows `showing`, on the stage. Keyboard focus
    /// inside a view that is going away moves to the stage itself; it stays
    /// in a view that shows the same file in the same viewer, such as the
    /// code view after the file changed on disk.
    fn show(
        &mut self,
        content: Content,
        showing: Option<Showing>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let same_view = match (&self.showing, &showing) {
            (Some(was), Some(now)) => was.same_view(now),
            _ => false,
        };
        if !same_view
            && self.focus_handle.contains_focused(window, cx)
            && !self.focus_handle.is_focused(window)
        {
            window.focus(&self.focus_handle, cx);
        }
        self.content = content;
        self.showing = showing;
        cx.notify();
    }

    /// Starts decoding the gallery images either side of the file on the
    /// stage, so ← and → find them ready.
    fn preload_neighbours(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let (Some(explorer), Some(current)) = (state.explorer(), state.current()) else {
            return;
        };
        let tree = explorer.tree();
        let paths: Vec<Arc<Path>> = [
            explorer.gallery_next(current),
            explorer.gallery_previous(current),
        ]
        .into_iter()
        .flatten()
        .filter(|&id| is_image_extension(tree.extension(id)))
        .map(|id| tree.abs_path(id).into())
        .collect();
        if paths.is_empty() {
            return;
        }
        let staged = self.asked.staged;
        self.preload = Some(cx.spawn_in(window, async move |this, cx| {
            let stamped: Vec<(Arc<Path>, Stamp)> = cx
                .background_spawn(async move {
                    // A large picture is decoded only when asked for.
                    paths
                        .into_iter()
                        .filter_map(|path| load::stamp(&path).map(|stamp| (path, stamp)))
                        .filter(|(_, stamp)| stamp.len() <= load::PRELOAD_LIMIT)
                        .collect()
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                if this.asked.staged != staged {
                    return;
                }
                this.images.update(cx, |images, cx| {
                    for (path, stamp) in stamped {
                        images.prepare(&Resource::Path(path), stamp, false, window, cx);
                    }
                });
            })
        }));
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

    fn leave_presentation(
        &mut self,
        _: &LeavePresentation,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, AppState::stop_presenting);
    }

    fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        match self.content {
            Content::Code(_) => self.text_views.scroll_code(scroll, cx),
            Content::Markdown(_) => self.text_views.scroll_markdown(scroll, cx),
            Content::Blank | Content::Image(..) | Content::Unsupported { .. } => {}
        }
    }

    // Rendering

    fn render_content(&self, cx: &App) -> Option<AnyElement> {
        Some(match &self.content {
            Content::Blank => return None,
            Content::Image(file, len) => self.render_image(file, *len, cx),
            Content::Markdown(file) => self
                .text_views
                .render_markdown(file.name.clone())
                .into_any_element(),
            Content::Code(file) => self
                .text_views
                .render_code(file.name.clone())
                .into_any_element(),
            Content::Unsupported { name, size } => {
                unsupported(name.clone(), *size, cx.theme().muted_foreground)
            }
        })
    }

    /// The image, scaled down to fit and never up, centred: inset from the
    /// pane's edges, or using the whole window while presenting. A file
    /// GPUI cannot decode shows as unsupported.
    ///
    /// An `img` gives itself the picture's aspect ratio, and as a block's
    /// child it takes its height from its width, so a tall picture would
    /// run off the bottom. As the only item of a flex frame placed by its
    /// insets, its full height is the frame's.
    fn render_image(&self, file: &File, len: u64, cx: &App) -> AnyElement {
        let name = file.name.clone();
        let (fallback_name, muted) = (name.clone(), cx.theme().muted_foreground);
        div()
            .id("stage-image")
            .test_support()
            .role(Role::Image)
            .aria_label(name)
            .absolute()
            .inset_0()
            .flex()
            .when(!self.asked.presenting, Styled::p_6)
            .child(
                img(Arc::clone(&file.path))
                    .image_cache(&self.images)
                    // An id lets an animated GIF play. Keyed by the file,
                    // so each file starts from its first frame and a
                    // rescan, which renumbers the tree, does not restart it.
                    .id(("image", path_key(&file.path)))
                    .size_full()
                    .object_fit(ObjectFit::ScaleDown)
                    .with_fallback(move || unsupported(fallback_name.clone(), Some(len), muted)),
            )
            .into_any_element()
    }

    /// The file's place in the gallery, quiet and centred.
    fn render_position(at: usize, of: usize, cx: &App) -> impl IntoElement {
        let text: SharedString = format!("{at} / {of}").into();
        let theme = cx.theme();
        div()
            .id("stage-position")
            .test_support()
            .role(Role::Label)
            .aria_label(text.clone())
            .flex_shrink_0()
            .h_8()
            .w_full()
            .flex()
            .items_center()
            .justify_center()
            .font_family(theme.mono_font_family.clone())
            .font_features(TABULAR_FIGURES.clone())
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text)
    }
}

/// A number for a path, to key an element by its file.
fn path_key(path: &Path) -> usize {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    // Only a key: on a 32-bit target, half the bits do as well.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a key, not a count: losing high bits only makes collisions likelier"
    )]
    let key = hasher.finish() as usize;
    key
}

/// A file the stage cannot show: its name and size, centred and muted.
fn unsupported(name: SharedString, size: Option<u64>, muted: Hsla) -> AnyElement {
    let size: Option<SharedString> = size.map(|bytes| format_size(bytes).into());
    v_flex()
        .id("stage-unsupported")
        .test_support()
        .role(Role::Group)
        .aria_label(name.clone())
        .size_full()
        .items_center()
        .justify_center()
        .gap_1()
        .px_8()
        .text_color(muted)
        .child(
            div()
                .id("stage-file")
                .test_support()
                .role(Role::Label)
                .aria_label(name.clone())
                .max_w_full()
                .truncate()
                .text_sm()
                .child(name),
        )
        .when_some(size, |view, size| {
            view.child(
                div()
                    .id("stage-size")
                    .test_support()
                    .role(Role::Label)
                    .aria_label(size.clone())
                    .text_xs()
                    .font_features(TABULAR_FIGURES.clone())
                    .child(size),
            )
        })
        .into_any_element()
}

impl Render for Stage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let content = self.render_content(cx);
        let theme = cx.theme();

        v_flex()
            .id("stage")
            .test_support()
            .track_focus(&self.focus_handle)
            .key_context(if self.asked.presenting {
                PRESENTATION
            } else {
                STAGE
            })
            .on_action(cx.listener(Self::next_file))
            .on_action(cx.listener(Self::previous_file))
            .on_action(cx.listener(Self::present))
            .on_action(cx.listener(Self::leave_presentation))
            .on_action(cx.listener(|this, _: &ScrollUp, _, cx| this.scroll(Scroll::Up, cx)))
            .on_action(cx.listener(|this, _: &ScrollDown, _, cx| this.scroll(Scroll::Down, cx)))
            .on_action(cx.listener(|this, _: &ScrollPageUp, _, cx| {
                this.scroll(Scroll::PageUp, cx);
            }))
            .on_action(cx.listener(|this, _: &ScrollPageDown, _, cx| {
                this.scroll(Scroll::PageDown, cx);
            }))
            .on_action(cx.listener(|this, _: &ScrollToTop, _, cx| this.scroll(Scroll::Top, cx)))
            .on_action(cx.listener(|this, _: &ScrollToBottom, _, cx| {
                this.scroll(Scroll::Bottom, cx);
            }))
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .children(content),
            )
            .when_some(self.asked.position, |stage, (at, of)| {
                stage.child(Self::render_position(at, of, cx))
            })
    }
}
