//! The stage's text views: code in the Kit's editor, Markdown in the Kit's
//! text view. Each is one entity, made once and given each file's text in
//! turn, so its layout caches and highlighter outlive a file change.
//!
//! The code view is read-only, with line numbers, `syntect` highlighting
//! picked by the file's extension (or its name, for `Makefile` and the
//! like) and no soft wrap: long lines scroll sideways. Plain prose (`.txt`,
//! `.log`, `LICENSE`) is wrapped instead, with no line numbers. Search and the
//! context menu are off: the stage shows code, it does not edit it.
//! Folding stays on: blocks fold from chevrons that show only while the
//! pointer is over the gutter, and their column is the gap that keeps the
//! line numbers off the code.
//!
//! A link in the Markdown view opens only when it is a web or mail link
//! (`http`, `https`, `mailto`); any other link does nothing.
//!
//! An image in the Markdown view loads from the file's own folder when its
//! address is a relative path, as it would on a code host; a `data:` address
//! carries its own image. Any other address is left to GPUI's loader, which
//! fetches nothing: the app has no HTTP client, so a file cannot make it
//! call out.
//!
//! The Markdown view is the text view's own scrolling mode, which lays out
//! only the blocks on screen: laying out every block each frame took 23 ms
//! a frame at 86 KB and 0.6 s at 345 KB. The text keeps to a centred
//! column of readable width, so its scrollbar sits at the column's edge;
//! the wheel over the margins either side scrolls the column too.

use std::path::Path;
use std::sync::Arc;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Entity, Image, ImageFormat, ImageSource,
    IntoElement, ListOffset, ListState, MouseButton, ParentElement as _, Pixels, Rems, Role,
    SharedString, SharedUri, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    Window, div, point, prelude::*, px,
};

use super::load::Stamp;

/// How far ↑ and ↓ scroll: 40 px, a browser's arrow step.
const STEP: Pixels = px(40.);

/// The editor's language with no highlighter: its name for plain text.
const PLAIN: &str = "text";

/// The Markdown column's widest measure: 720 px, about 75 characters of
/// body text.
const MEASURE: Rems = Rems(45.);

/// A scroll the stage's keys ask of the text on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scroll {
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

impl Scroll {
    /// The scroll offset after this scroll from `y`, in a viewport `page`
    /// tall. Offsets are zero at the top and negative below it; the views
    /// clamp the result at the end.
    fn from(self, y: Pixels, page: Pixels) -> Pixels {
        // A page keeps one step of the last screen in sight.
        let page = (page - STEP).max(STEP);
        match self {
            Self::Up => (y + STEP).min(px(0.)),
            Self::Down => y - STEP,
            Self::PageUp => (y + page).min(px(0.)),
            Self::PageDown => y - page,
            Self::Top => px(0.),
            Self::Bottom => Pixels::MIN,
        }
    }
}

/// Which file and version a view holds.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Holds {
    path: Arc<Path>,
    stamp: Stamp,
}

/// The code view and the Markdown view. See the module docs.
pub(crate) struct TextViews {
    code: Entity<EditorState>,
    markdown: Entity<TextViewState>,
    code_holds: Option<Holds>,
    markdown_holds: Option<Holds>,
    /// Whether the code view shows prose: wrapped, with no line numbers.
    code_prose: bool,
}

impl std::fmt::Debug for TextViews {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextViews")
            .field("code_holds", &self.code_holds)
            .field("markdown_holds", &self.markdown_holds)
            .finish_non_exhaustive()
    }
}

