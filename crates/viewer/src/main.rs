use std::path::PathBuf;

use viewer::{AppAssets, StateStore, open_main_window};

fn main() {
    // `strikebrain-viewer <folder>` opens the folder; with no argument the
    // folder open last time reopens, as it was seen.
    let folder = std::env::args_os().nth(1).map(PathBuf::from);
    let store = StateStore::in_config_dir();

    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            viewer::init(cx);
            // Closing the last window quits, as ⌘Q does.
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            open_main_window(folder, store, true, cx).expect("open the main window");
            // Come to the front when launched from a terminal too. An
            // occluded window does not draw.
            cx.activate(true);
        });
}
