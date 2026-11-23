//! Brainstrike's colours, installed as the Kit's light and dark themes.
//!
//! The Kit derives every other colour (hover and active shades, button,
//! list and scrollbar colours) from the ones set here. The app follows the
//! system appearance: [`install`] picks the theme for it at launch and
//! [`follow_system`] switches when it changes.

use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui_kit::component::highlighter::{HighlightTheme, HighlightThemeStyle, ThemeStyle};
use gpui_kit::component::{Colorize as _, Theme, ThemeConfig, ThemeConfigColors, ThemeMode, hsl};
use gpui_kit::{App, FontFeatures, Hsla, SharedString, Window};

/// Tabular figures, so numbers line up digit under digit: sizes in the
/// tree, the stage's size and position. Made once; each use clones the
/// `Arc`.
pub(crate) static TABULAR_FIGURES: LazyLock<FontFeatures> =
    LazyLock::new(|| FontFeatures(Arc::new(vec![("tnum".into(), 1)])));

/// One colour as `(hue, saturation %, lightness %)`, light then dark.
type Pair = ((f32, f32, f32), (f32, f32, f32));

// The six tokens the design spec lists.
const BACKGROUND: Pair = ((30., 25., 99.), (30., 8., 9.));
const PRIMARY: Pair = ((215., 55., 45.), (215., 50., 58.));
const MUTED: Pair = ((30., 15., 94.), (30., 5., 18.));
const ACCENT: Pair = ((30., 12., 90.), (30., 5., 22.));
const BORDER: Pair = ((30., 10., 90.), (30., 5., 20.));
/// Folder icons.
const INFO: Pair = ((215., 50., 38.), (215., 45., 74.));

// The rest of brainstrike's theme file (`packages/ui/src/themes/brainstrike.css`),
// so text, rings and warnings match the six above.
const FOREGROUND: Pair = ((25., 20., 14.), (30., 10., 93.));
const PRIMARY_FOREGROUND: Pair = ((0., 0., 100.), (0., 0., 100.));
const SECONDARY: Pair = ((30., 20., 97.), (30., 5., 15.));
const MUTED_FOREGROUND: Pair = ((25., 8., 42.), (30., 6., 59.));
/// How strong the focus colour is: brainstrike outlines a focused row in
/// its foreground at half strength (`outline-foreground/50`). The ring token
/// is that colour, with its alpha. The Kit draws a focused input as a border
/// in the ring colour plus a 3 px band in the same colour at alpha 0.5, which
/// replaces the colour's own alpha; at 0.5 the two bands are one colour, so
/// the input's ring reads as one soft ring, the tree's ring's twin.
/// Brainstrike's opaque `hsl(25 12% 38%)` made a heavy double ring.
const RING_ALPHA: f32 = 0.5;
/// The kind control's track. The Kit draws the chosen segment in the
/// background colour, so the track sits below it: muted in light mode, and
/// darker than the background in dark mode, where muted would make the
/// chosen segment look sunken.
const SEGMENTED_TRACK: Pair = ((30., 15., 94.), (30., 8., 5.));
const WARNING: Pair = ((30., 90., 29.), (38., 88., 60.));
const DANGER: Pair = ((8., 55., 39.), (8., 50., 75.));
const POPOVER: Pair = ((0., 0., 100.), (30., 6., 14.));
/// The primary button: white text on primary blue at 4.5:1 or more. Dark
/// mode's primary is too light for white text (3.7:1), so its buttons take
/// light mode's blue, a step darker on hover and press.
const BUTTON_PRIMARY: Pair = ((215., 55., 45.), (215., 55., 45.));
const BUTTON_PRIMARY_HOVER: Pair = ((215., 55., 40.), (215., 55., 40.));
const BUTTON_PRIMARY_ACTIVE: Pair = ((215., 55., 36.), (215., 55., 36.));

// Code, in the stage's code view and in Markdown code blocks: quiet colours
// on brainstrike's warm neutrals, the keywords in its primary blue. The Kit's
// default light palette puts comments in a bright blue.
const COMMENT: Pair = ((30., 8., 50.), (30., 6., 52.));
const KEYWORD: Pair = ((215., 55., 42.), (215., 60., 72.));
const STRING: Pair = ((135., 35., 30.), (105., 30., 66.));
const FUNCTION: Pair = ((260., 32., 45.), (255., 50., 80.));
const TYPE: Pair = ((185., 45., 30.), (180., 35., 66.));
const NUMBER: Pair = ((25., 70., 40.), (32., 70., 66.));
const SPECIAL: Pair = ((8., 50., 45.), (8., 55., 72.));
const ATTRIBUTE: Pair = ((38., 45., 35.), (38., 45., 62.));