impl TextViews {
    pub(crate) fn new<T: 'static>(window: &mut Window, cx: &mut Context<T>) -> Self {
        let code = cx.new(|cx| {
            EditorState::new(window, cx)
                .line_number(true)
                .soft_wrap(false)
                .searchable(false)
                .context_menu(false)
        });
        let markdown = cx.new(|cx| TextViewState::markdown("", cx).selectable(true));
        Self {
            code,
            markdown,
            code_holds: None,
            markdown_holds: None,
            code_prose: false,
        }
    }

    /// Whether the code view shows prose: wrapped, with no line numbers.
    pub(crate) fn code_wraps(&self) -> bool {
        self.code_prose
    }

    /// Puts `text`, the file at `path` as stamped, in the code view, with
    /// highlighting for `language`: an extension or a file name. `prose`
    /// wraps the lines and numbers none. Showing the version it holds again
    /// keeps where it was scrolled.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call site; the file's parts travel together from the read"
    )]
    pub(crate) fn show_code<T: 'static>(
        &mut self,
        path: &Arc<Path>,
        stamp: Stamp,
        text: &SharedString,
        language: &str,
        prose: bool,
        window: &mut Window,
        cx: &mut Context<T>,
    ) {
        let holds = Holds {
            path: Arc::clone(path),
            stamp,
        };
        if self.code_holds.as_ref() == Some(&holds) {
            return;
        }
        self.code_holds = Some(holds);
        let language: SharedString = language.to_owned().into();
        let wrap_changes = prose != self.code_prose;
        self.code_prose = prose;
        self.code.update(cx, |editor, cx| {
            if wrap_changes {
                editor.set_line_number(!prose, window, cx);
                editor.set_soft_wrap(prose, window, cx);
            }
            // The text goes in as plain text, then the language: the Kit's
            // editor parses a document of up to 64 KB at once, on this
            // thread, both when the text is set and on the next frame, so
            // with the language first it parsed every file twice.
            editor.set_highlighter(PLAIN, cx);
            // Starts at the top, with no selection.
            editor.set_value(text, window, cx);
            editor.set_highlighter(language, cx);
        });
    }

    /// Puts `text` in the Markdown view, scrolled to the top. Showing the
    /// version it holds again keeps where it was scrolled.
    pub(crate) fn show_markdown<T: 'static>(
        &mut self,
        path: &Arc<Path>,
        stamp: Stamp,
        text: &SharedString,
        cx: &mut Context<T>,
    ) {
        let holds = Holds {
            path: Arc::clone(path),
            stamp,
        };
        if self.markdown_holds.as_ref() == Some(&holds) {
            return;
        }
        self.markdown_holds = Some(holds);
        self.markdown.update(cx, |markdown, cx| {
            markdown.set_text(text, cx);
            markdown.list_state().scroll_to(ListOffset::default());
        });
    }

    /// Scrolls the code view.
    pub(crate) fn scroll_code<T: 'static>(&self, scroll: Scroll, cx: &mut Context<T>) {
        self.code.update(cx, |editor, cx| {
            let offset = editor.scroll_offset();
            let page = editor.input_bounds().size.height;
            editor.set_scroll_offset(point(offset.x, scroll.from(offset.y, page)), cx);
        });
    }

    /// Scrolls the Markdown view.
    pub(crate) fn scroll_markdown<T: 'static>(&self, scroll: Scroll, cx: &mut Context<T>) {
        self.markdown.update(cx, |markdown, cx| {
            let list = markdown.list_state();
            let page = list.viewport_bounds().size.height;
            let y = list.scroll_px_offset_for_scrollbar().y;
            scroll_list(list, scroll.from(y, page) - y);
            cx.notify();
        });
    }

    /// How far the code view is scrolled down.
    pub(crate) fn code_scroll(&self, cx: &App) -> Pixels {
        -self.code.read(cx).scroll_offset().y
    }

    /// How far the Markdown view is scrolled down.
    pub(crate) fn markdown_scroll(&self, cx: &App) -> Pixels {
        -self
            .markdown
            .read(cx)
            .list_state()
            .scroll_px_offset_for_scrollbar()
            .y
    }

    /// The code view, labelled with the file's name. The Kit keeps a code
    /// editor's left padding to 6 px; 8 px more here puts the line numbers
    /// on the 16 px inset the Markdown column and the explorer share.
    pub(crate) fn render_code(&self, name: SharedString) -> impl IntoElement {
        div()
            .id("stage-code")
            .test_support()
            .size_full()
            .pl_2()
            .child(
                Editor::new(&self.code)
                    .readonly(true)
                    .appearance(false)
                    .large()
                    .aria_label(name)
                    .size_full(),
            )
    }

    /// The Markdown view, labelled with the file's name: the text view in
    /// a centred column, its scrolling taking the wheel over the margins.
    pub(crate) fn render_markdown(&self, name: SharedString) -> impl IntoElement {
        let markdown = self.markdown.clone();
        let file = self.markdown_holds.as_ref().map(|holds| holds.path.clone());
        div()
            .id("stage-markdown")
            .test_support()
            .role(Role::Document)
            .aria_label(name)
            .size_full()
            .flex()
            .justify_center()
            .on_scroll(move |event, window, cx| {
                // Over the column, the text view scrolls itself.
                if markdown.read(cx).bounds().contains(&event.position) {
                    return;
                }
                let delta = event.delta.pixel_delta(window.line_height()).y;
                markdown.update(cx, |markdown, cx| {
                    let list = markdown.list_state();
                    scroll_list(list, delta);
                    cx.notify();
                });
            })
            .child(
                TextView::new(&self.markdown)
                    .scrollable(true)
                    .selectable(true)
                    .on_link_click(open_link)
                    .image_source(move |url| {
                        let folder = file.as_deref().and_then(Path::parent);
                        image_source(folder, url)
                    })
                    // In place of the language label the Kit puts in each
                    // code block's corner: the stage adds no words of its
                    // own.
                    .code_block_actions(|_, _, _| div())
                    .h_full()
                    .w_full()
                    .max_w(MEASURE)
                    .px_8()
                    .pt_6(),
            )
    }
}

