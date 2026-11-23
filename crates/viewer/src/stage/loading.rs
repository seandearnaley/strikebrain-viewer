//! Loading: following the state, reading the file asked for off the UI
//! thread, showing what lands, and decoding the gallery's neighbours
//! ahead. See the stage's module docs.

use std::path::Path;
use std::sync::Arc;

use explorer::{Viewer, is_image_extension};
use gpui_kit::{App, AppContext as _, Context, Resource, Window};

use super::load::{self, Held, Loaded, Stamp};
use super::{Asked, CachedText, Content, File, Showing, Stage};

impl Stage {
    /// Follows the state: starts reading a new file, and redraws when the
    /// gallery position changes, the presentation starts or ends, or the
    /// gallery shows or hides. While the gallery shows nothing is read for
    /// the file view; back from it, another file than the one the view last
    /// showed starts blank rather than showing the old one meanwhile.
    pub(super) fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let asked = Asked::of(self.state.read(cx));
        if asked == self.asked {
            return;
        }
        let was = std::mem::replace(&mut self.asked, asked);
        if asked.gallery {
            self.load = None;
            self.preload = None;
        } else if asked.staged != was.staged || was.gallery {
            if was.gallery {
                let file = self.asked_file(cx).map(|file| file.path);
                let same = file
                    .zip(self.showing.as_ref())
                    .is_some_and(|(file, showing)| file == showing.path);
                if !same {
                    self.content = Content::Blank;
                    self.showing = None;
                }
            }
            self.start_load(window, cx);
        }
        let shape = |asked: Asked| (asked.position, asked.presenting, asked.gallery, asked.back);
        if shape(asked) != shape(was) {
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
        let path = tree.abs_path(id);
        Some(File {
            staged: self.asked.staged,
            // A tree built from paths alone lists relative paths, not files.
            in_memory: !path.is_absolute(),
            path: path.into(),
            name: name.to_owned().into(),
            size: tree.size(id),
            language: language.to_owned().into(),
            prose: super::text::is_prose(name, extension),
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
                    self.text_views.show_code(
                        &file.path,
                        stamp,
                        &text,
                        &file.language,
                        file.prose,
                        window,
                        cx,
                    );
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
}
