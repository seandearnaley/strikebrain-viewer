//! The stage's text views: code in the Kit's editor, Markdown in the Kit's
//! text view. Each is one entity, made once and given each file's text in
//! turn, so its layout caches and highlighter outlive a file change.
//!
//! The code view is read-only, with line numbers, `syntect` highlighting
//! picked by the file's extension (or its name, for `Makefile` and the
//! like) and no soft wrap: long lines scroll sideways. Search and the
//! context menu are off: the stage shows code, it does not edit it.
//! Folding stays on: blocks fold from chevrons that show only while the
//! pointer is over the gutter, and their column is the gap that keeps the
//! line numbers off the code.
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
    App, AppContext as _, Context, Entity, IntoElement, ListOffset, ListState, ParentElement as _,
    Pixels, Rems, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div, point, prelude::*, px,
};

use super::load::Stamp;

/// How far ↑ and ↓ scroll: 40 px, a browser's arrow step.
const STEP: Pixels = px(40.);

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
        }
    }

    /// Puts `text`, the file at `path` as stamped, in the code view, with
    /// highlighting for `language`: an extension or a file name. Showing
    /// the version it holds again keeps where it was scrolled.
    pub(crate) fn show_code<T: 'static>(
        &mut self,
        path: &Arc<Path>,
        stamp: Stamp,
        text: &SharedString,
        language: &str,
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
        self.code.update(cx, |editor, cx| {
            editor.set_highlighter(language, cx);
            // Starts at the top, with no selection.
            editor.set_value(text, window, cx);
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
    use super::{STEP, Scroll};
    use gpui_kit::{Pixels, px};

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