/// Whether a file is plain prose rather than code, by its name and its
/// lowercase extension: `.txt`, `.text` and `.log` files, and files with no
/// extension other than the build files the code view highlights by name
/// (`Makefile`, `Dockerfile` and the like). The code view wraps prose and
/// numbers none of its lines.
pub(crate) fn is_prose(name: &str, extension: &str) -> bool {
    const CODE_NAMES: [&str; 10] = [
        "makefile",
        "gnumakefile",
        "dockerfile",
        "justfile",
        "rakefile",
        "gemfile",
        "podfile",
        "procfile",
        "vagrantfile",
        "jenkinsfile",
    ];
    match extension {
        "txt" => !name.eq_ignore_ascii_case("cmakelists.txt"),
        "text" | "log" => true,
        "" => {
            !name.starts_with('.')
                && !CODE_NAMES
                    .iter()
                    .any(|code| name.eq_ignore_ascii_case(code))
        }
        _ => false,
    }
}

/// A click on a Markdown link: opens it as the Kit would by default, but
/// only a web or mail link. A file can hold any URL, and a `file://` URL
/// or an app's own scheme would open whatever it names, a script included.
fn open_link(url: &SharedString, event: &ClickEvent, _: &mut Window, cx: &mut App) {
    if is_activation(event) && is_web_or_mail(url) {
        cx.open_url(url);
    }
}

/// Whether a click activates a link: the clicks the Kit's own handler
/// opens a link on.
fn is_activation(event: &ClickEvent) -> bool {
    match event {
        ClickEvent::Mouse(click) => {
            matches!(click.up.button, MouseButton::Left | MouseButton::Middle)
        }
        ClickEvent::Keyboard(_) => true,
        ClickEvent::Touch(click) => !click.long_press,
    }
}

/// Whether `url` is an `http`, `https` or `mailto` link, the scheme in any
/// case.
fn is_web_or_mail(url: &str) -> bool {
    url.split_once(':').is_some_and(|(scheme, rest)| {
        !rest.is_empty()
            && ["http", "https", "mailto"]
                .iter()
                .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
    })
}

/// Where the Markdown view loads the image at `url` from, for a file in
/// `folder`. See the module docs.
fn image_source(folder: Option<&Path>, url: &SharedUri) -> ImageSource {
    if let Some(image) = data_url_image(url) {
        return ImageSource::Image(image);
    }
    match folder.zip(relative_path(url)) {
        Some((folder, path)) => folder.join(path).into(),
        None => url.clone().into(),
    }
}

/// The path in `url` when it is a relative one: no scheme, no leading `/`
/// (a code host reads that from the repository's root, which a file does
/// not know), and something before any `?query` or `#fragment`. The path is
/// taken as written: `%20` is not turned into a space.
fn relative_path(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let first = path.split('/').next().unwrap_or(path);
    (!path.is_empty() && !path.starts_with('/') && !first.contains(':')).then_some(path)
}

/// The image a `data:` URL carries, as the Kit decodes one when no image
/// source is set. `None` for any other URL and for one that is not an image
/// GPUI can decode.
fn data_url_image(url: &str) -> Option<Arc<Image>> {
    let data = data_url::DataUrl::process(url).ok()?;
    let mime = data.mime_type();
    let format = ImageFormat::from_mime_type(&format!("{}/{}", mime.type_, mime.subtype))?;
    let (bytes, _fragment) = data.decode_to_vec().ok()?;
    Some(Arc::new(Image::from_bytes(format, bytes)))
}

