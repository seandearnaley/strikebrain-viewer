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
const RING: Pair = ((25., 12., 38.), (30., 6., 52.));
const WARNING: Pair = ((30., 90., 29.), (38., 88., 60.));
const DANGER: Pair = ((8., 55., 39.), (8., 50., 75.));
const POPOVER: Pair = ((0., 0., 100.), (30., 6., 14.));

/// Installs brainstrike's light and dark themes and applies the one that
/// matches the system appearance. Call once, after `gpui_kit::init`.
pub fn install(cx: &mut App) {
    Theme::update(cx, |theme| {
        theme.light_theme = Rc::new(config(ThemeMode::Light));
        theme.dark_theme = Rc::new(config(ThemeMode::Dark));
    });
    Theme::change(cx.window_appearance(), None, cx);
}

/// Switches to the theme for the window's current appearance.
pub fn follow_system(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
}

fn config(mode: ThemeMode) -> ThemeConfig {
    let pick = |(light, dark): Pair| -> Option<SharedString> {
        let (h, s, l) = if mode.is_dark() { dark } else { light };
        Some(hsl(h, s, l).to_hex().into())
    };

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
    colors.ring = pick(RING);
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
}
