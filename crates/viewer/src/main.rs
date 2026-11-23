use std::path::PathBuf;

use futures::StreamExt as _;
use futures::channel::mpsc;
use viewer::{AppAssets, StateStore, open_main_window, path_from_url};

fn main() {
    // `strikebrain-viewer <folder or file>...` opens those folders, and a
    // file's folder with the file on the stage; with none the folders open
    // last time reopen, as they were seen.
    let asked: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let store = StateStore::in_config_dir();

    // Folders and files the system hands over, at launch or while running:
    // "Open With", or a drop on the Dock icon. They arrive as `file://`
    // URLs, off any GPUI context, so they wait in a channel for the window,
    // each batch whole: files handed over together show as the gallery.
    let (handed_over, mut batches) = mpsc::unbounded::<Vec<PathBuf>>();
    let app = gpui_kit::application().with_assets(AppAssets);
    app.on_open_urls(move |urls| {
        let paths = urls.iter().filter_map(|url| path_from_url(url)).collect();
        // The app is quitting when nothing listens.
        handed_over.unbounded_send(paths).unwrap_or_default();
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        viewer::init(cx);
        // Closing the window quits, as ⌘Q does: `open_main_window` makes
        // its close button quit, keeping the window for the quit's save.
        let (_, view) = open_main_window(asked, store, true, cx).expect("open the main window");
        let view = view.downgrade();
        cx.spawn(async move |cx| {
            while let Some(paths) = batches.next().await {
                // The window is gone only as the app quits.
                let opened = view.update(cx, |view, cx| view.open_paths(paths, cx));
                opened.unwrap_or_default();
            }
        })
        .detach();
        // Come to the front when launched from a terminal too. An
        // occluded window does not draw.
        cx.activate(true);
    });
}
