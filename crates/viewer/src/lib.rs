//! The Strikebrain Viewer application: GPUI views over the explorer engine.

mod app;
mod assets;
mod columns;
mod explorer_panel;
mod keys;
mod launch;
mod menus;
mod persist;
mod row;
mod stage;
mod state;
mod theme;

pub use app::{AppView, EXPLORER_MIN_WIDTH, EXPLORER_WIDTH, SAVE_DELAY, Session};
pub use assets::AppAssets;
pub use launch::{folder_from_url, open_main_window};
pub use persist::{
    APP_ID, FolderView, Plan, STATE_FILE_VAR, Saved, SavedWindow, StateStore, fit, plan_launch,
};
pub use stage::StageProbe;
pub use state::{AppState, Launch, ScanStatus, StateOptions};

use gpui_kit::App;

/// Sets the app up: key bindings, the Quit and About actions, the menu bar
/// and brainstrike's theme. Call once, after `gpui_kit::init` and before
/// opening a window.
pub fn init(cx: &mut App) {
    cx.bind_keys(keys::bindings());
    cx.on_action(|_: &keys::Quit, cx| cx.quit());
    cx.on_action(menus::about);
    menus::install(false, cx);
    theme::install(cx);
}
