use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size};
use viewer::{AppView, EXPLORER_WIDTH};

#[gpui_kit::test]
fn window_shows_explorer_left_of_stage(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (handle, _view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Point::default(),
                    size(px(1000.), px(700.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| AppView::new(window, cx)),
        )
        .expect("open test window")
    });

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);

        let explorer = window.find("explorer-pane");
        let stage = window.find("stage-pane");

        assert!(explorer.visible());
        assert!(stage.visible());
        // The divider takes a few pixels, so allow a small tolerance.
        let width = explorer.bounds().size.width;
        assert!(width > px(EXPLORER_WIDTH - 12.) && width < px(EXPLORER_WIDTH + 12.));
        assert!(stage.bounds().left() >= explorer.bounds().right());
        assert!(stage.bounds().size.width > px(600.));
    })
    .unwrap();
}
