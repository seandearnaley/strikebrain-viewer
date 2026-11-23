//! One row of the explorer tree.
//!
//! Brainstrike's measurements, on the rem scale so they follow the theme's
//! base font (16 px, where every value below is exact): 28 px tall plus a
//! 1 px rule, the panel's 8 px inset, a 24 px chevron target, then 6 px
//! gaps around a 16 px checkbox and a 16 px icon, and the name at 14/16 px.
//! The Type and Size cells follow as the tree widens; see
//! [`crate::columns`]. A top-level folder's row shows a remove button over
//! the Size cell while it is hovered or focused, and its full path as a
//! tooltip.
//!
//! The indent is not brainstrike's: 12 px per level, half the chevron's
//! target, where brainstrike has 20 px. At the default 300 px width a name
//! five levels down keeps about 80 px, where 20 px left it 50 px: too few
//! characters to tell files apart.
//!
//! In light mode hover is the muted background at full strength, not the
//! spec's half, which does not show on the light background. The focus ring is the theme's ring colour (brainstrike's
//! foreground at half strength), 2 px with a 4 px radius, kept 2 px in from
//! the pane's edge and the divider.

use std::rc::Rc;

use explorer::{
    Check, Kind, NodeId, RowView, format_size, is_binary_extension, is_image_extension,
};
use gpui_kit::assets::IconName as AssetIcon;
use gpui_kit::base::{Checkbox, CheckboxIndicator, CheckboxState, RoleOverride};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::{
    AccessibleAction, App, Font, IntoElement, MouseButton, ParentElement as _, Pixels, Rems,
    RenderOnce, Role, SharedString, StatefulInteractiveElement, Styled as _, TestSupportExt as _,
    Toggled, Window, container_query, div, font, prelude::*, rems,
};

use crate::columns::{self, Columns};
use crate::state::RowAction;
use crate::theme::TABULAR_FIGURES;

/// Indent per level below the top: 12 px. See the module docs.
const INDENT: Rems = Rems(0.75);

/// How far the focus ring stays in from the pane's edge and the divider:
/// 2 px.
const RING_INSET: Rems = Rems(0.125);

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
type HoverHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

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
    /// A top-level folder's full path, and why it could not be read.
    tooltip: Option<SharedString>,
    /// Whether the remove button shows: a top-level folder's row, hovered
    /// or focused.
    removable: bool,
    on_event: RowHandler,
    on_hover: Option<HoverHandler>,
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
            tooltip: None,
            removable: false,
            on_event: Rc::new(on_event),
            on_hover: None,
        }
    }

    /// The row's tooltip, which is also its accessible description: a
    /// top-level folder's full path, and why it could not be read.
    pub(crate) fn tooltip(mut self, tooltip: Option<SharedString>) -> Self {
        self.tooltip = tooltip;
        self
    }

    /// Shows the remove button over the Size cell: a top-level folder's
    /// row while it is hovered or focused.
    pub(crate) fn removable(mut self, removable: bool) -> Self {
        self.removable = removable;
        self
    }

    /// Hears the pointer enter and leave the row.
    pub(crate) fn on_hover(
        mut self,
        handler: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_hover = Some(Rc::new(handler));
        self
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
        let (accent, border, ring) = (theme.accent, theme.border, theme.ring);
        // The spec's half-strength muted is 2.5 % lightness from the light
        // background and does not show; full strength does. In dark mode half
        // strength shows, and full strength would come close to the current
        // row's accent.
        let hover = if theme.is_dark() {
            theme.muted.opacity(0.5)
        } else {
            theme.muted
        };
        let toggled = match self.check {
            Check::Off => Toggled::False,
            Check::Mixed => Toggled::Mixed,
            Check::On => Toggled::True,
        };
        let on_click = Rc::clone(&self.on_event);
        let on_disclose = Rc::clone(&self.on_event);
        let (current, focus, disclosure) = (self.current, self.focus, self.disclosure);
        let tooltip = self.tooltip.clone();
        let on_hover = self.on_hover.clone();

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
                    row.hover(|style| style.bg(hover))
                }
            })
            .on_click(move |_, window, cx| on_click(&RowAction::Show, window, cx))
            .when_some(on_hover, |row, on_hover| {
                row.on_hover(move |hovered, window, cx| on_hover(hovered, window, cx))
            })
            .when_some(tooltip, |row, tooltip| {
                row.aria_description(tooltip.clone())
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            })
            .child(
                div()
                    .h_7()
                    .w_full()
                    .child(container_query(move |size, window, cx| {
                        let columns = Columns::for_width(size.width, window.rem_size());
                        let shifts = BaselineShifts::new(window, cx);
                        self.body(columns, shifts, cx)
                    })),
            )
            .when(focus == Focus::Ringed, |row| {
                row.child(
                    div()
                        .id("ring")
                        .test_support()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(RING_INSET)
                        .right(RING_INSET)
                        .rounded_sm()
                        .border_2()
                        .border_color(ring),
                )
            })
    }
}

