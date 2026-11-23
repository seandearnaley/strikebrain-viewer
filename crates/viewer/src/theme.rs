//! Brainstrike's colours, installed as the Kit's light and dark themes.
//!
//! The Kit derives every other colour (hover and active shades, button,
//! list and scrollbar colours) from the ones set here. The app follows the
//! system appearance: [`install`] picks the theme for it at launch and
//! [`follow_system`] switches when it changes.

use std::rc::Rc;

use gpui_kit::component::{Colorize as _, Theme, ThemeConfig, ThemeConfigColors, ThemeMode, hsl};
use gpui_kit::{App, SharedString, Window};

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

    let mut config = ThemeConfig::default();
    config.name = if mode.is_dark() {
        "Brainstrike Dark".into()
    } else {
        "Brainstrike Light".into()
    };
    config.mode = mode;
    config.colors = colors;
    config
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
                gpui_kit::Hsla::parse_hex(&hex.expect("a colour")).expect("a hex colour")
            };
            let (ring, foreground) = (parse(colors.ring), parse(colors.foreground));
            assert!((ring.a - RING_ALPHA).abs() < 0.01, "{mode:?}: {ring:?}");
            assert_eq!(ring.alpha(1.).to_hex(), foreground.to_hex(), "{mode:?}");
        }
    }
}
