//! The two panes and the divider between them.

use gpui_kit::AppContext as _;
use gpui_kit::TestAppContext;
use gpui_kit::px;
use gpui_kit::test::TestWindowExt as _;
use viewer::{EXPLORER_MIN_WIDTH, EXPLORER_WIDTH, Launch};

use crate::{drag_divider, open_app};

#[gpui_kit::test]
fn window_shows_explorer_left_of_stage(cx: &mut TestAppContext) {
    let (handle, _) = open_app(cx, Launch::Empty);

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
    let (handle, _) = open_app(cx, Launch::Empty);

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
    let (handle, _) = open_app(cx, Launch::Empty);

    drag_divider(cx, handle, 50.);

    cx.update_window(handle, |_, window, _| {
        let explorer = window.find("explorer-pane").bounds();

        assert_eq!(explorer.size.width, px(EXPLORER_MIN_WIDTH));
    })
    .unwrap();
}
