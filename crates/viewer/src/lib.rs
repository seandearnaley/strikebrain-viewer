//! The Strikebrain Viewer application: GPUI views over the explorer engine.

mod app;
mod assets;
mod columns;
mod explorer_panel;
pub mod keys;
mod launch;
mod menus;
mod persist;
mod row;
mod search;
mod stage;
mod state;
mod theme;
mod title_bar;
mod update;

pub use app::{AppView, EXPLORER_MIN_WIDTH, EXPLORER_WIDTH, SAVE_DELAY, STAGE_MIN_WIDTH, Session};
pub use assets::AppAssets;
pub use launch::{open_main_window, path_from_url};
pub use persist::{
    APP_ID, Plan, STATE_FILE_VAR, Saved, SavedView, SavedWindow, StateStore, fit, plan_launch,
};
pub use stage::{GalleryProbe, StageProbe};
pub use state::{AppState, FolderLayout, Launch, ScanStatus, StageView, StateOptions};
pub use update::{Owner, UpdateOptions, UpdateSettings};

use gpui_kit::App;

/// Sets the app up: key bindings, the Quit and About actions, the menu bar
/// and brainstrike's theme. Call once, after `gpui_kit::init` and before
/// opening a window.
pub fn init(cx: &mut App) {
    cx.bind_keys(keys::bindings());
    cx.on_action(|_: &keys::Quit, cx| cx.quit());
    cx.on_action(menus::about);
    cx.on_action(|_: &keys::Hide, cx| cx.hide());
    cx.on_action(|_: &keys::HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &keys::ShowAll, cx| cx.unhide_other_apps());
    menus::install(menus::MenuBar::default(), cx);
    theme::install(cx);
}
