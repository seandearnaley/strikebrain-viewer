//! One row of the explorer tree.
//!
//! Brainstrike's measurements, on the rem scale so they follow the theme's
//! base font (16 px, where every value below is exact): 28 px tall plus a
//! 1 px rule, the panel's 8 px inset, 20 px of indent per level, a 24 px
//! chevron target, then 6 px gaps around a 16 px checkbox and a 16 px icon,
//! and the name at 14/16 px. The Type and Size cells follow as the tree
//! widens; see [`crate::columns`].

use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use explorer::{Check, Kind, NodeId, RowView, format_size};
use gpui_kit::base::{Checkbox, CheckboxIndicator, CheckboxState, RoleOverride};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex};
use gpui_kit::{
    AccessibleAction, App, FontFeatures, IntoElement, MouseButton, ParentElement as _, Rems,
    RenderOnce, Role, SharedString, StatefulInteractiveElement, Styled as _, TestSupportExt as _,
    Toggled, Window, container_query, div, prelude::*, rems,
};

use crate::columns::{self, Columns};
use crate::state::RowAction;

/// Indent per level below the top: 20 px.
const INDENT: Rems = Rems(1.25);

/// Tabular figures, so sizes line up digit under digit. Made once; each row
/// clones the `Arc`.
static TABULAR_FIGURES: LazyLock<FontFeatures> =
    LazyLock::new(|| FontFeatures(Arc::new(vec![("tnum".into(), 1)])));

/// What the chevron target holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disclosure {
    /// Nothing: a file, or a folder that cannot open.
    Leaf,
    Closed,
    Open,
    /// A warning: a folder that could not be read.
    Unreadable,
}

/// How the row shows keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Elsewhere,
    /// The focus row, before any key press: no ring.
    Here,
    /// The focus row with its ring.
    Ringed,
}

type RowHandler = Rc<dyn Fn(&RowAction, &mut Window, &mut App)>;

/// One row, built fresh each frame from the engine's row view.
#[derive(IntoElement)]
pub(crate) struct TreeRow {
    id: NodeId,
    depth: u16,
    kind: Kind,
    name: SharedString,
    /// The extension, for the Type cell; empty for folders.
    ext: SharedString,
    check: Check,
    disclosure: Disclosure,
    size: Option<u64>,
    current: bool,
    focus: Focus,
    on_event: RowHandler,
}

impl TreeRow {
    pub(crate) fn new(
        row: &RowView<'_>,
        on_event: impl Fn(&RowAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        let disclosure = match (row.kind, row.can_open, row.open) {
            (Kind::Unreadable, ..) => Disclosure::Unreadable,
            (_, true, true) => Disclosure::Open,
            (_, true, false) => Disclosure::Closed,
            (_, false, _) => Disclosure::Leaf,
        };
        Self {
            id: row.id,
            depth: row.depth,
            kind: row.kind,
            name: row.name.to_owned().into(),
            ext: row.ext.to_owned().into(),
            check: row.check,
            disclosure,
            size: row.size,
            current: false,
            focus: Focus::Elsewhere,
            on_event: Rc::new(on_event),
        }
    }

    /// The row of the file on the stage: accent background.
    pub(crate) fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }

    /// The keyboard focus row. `ring` draws the focus ring, which shows only
    /// after a key press.
    pub(crate) fn focused(mut self, focused: bool, ring: bool) -> Self {
        self.focus = match (focused, ring) {
            (false, _) => Focus::Elsewhere,
            (true, false) => Focus::Here,
            (true, true) => Focus::Ringed,
        };
        self
    }

    fn checkbox_state(&self) -> CheckboxState {
        match self.check {
            Check::Off => CheckboxState::Unchecked,
            Check::Mixed => CheckboxState::Indeterminate,
            Check::On => CheckboxState::Checked,
        }
    }
}

impl RenderOnce for TreeRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let (accent, muted, border, ring) = (theme.accent, theme.muted, theme.border, theme.ring);
        let toggled = match self.check {
            Check::Off => Toggled::False,
            Check::Mixed => Toggled::Mixed,
            Check::On => Toggled::True,
        };
        let on_click = Rc::clone(&self.on_event);
        let on_disclose = Rc::clone(&self.on_event);
        let (current, focus, disclosure) = (self.current, self.focus, self.disclosure);

        div()
            .id(("row", self.id.index()))
            .test_support()
            .role(Role::TreeItem)
            .aria_label(self.name.clone())
            .aria_level(usize::from(self.depth))
            .aria_toggled(toggled)
            .aria_selected(current)
            .when(
                focus != Focus::Elsewhere,
                StatefulInteractiveElement::aria_active_descendant,
            )
            .when(
                matches!(disclosure, Disclosure::Open | Disclosure::Closed),
                |row| {
                    let open = disclosure == Disclosure::Open;
                    row.aria_expanded(open).on_a11y_action(
                        if open {
                            AccessibleAction::Collapse
                        } else {
                            AccessibleAction::Expand
                        },
                        move |_, window, cx| on_disclose(&RowAction::ToggleOpen, window, cx),
                    )
                },
            )
            .relative()
            .w_full()
            .border_b_1()
            .border_color(border)
            .map(|row| {
                if current {
                    row.bg(accent)
                } else {
                    row.hover(|style| style.bg(muted.opacity(0.5)))
                }
            })
            .on_click(move |_, window, cx| on_click(&RowAction::Show, window, cx))
            .child(
                div()
                    .h_7()
                    .w_full()
                    .child(container_query(move |size, window, cx| {
                        self.body(Columns::for_width(size.width, window.rem_size()), cx)
                    })),
            )
            .when(focus == Focus::Ringed, |row| {
                row.child(
                    div()
                        .id("ring")
                        .test_support()
                        .absolute()
                        .inset_0()
                        .border_2()
                        .border_color(ring),
                )
            })
    }
}

