use std::path::PathBuf;

use gpui_kit::{AppContext, TitlebarOptions, WindowOptions};
use viewer::{AppAssets, AppView, Launch};

fn main() {
    // `strikebrain-viewer <folder>` opens the folder; with no argument the
    // explorer offers "Open Folder".
    let launch = std::env::args_os().nth(1).map_or(Launch::Empty, |folder| {
        Launch::Folder(PathBuf::from(folder))
    });

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

            gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Strikebrain Viewer".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| AppView::new(launch, window, cx)),
            )
            .expect("open main window");
            // Come to the front when launched from a terminal too. An
            // occluded window does not draw.
            cx.activate(true);
        });
}