/// Installs brainstrike's light and dark themes and applies the one that
/// matches the system appearance. Call once, after `gpui_kit::init`.
pub(crate) fn install(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.light_theme = Rc::new(config(ThemeMode::Light));
        theme.dark_theme = Rc::new(config(ThemeMode::Dark));
    });
    Theme::change(cx.window_appearance(), None, cx);
}

/// Switches to the theme for the window's current appearance.
pub(crate) fn follow_system(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
}

fn config(mode: ThemeMode) -> ThemeConfig {
    let pick_alpha = |(light, dark): Pair, alpha: f32| -> Option<SharedString> {
        let (h, s, l) = if mode.is_dark() { dark } else { light };
        Some(hsl(h, s, l).opacity(alpha).to_hex().into())
    };
    let pick = |pair: Pair| pick_alpha(pair, 1.);

    let mut colors = ThemeConfigColors::default();
    colors.background = pick(BACKGROUND);
    colors.foreground = pick(FOREGROUND);
    colors.primary = pick(PRIMARY);
    colors.primary_foreground = pick(PRIMARY_FOREGROUND);
    colors.secondary = pick(SECONDARY);
    colors.secondary_foreground = pick(FOREGROUND);
    colors.muted = pick(MUTED);
    colors.muted_foreground = pick(MUTED_FOREGROUND);
    colors.accent = pick(ACCENT);
    colors.accent_foreground = pick(FOREGROUND);
    colors.border = pick(BORDER);
    colors.input = pick(BORDER);
    colors.ring = pick_alpha(FOREGROUND, RING_ALPHA);
    colors.tab_bar_segmented = pick(SEGMENTED_TRACK);
    // Unchosen segments muted, the chosen one in the text colour.
    colors.tab_foreground = pick(MUTED_FOREGROUND);
    colors.tab_active_foreground = pick(FOREGROUND);
    colors.info = pick(INFO);
    colors.warning = pick(WARNING);
    colors.danger = pick(DANGER);
    colors.popover = pick(POPOVER);
    colors.popover_foreground = pick(FOREGROUND);
    if mode.is_dark() {
        // Light mode derives the same colours from its own primary.
        colors.button_primary = pick(BUTTON_PRIMARY);
        colors.button_primary_hover = pick(BUTTON_PRIMARY_HOVER);
        colors.button_primary_active = pick(BUTTON_PRIMARY_ACTIVE);
        colors.button_primary_foreground = pick(PRIMARY_FOREGROUND);
    }

    let mut config = ThemeConfig::default();
    config.highlight = Some(highlight(mode));
    config.name = if mode.is_dark() {
        "Brainstrike Dark".into()
    } else {
        "Brainstrike Light".into()
    };
    config.mode = mode;
    config.colors = colors;
    config
}