impl TreeRow {
    /// Chevron, checkbox, icon and name, then the Type and Size cells that
    /// `columns` shows.
    fn body(self, columns: Columns, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let indent = INDENT * f32::from(self.depth.saturating_sub(1));
        let size = self.size;
        let mono = theme.mono_font_family.clone();
        let muted_foreground = theme.muted_foreground;
        let ext = self.ext.clone();

        columns::line()
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .pl(indent)
                    .gap_1p5()
                    .child(self.chevron(cx))
                    .child(self.checkbox(cx))
                    .child(self.icon(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .line_height(rems(1.))
                            .text_color(theme.foreground)
                            .child(self.name),
                    ),
            )
            .when(columns.extension, |line| {
                line.child(
                    columns::extension_cell()
                        .id("type")
                        .test_support()
                        .text_xs()
                        .text_color(muted_foreground)
                        .child(ext),
                )
            })
            .when(columns.size, |line| {
                line.child(
                    columns::size_cell()
                        .id("size")
                        .test_support()
                        .font_family(mono)
                        .font_features(TABULAR_FIGURES.clone())
                        .text_xs()
                        .text_color(muted_foreground)
                        .when_some(size, |cell, bytes| cell.child(format_size(bytes))),
                )
            })
    }

    /// The 24 px chevron target.
    fn chevron(&self, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let target = div()
            .id("chevron")
            .test_support()
            .size_6()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center();
        let chevron = match self.disclosure {
            Disclosure::Leaf => return target,
            Disclosure::Unreadable => {
                return target.child(
                    div().id("warning").test_support().child(
                        Icon::new(IconName::TriangleAlert)
                            .size_4()
                            .text_color(theme.warning),
                    ),
                );
            }
            Disclosure::Closed => IconName::ChevronRight,
            Disclosure::Open => IconName::ChevronDown,
        };
        let on_event = Rc::clone(&self.on_event);
        target
            .rounded(theme.radius)
            .hover(|style| style.bg(theme.muted))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_event(&RowAction::ToggleOpen, window, cx);
            })
            .child(
                Icon::new(chevron)
                    .size_3()
                    .text_color(theme.muted_foreground),
            )
    }

    /// The Kit's Base checkbox, drawn as the spec asks: primary when checked
    /// or mixed with a white mark, the background with a primary border when
    /// not. A click does not take focus from the tree.
    fn checkbox(&self, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let state = self.checkbox_state();
        let mark = match state {
            CheckboxState::Checked => Some(IconName::Check),
            CheckboxState::Indeterminate => Some(IconName::Minus),
            CheckboxState::Unchecked => None,
        };
        let on_event = Rc::clone(&self.on_event);
        // The row carries the name and the checked state; the checkbox is
        // drawn, not announced a second time.
        Checkbox::new("check")
            .role(RoleOverride::Presentational)
            .state(state)
            .tab_stop(false)
            .on_change(move |_, _, window, cx| {
                cx.stop_propagation();
                on_event(&RowAction::ToggleCheck, window, cx);
            })
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .flex_shrink_0()
            .child(
                CheckboxIndicator::new()
                    .state(state)
                    .size_4()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme.primary)
                    .bg(theme.background)
                    .styles(|styles| {
                        styles
                            .checked(|style| style.bg(theme.primary))
                            .indeterminate(|style| style.bg(theme.primary))
                    })
                    .when_some(mark, |indicator, mark| {
                        indicator.child(
                            Icon::new(mark)
                                .size_3()
                                .text_color(theme.primary_foreground),
                        )
                    }),
            )
    }

    /// Folder, open folder, file or symlink, 16 px.
    fn icon(&self, cx: &App) -> Icon {
        let theme = cx.theme();
        match self.kind {
            Kind::Dir | Kind::Unreadable => Icon::new(if self.disclosure == Disclosure::Open {
                IconName::FolderOpen
            } else {
                IconName::Folder
            })
            .text_color(theme.info),
            Kind::File => Icon::new(IconName::File).text_color(theme.muted_foreground),
            Kind::Symlink => Icon::new(gpui_kit::assets::IconName::FileSymlink)
                .text_color(theme.muted_foreground),
        }
        .size_4()
    }
}
