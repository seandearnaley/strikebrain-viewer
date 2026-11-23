//! Drawing the stage: the file in its viewer, a file it cannot show, and
//! the position in the gallery.

use std::path::Path;
use std::sync::Arc;

use explorer::format_size;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Hsla, IntoElement, ObjectFit, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    TestSupportExt as _, Window, div, img, prelude::*,
};

use crate::keys::{
    PRESENTATION, STAGE, ScrollDown, ScrollPageDown, ScrollPageUp, ScrollToBottom, ScrollToTop,
    ScrollUp,
};
use crate::theme::TABULAR_FIGURES;

use super::text::Scroll;
use super::{Content, File, Stage};

impl Stage {
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

    /// Whether the presentation shows a picture.
    fn presents_image(&self) -> bool {
        self.asked.presenting && matches!(self.content, Content::Image(..))
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
            // A presented picture sits on black in either mode, as in a
            // photo viewer; everything else on the stage's surface.
            .bg(if self.presents_image() {
                gpui_kit::black()
            } else {
                theme.background
            })
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