/// Moves a list's scroll offset by `delta` (negative is down the text),
/// stopping at its top and its end.
fn scroll_list(list: &ListState, delta: Pixels) {
    let y = list.scroll_px_offset_for_scrollbar().y;
    let end = -list.max_offset_for_scrollbar().y;
    let to = (y + delta).clamp(end.min(px(0.)), px(0.));
    list.scroll_by(y - to);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{STEP, Scroll, image_source, is_prose, is_web_or_mail, relative_path};
    use gpui_kit::{ImageFormat, ImageSource, Pixels, Resource, SharedUri, px};

    #[test]
    fn only_a_relative_path_is_read_from_the_files_folder() {
        for (url, path) in [
            ("banner.webp", "banner.webp"),
            ("assets/readme/banner.webp", "assets/readme/banner.webp"),
            ("./a.png", "./a.png"),
            ("../shared/a.png", "../shared/a.png"),
            ("a.png?raw=true", "a.png"),
            ("a.svg#dark", "a.svg"),
            ("notes/12:30.png", "notes/12:30.png"),
        ] {
            assert_eq!(relative_path(url), Some(path), "{url}");
        }
        for url in [
            "https://example.com/a.png",
            "file:///etc/a.png",
            "data:image/png;base64,iVBORw0KGgo=",
            "/assets/a.png",
            "#heading",
            "?raw=true",
            "",
        ] {
            assert_eq!(relative_path(url), None, "{url}");
        }
    }

    #[test]
    fn a_markdown_image_comes_from_the_folder_its_own_data_or_nowhere() {
        let folder = Some(Path::new("/repo/docs"));
        let source = |folder, url: &str| image_source(folder, &SharedUri::from(url.to_owned()));

        let ImageSource::Resource(Resource::Path(path)) = source(folder, "img/a.png?raw=true")
        else {
            panic!("a relative path loads from a file");
        };
        assert_eq!(&*path, Path::new("/repo/docs/img/a.png"));

        let ImageSource::Image(image) = source(folder, "data:image/png;base64,iVBORw0KGgo=") else {
            panic!("a data URL carries its image");
        };
        assert_eq!(image.format(), ImageFormat::Png);
        assert_eq!(image.bytes(), b"\x89PNG\r\n\x1a\n");

        // Left to GPUI's loader, which has no HTTP client to fetch with.
        for (folder, url) in [
            (folder, "https://example.com/a.png"),
            (folder, "/etc/a.png"),
            (None, "a.png"),
        ] {
            assert!(
                matches!(source(folder, url), ImageSource::Resource(Resource::Uri(_))),
                "{url}"
            );
        }
    }

    #[test]
    fn prose_is_plain_text_and_names_without_an_extension() {
        for (name, extension) in [
            ("notes.txt", "txt"),
            ("README.TEXT", "text"),
            ("build.log", "log"),
            ("LICENSE", ""),
            ("README", ""),
        ] {
            assert!(is_prose(name, extension), "{name}");
        }
        for (name, extension) in [
            ("main.rs", "rs"),
            ("notes.md", "md"),
            ("CMakeLists.txt", "txt"),
            ("Makefile", ""),
            ("Dockerfile", ""),
            (".gitignore", ""),
            ("data.json", "json"),
        ] {
            assert!(!is_prose(name, extension), "{name}");
        }
    }

    #[test]
    fn only_web_and_mail_links_are_opened() {
        for url in [
            "https://example.com",
            "http://example.com/a?b",
            "HTTPS://EXAMPLE.COM",
            "mailto:someone@example.com",
        ] {
            assert!(is_web_or_mail(url), "{url}");
        }
        for url in [
            "file:///Applications/Calculator.app",
            "file:///tmp/x.command",
            "vscode://file/etc/passwd",
            "javascript:alert(1)",
            "ftp://example.com",
            "other.md",
            "#heading",
            "https:",
            " https://example.com",
            "",
        ] {
            assert!(!is_web_or_mail(url), "{url}");
        }
    }

    #[test]
    fn keys_scroll_by_a_step_or_a_page_and_stop_at_the_top() {
        let page = px(500.);
        assert_eq!(Scroll::Down.from(px(0.), page), -STEP);
        assert_eq!(Scroll::Up.from(-STEP, page), px(0.));
        // Never above the top.
        assert_eq!(Scroll::Up.from(px(-10.), page), px(0.));
        assert_eq!(Scroll::PageUp.from(px(-100.), page), px(0.));
        // A page keeps one step of the screen before.
        assert_eq!(Scroll::PageDown.from(px(0.), page), px(-460.));
        assert_eq!(Scroll::PageUp.from(px(-1000.), page), px(-540.));
        assert_eq!(Scroll::Top.from(px(-1000.), page), px(0.));
        // The bottom is the view's to clamp.
        assert_eq!(Scroll::Bottom.from(px(0.), page), Pixels::MIN);
        // A viewport smaller than a step still pages by a step.
        assert_eq!(Scroll::PageDown.from(px(0.), px(10.)), -STEP);
    }
}
