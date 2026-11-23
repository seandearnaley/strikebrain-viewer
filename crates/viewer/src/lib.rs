//! The Strikebrain Viewer application: GPUI views over the explorer engine.

mod app;
mod assets;
mod columns;
mod explorer_panel;
mod keys;
mod row;
mod stage;
mod state;
mod theme;

pub use app::{AppView, EXPLORER_MIN_WIDTH, EXPLORER_WIDTH};
pub use assets::AppAssets;
pub use state::{AppState, Launch, ScanStatus};

use gpui_kit::App;

/// Sets the app up: key bindings, the Quit action and brainstrike's theme.
/// Call once, after `gpui_kit::init` and before opening a window.
pub fn init(cx: &mut App) {
    cx.bind_keys(keys::bindings());
    cx.on_action(|_: &keys::Quit, cx| cx.quit());
    theme::install(cx);
}
