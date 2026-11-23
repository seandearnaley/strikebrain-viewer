use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, point,
    px, size,
};
use viewer::{AppView, EXPLORER_MIN_WIDTH, EXPLORER_WIDTH};

/// Opens the app in a 1000 x 700 window and returns the window.
fn open_app(cx: &mut TestAppContext) -> AnyWindowHandle {
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
    handle
}

/// Drags the divider from its starting position to `to_x`, at the explorer's mid-height.
fn drag_divider(cx: &mut TestAppContext, handle: AnyWindowHandle, to_x: f32) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let explorer = window.find("explorer-pane").bounds();
        assert_eq!(explorer.right(), px(EXPLORER_WIDTH));

        let mid_height = explorer.center().y;
        window.drag(
            point(px(EXPLORER_WIDTH), mid_height),
            point(px(to_x), mid_height),
            cx,
        );
        window.render_frame(cx);
    })
    .expect("drag the divider in the test window");
}

#[gpui_kit::test]
fn window_shows_explorer_left_of_stage(cx: &mut TestAppContext) {
    let handle = open_app(cx);

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);

        let explorer = window.find("explorer-pane");
        let stage = window.find("stage-pane");

        assert!(explorer.visible());
        assert!(stage.visible());
        // The divider is positioned absolutely over the panel boundary, so it
        // takes no width from either pane.
        assert_eq!(explorer.bounds().size.width, px(EXPLORER_WIDTH));
        assert!(stage.bounds().left() >= explorer.bounds().right());
        assert!(stage.bounds().size.width > px(600.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn dragging_the_divider_resizes_the_explorer(cx: &mut TestAppContext) {
    let handle = open_app(cx);

    drag_divider(cx, handle, 400.);

    cx.update_window(handle, |_, window, _| {
        let explorer = window.find("explorer-pane").bounds();
        let stage = window.find("stage-pane").bounds();

        assert_eq!(explorer.size.width, px(400.));
        assert_eq!(stage.left(), explorer.right());
    })
    .unwrap();
}

#[gpui_kit::test]
fn explorer_cannot_shrink_below_its_minimum(cx: &mut TestAppContext) {
    let handle = open_app(cx);

    drag_divider(cx, handle, 50.);

    cx.update_window(handle, |_, window, _| {
        let explorer = window.find("explorer-pane").bounds();

        assert_eq!(explorer.size.width, px(EXPLORER_MIN_WIDTH));
    })
    .unwrap();
}
