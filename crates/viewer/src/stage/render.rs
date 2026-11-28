//! Drawing the stage: the file in its viewer, a file it cannot show, and
//! the position in the gallery.

use std::path::Path;

use explorer::format_size;
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Hsla, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div, prelude::*,
};

use crate::keys::{
    FocusFirstTile, FocusLastTile, FocusNextTile, FocusPreviousTile, FocusTileAbove,
    FocusTileBelow, FocusTilePageDown, FocusTilePageUp, GALLERY, OpenTile, PRESENTATION, STAGE,
    ScrollDown, ScrollPageDown, ScrollPageUp, ScrollToBottom, ScrollToTop, ScrollUp, ShowGallery,
};
use crate::state::TileMotion;
use crate::theme::TABULAR_FIGURES;

use super::text::Scroll;
use super::{Content, Gallery, Stage};

impl Stage {
    fn render_content(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(match &self.content {
            Content::Blank => return None,
            Content::Image(file, len) => self.render_image(file, *len, window, cx),
            Content::Markdown(file) => self
                .text_views
                .render_markdown(file.name.clone(), cx.weak_entity())
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

    /// Whether the presentation shows a picture.
    fn presents_image(&self) -> bool {
        self.asked.presenting && matches!(self.content, Content::Image(..))
    }

    /// The bar under the file: its place in the gallery, quiet and
    /// centred, and left of it the grid button back to the gallery while
    /// two or more files are checked.
    fn render_bar(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let position = self.asked.position.map(|(at, of)| {
            let text: SharedString = format!("{at} / {of}").into();
            div()
                .id("stage-position")
                .test_support()
                .role(Role::Label)
                .aria_label(text.clone())
                .font_family(theme.mono_font_family.clone())
                .font_features(TABULAR_FIGURES.clone())
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        });
        let back = self.asked.back.map(|destination| {
            let (label, icon) = if destination == crate::StageView::Folder {
                ("Back to Folder", AssetIcon::Folder)
            } else {
                ("Gallery", AssetIcon::LayoutGrid)
            };
            Button::new("gallery-button")
                .icon(Icon::new(icon).text_color(theme.muted_foreground))
                .ghost()
                .compact()
                .small()
                .tooltip(label)
                .accessibility_label(label)
                .on_click(cx.listener(|this, _, window, cx| this.show_gallery(false, window, cx)))
        });
        // Either side of the position takes half of what is left, so the
        // position stays centred with the button against it.
        h_flex()
            .flex_shrink_0()
            .h_8()
            .w_full()
            .gap_2()
            .child(h_flex().flex_1().justify_end().children(back))
            .children(position)
            .child(div().flex_1())
    }

    /// Hands a key the gallery acts on to it.
    fn gallery_key(&mut self, motion: TileMotion, cx: &mut Context<Self>) {
        self.gallery
            .update(cx, |gallery, cx| gallery.move_focus(motion, cx));
    }
}

/// A number for a path, to key an element by its file.
pub(super) fn path_key(path: &Path) -> usize {
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
pub(super) fn unsupported(name: SharedString, size: Option<u64>, muted: Hsla) -> AnyElement {
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let gallery = self.asked.gallery || self.asked.folder.is_some();
        let content = if gallery {
            Some(self.gallery.clone().into_any_element())
        } else {
            self.render_content(window, cx)
        };
        let bar = (self.asked.position.is_some() || self.asked.back.is_some())
            .then(|| self.render_bar(cx));
        let theme = cx.theme();

        v_flex()
            .id("stage")
            .test_support()
            .track_focus(&self.focus_handle)
            .key_context(if self.asked.presenting {
                PRESENTATION
            } else if gallery {
                GALLERY
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
            .on_action(cx.listener(|this, _: &ShowGallery, window, cx| {
                this.show_gallery(true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusPreviousTile, _, cx| {
                this.gallery_key(TileMotion::Previous, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusNextTile, _, cx| {
                this.gallery_key(TileMotion::Next, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusTileAbove, _, cx| {
                this.gallery_key(TileMotion::Up, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusTileBelow, _, cx| {
                this.gallery_key(TileMotion::Down, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusFirstTile, _, cx| {
                this.gallery_key(TileMotion::First, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusLastTile, _, cx| {
                this.gallery_key(TileMotion::Last, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusTilePageUp, _, cx| {
                // The gallery knows how many rows a page holds.
                this.gallery_key(TileMotion::PageUp(1), cx);
            }))
            .on_action(cx.listener(|this, _: &FocusTilePageDown, _, cx| {
                this.gallery_key(TileMotion::PageDown(1), cx);
            }))
            .on_action(cx.listener(|this, _: &OpenTile, _, cx| {
                this.gallery.update(cx, Gallery::open_focused);
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
            .children(bar)
    }
}
