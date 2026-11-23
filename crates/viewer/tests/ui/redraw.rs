//! What a frame redraws: work in the explorer leaves the stage alone.

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, InputEvent as _, KeyDownEvent, KeyUpEvent, Keystroke,
    TestAppContext,
};
use viewer::AppView;

use crate::{PROJECT, click, focus, node, open_app, press, tree};

fn stage_renders(cx: &mut TestAppContext, view: &Entity<AppView>) -> usize {
    cx.update(|cx| view.read(cx).stage_probe(cx).renders())
}

/// Presses `key` as the platform delivers it, with no forced redraw: the
/// harness then draws only what the key made dirty, as a real frame does.
fn key(cx: &mut TestAppContext, handle: AnyWindowHandle, key: &'static str) {
    let keystroke = Keystroke::parse(key).expect("a valid keystroke");
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_event(
            KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    })
    .expect("press a key");
    cx.run_until_parked();
}

#[gpui_kit::test]
fn moving_focus_in_the_tree_leaves_the_stage_alone(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    // The first key after the pointer redraws every view once: GPUI
    // refreshes the window when the input changes from pointer to keys.
    press(cx, handle, "down");
    let before = stage_renders(cx, &view);

    for _ in 0..3 {
        key(cx, handle, "down");
    }
    assert_eq!(focus(cx, &view), Some(node(cx, &view, "README.md")));
    assert_eq!(stage_renders(cx, &view), before);

    // Enter shows the file: now the stage draws.
    key(cx, handle, "enter");
    assert!(stage_renders(cx, &view) > before);
}

#[gpui_kit::test]
fn moving_focus_in_the_tree_leaves_the_gallery_alone(cx: &mut TestAppContext) {
    let (handle, view) = open_app(cx, tree(PROJECT));
    click(cx, handle, "select-all");
    press(cx, handle, "down");
    let gallery_renders =
        |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).gallery_probe(cx).renders());
    let before = (stage_renders(cx, &view), gallery_renders(cx));
    assert!(before.1 > 0, "the gallery shows");

    for _ in 0..3 {
        key(cx, handle, "down");
    }
    assert_eq!((stage_renders(cx, &view), gallery_renders(cx)), before);

    // Space checks the focus row's file: the gallery changes and draws.
    key(cx, handle, "space");
    assert!(gallery_renders(cx) > before.1);
}