/// How far to move the 12 px Type and Size text down so it sits on the
/// 14 px name's baseline. Each is centred in the row in a 16 px line box,
/// where its baseline falls at half the box plus half of ascent minus
/// descent, so smaller text lands higher: about 0.7 px for the Type and a
/// pixel for the monospace Size. GPUI's layout has no baseline alignment,
/// so the gaps are measured from the fonts.
#[derive(Clone, Copy, Debug)]
struct BaselineShifts {
    extension: Pixels,
    size: Pixels,
}

impl BaselineShifts {
    fn new(window: &Window, cx: &App) -> Self {
        let rem = window.rem_size();
        let line = rems(1.).to_pixels(rem);
        let small = rems(0.75).to_pixels(rem);
        let text = window.text_system();
        let ui = window.text_style().font();
        let name = text.resolve_font(&ui);
        let baseline = text.baseline_offset(name, rems(0.875).to_pixels(rem), line);
        let below =
            |family: Font| baseline - text.baseline_offset(text.resolve_font(&family), small, line);
        Self {
            extension: below(ui),
            size: below(font(cx.theme().mono_font_family.clone())),
        }
    }
}

impl TreeRow {
    /// Chevron, checkbox, icon and name, then the Type and Size cells that
    /// `columns` shows.
    fn body(self, columns: Columns, shifts: BaselineShifts, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let indent = INDENT * f32::from(self.depth.saturating_sub(1));
        let size = self.size;
        let mono = theme.mono_font_family.clone();
        let muted_foreground = theme.muted_foreground;
        let ext = self.ext.clone();
        let remove = self.removable.then(|| self.remove_button(cx));

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
                        .line_height(rems(1.))
                        .relative()
                        .top(shifts.extension)
                        .text_color(muted_foreground)
                        .child(ext),
                )
            })
            .map(|line| match (remove, columns.size) {
                // Over the Size cell, in place of the size.
                (Some(remove), true) => line.child(columns::size_cell().child(remove)),
                (Some(remove), false) => line.child(div().flex_shrink_0().child(remove)),
                (None, true) => line.child(
                    columns::size_cell()
                        .id("size")
                        .test_support()
                        .font_family(mono)
                        .font_features(TABULAR_FIGURES.clone())
                        .text_xs()
                        .line_height(rems(1.))
                        .relative()
                        .top(shifts.size)
                        .text_color(muted_foreground)
                        .when_some(size, |cell, bytes| cell.child(format_size(bytes))),
                ),
                (None, false) => line,
            })
    }

    /// The remove button of a top-level folder's row: a 24 px ghost icon
    /// button, muted as the chevrons are. A press does not reach the row.
    fn remove_button(&self, cx: &App) -> Button {
        let on_event = Rc::clone(&self.on_event);
        Button::new("remove-folder")
            .icon(Icon::new(IconName::Close).text_color(cx.theme().muted_foreground))
            .ghost()
            .compact()
            .small()
            .tooltip("Remove Folder")
            .accessibility_label("Remove Folder")
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                on_event(&RowAction::Remove, window, cx);
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

    /// Folder, open folder, symlink, or a file by its kind, 16 px. Files
    /// stay quiet, in the muted colour.
    fn icon(&self, cx: &App) -> Icon {
        let theme = cx.theme();
        match self.kind {
            Kind::Dir | Kind::Unreadable => Icon::new(if self.disclosure == Disclosure::Open {
                IconName::FolderOpen
            } else {
                IconName::Folder
            })
            .text_color(theme.info),
            Kind::File => Icon::new(file_icon(&self.ext)).text_color(theme.muted_foreground),
            Kind::Symlink => Icon::new(AssetIcon::FileSymlink).text_color(theme.muted_foreground),
        }
        .size_4()
    }
}

/// A file's icon by its lowercase extension, in the kinds the stage tells
/// apart: a picture, Markdown, other text or code, and anything else (a
/// known binary format), as the Images and Text kinds sort them.
fn file_icon(extension: &str) -> AssetIcon {
    if is_image_extension(extension) {
        AssetIcon::FileImage
    } else if matches!(extension, "md" | "markdown") {
        AssetIcon::FileText
    } else if is_binary_extension(extension) {
        AssetIcon::File
    } else {
        AssetIcon::FileCode
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::assets::IconName as AssetIcon;

    use super::file_icon;

    #[test]
    fn a_file_icon_says_its_kind() {
        assert_eq!(file_icon("png"), AssetIcon::FileImage);
        assert_eq!(file_icon("svg"), AssetIcon::FileImage);
        assert_eq!(file_icon("md"), AssetIcon::FileText);
        assert_eq!(file_icon("rs"), AssetIcon::FileCode);
        assert_eq!(file_icon("txt"), AssetIcon::FileCode);
        assert_eq!(file_icon(""), AssetIcon::FileCode);
        assert_eq!(file_icon("zip"), AssetIcon::File);
    }
}