/// The syntax colours for `mode`, over the Kit's defaults for it. Each
/// theme carries its own, so code turns dark with the rest of the window;
/// a theme without one keeps whichever palette came before.
fn highlight(mode: ThemeMode) -> HighlightThemeStyle {
    let pick = |(light, dark): Pair| -> Hsla {
        let (h, s, l) = if mode.is_dark() { dark } else { light };
        hsl(h, s, l)
    };
    let defaults = if mode.is_dark() {
        HighlightTheme::default_dark()
    } else {
        HighlightTheme::default_light()
    };
    let mut style = defaults.style.clone();
    // The editor sits on the stage's own background, gutter included.
    style.editor_background = Some(pick(BACKGROUND));
    style.editor_gutter_background = Some(pick(BACKGROUND));
    style.editor_foreground = Some(pick(FOREGROUND));
    // A viewer has no line being edited: no band behind the caret's line.
    style.editor_active_line = None;
    // Nor a line whose number stands out: every number is muted. GPUI Base
    // 0.10.0 does not read the active line's number colour yet: it draws
    // the caret's line number in the theme's foreground, so line 1 of a
    // file just opened stands out all the same (see the spec's Risks).
    style.editor_line_number = Some(pick(MUTED_FOREGROUND));
    style.editor_active_line_number = Some(pick(MUTED_FOREGROUND));

    let syntax = &mut style.syntax;
    // `ThemeStyle` cannot be made outside the Kit, so a style the defaults
    // leave out starts as a copy of one they set.
    syntax.comment_doc = syntax.comment_doc.or(syntax.comment);
    syntax.boolean = syntax.boolean.or(syntax.number);
    syntax.string_escape = syntax.string_escape.or(syntax.number);
    syntax.preproc = syntax.preproc.or(syntax.attribute);
    let paint = |style: &mut Option<ThemeStyle>, pair: Pair| {
        if let Some(style) = style {
            style.color = Some(pick(pair));
        }
    };
    paint(&mut syntax.comment, COMMENT);
    paint(&mut syntax.comment_doc, COMMENT);
    paint(&mut syntax.keyword, KEYWORD);
    paint(&mut syntax.tag, KEYWORD);
    paint(&mut syntax.string, STRING);
    paint(&mut syntax.string_escape, NUMBER);
    paint(&mut syntax.function, FUNCTION);
    paint(&mut syntax.type_, TYPE);
    paint(&mut syntax.number, NUMBER);
    paint(&mut syntax.boolean, NUMBER);
    paint(&mut syntax.constant, NUMBER);
    paint(&mut syntax.variable_special, SPECIAL);
    paint(&mut syntax.attribute, ATTRIBUTE);
    paint(&mut syntax.preproc, ATTRIBUTE);
    paint(&mut syntax.property, FOREGROUND);
    paint(&mut syntax.title, FOREGROUND);
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_mode_takes_its_own_half_of_a_pair() {
        let light = config(ThemeMode::Light);
        let dark = config(ThemeMode::Dark);

        assert_eq!(light.colors.background.as_deref(), Some("#FDFCFB"));
        assert_eq!(
            dark.colors.background,
            Some(hsl(30., 8., 9.).to_hex().into())
        );
        assert!(dark.mode.is_dark());
        assert!(!light.mode.is_dark());
    }

    #[test]
    fn the_ring_is_the_foreground_at_half_strength() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let colors = config(mode).colors;
            let parse = |hex: Option<SharedString>| {
                Hsla::parse_hex(&hex.expect("a colour")).expect("a hex colour")
            };
            let (ring, foreground) = (parse(colors.ring), parse(colors.foreground));
            assert!((ring.a - RING_ALPHA).abs() < 0.01, "{mode:?}: {ring:?}");
            assert_eq!(ring.alpha(1.).to_hex(), foreground.to_hex(), "{mode:?}");
        }
    }

    /// WCAG 2 contrast between two opaque colours.
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let luminance = |color: Hsla| {
            let rgb = color.to_rgb();
            let linear = |c: f32| {
                if c <= 0.039_28 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn a_primary_button_reads_at_four_and_a_half_to_one() {
        let parse = |hex: Option<SharedString>| {
            Hsla::parse_hex(&hex.expect("a colour")).expect("a hex colour")
        };
        // Light mode: the button is primary, its label primary foreground.
        let light = config(ThemeMode::Light).colors;
        let ratio = contrast(parse(light.primary_foreground), parse(light.primary));
        assert!(ratio >= 4.5, "light: {ratio:.2}");
        // Dark mode: primary is too light for white text, so the button has
        // colours of its own, at rest, hovered and pressed.
        let dark = config(ThemeMode::Dark).colors;
        let label = parse(dark.button_primary_foreground);
        assert!(contrast(label, parse(dark.primary)) < 4.5);
        for (state, fill) in [
            ("rest", dark.button_primary),
            ("hover", dark.button_primary_hover),
            ("press", dark.button_primary_active),
        ] {
            let ratio = contrast(label, parse(fill));
            assert!(ratio >= 4.5, "dark {state}: {ratio:.2}");
        }
    }

    #[test]
    fn code_takes_quiet_colours_from_its_own_mode() {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let config = config(mode);
            let style = config
                .highlight
                .expect("each theme carries its code colours");
            let pick = |(light, dark): Pair| {
                let (h, s, l) = if mode.is_dark() { dark } else { light };
                hsl(h, s, l)
            };
            let color = |style: Option<ThemeStyle>| style.and_then(|style| style.color);
            assert_eq!(color(style.syntax.comment), Some(pick(COMMENT)), "{mode:?}");
            assert_eq!(color(style.syntax.comment_doc), Some(pick(COMMENT)));
            assert_eq!(color(style.syntax.keyword), Some(pick(KEYWORD)));
            assert_eq!(color(style.syntax.boolean), Some(pick(NUMBER)));
            // The editor and its gutter sit on the stage's background, and
            // no line is marked as being edited.
            assert_eq!(style.editor_background, Some(pick(BACKGROUND)));
            assert_eq!(style.editor_gutter_background, Some(pick(BACKGROUND)));
            assert_eq!(style.editor_active_line, None);
            assert_eq!(
                style.editor_active_line_number,
                Some(pick(MUTED_FOREGROUND))
            );
        }
    }
}
