use gpui_kit::{AppContext, TitlebarOptions, WindowOptions};
use viewer::{AppAssets, AppView};

fn main() {
    gpui_kit::application().with_assets(AppAssets).run(|cx| {
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
            |window, cx| cx.new(|cx| AppView::new(window, cx)),
        )
        .expect("open main window");
        // Come to the front when launched from a terminal too. An
        // occluded window does not draw.
        cx.activate(true);
    });
}
