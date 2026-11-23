use std::path::PathBuf;

use futures::StreamExt as _;
use futures::channel::mpsc;
use viewer::{AppAssets, StateStore, folder_from_url, open_main_window};

fn main() {
    // `strikebrain-viewer <folder>` opens the folder; with no argument the
    // folder open last time reopens, as it was seen.
    let folder = std::env::args_os().nth(1).map(PathBuf::from);
    let store = StateStore::in_config_dir();

    // Folders the system hands over while running: "Open With", or a
    // folder dropped on the Dock icon. They arrive as `file://` URLs, off
    // any GPUI context, so they wait in a channel for the window.
    let (handed_over, mut folders) = mpsc::unbounded::<PathBuf>();
    let app = gpui_kit::application().with_assets(AppAssets);
    app.on_open_urls(move |urls| {
        for folder in urls.iter().filter_map(|url| folder_from_url(url)) {
            // The app is quitting when nothing listens.
            handed_over.unbounded_send(folder).unwrap_or_default();
        }
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        viewer::init(cx);
        // Closing the window quits, as ⌘Q does: `open_main_window` makes
        // its close button quit, keeping the window for the quit's save.
        let (_, view) = open_main_window(folder, store, true, cx).expect("open the main window");
        let view = view.downgrade();
        cx.spawn(async move |cx| {
            while let Some(folder) = folders.next().await {
                // The window is gone only as the app quits.
                let opened = view.update(cx, |view, cx| view.open_folder(folder, cx));
                opened.unwrap_or_default();
            }
        })
        .detach();
        // Come to the front when launched from a terminal too. An
        // occluded window does not draw.
        cx.activate(true);
    });
}
