//! The presentation: the same stage alone in the window, fullscreen,
//! entered from the stage with Space or Enter and left with Esc.

use std::path::Path;

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, ElementId, Entity, Pixels, Point, TestAppContext, px, size,
};
use viewer::{AppState, AppView, Launch};

use crate::stage::{Shown, numbered, open_checked, png, position, probe, show, shown};
use crate::{
    click, current, drag_divider, is_focused, node, open_app, press, read, row, state, write,
};

/// How the window is laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Layout {
    /// The explorer and the stage, with the explorer this wide.
    Split(Pixels),
    /// The stage alone, filling the window, with nothing else on screen.
    Presentation,
}

/// The window's layout after a fresh frame. A presentation must show the
/// stage and nothing else: no explorer, divider or position.
fn layout(cx: &mut TestAppContext, handle: AnyWindowHandle) -> Layout {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        if let Some(explorer) = window.try_find("explorer-pane") {
            assert!(window.try_find("stage-pane").is_some());
            return Layout::Split(explorer.bounds().size.width);
        }
        let stage = window.find("stage").bounds();
        assert_eq!(
            stage.origin,
            Point::default(),
            "the stage starts at the corner"
        );
        assert_eq!(
            stage.size,
            window.viewport_size(),
            "the stage fills the window"
        );
        assert!(window.try_find("stage-position").is_none());
        let stage_id = ElementId::from("stage");
        for element in window.elements() {
            assert!(
                element.path().contains(&stage_id),
                "only the stage is on screen, not {:?}",
                element.path()
            );
        }
        Layout::Presentation
    })
    .expect("read the layout")
}

fn presenting(cx: &mut TestAppContext, view: &Entity<AppView>) -> bool {
    read(cx, view, AppState::is_presenting)
}

fn fullscreen(cx: &mut TestAppContext, handle: AnyWindowHandle) -> bool {
    cx.update_window(handle, |_, window, _| window.is_fullscreen())
        .expect("read the window")
}

/// Takes the window into or out of fullscreen as the green button does.
fn toggle_fullscreen(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, _| window.toggle_fullscreen())
        .expect("toggle fullscreen");
    cx.run_until_parked();
}

/// Rescans the folder, as a refresh does, and lets it land.
fn refresh(cx: &mut TestAppContext, view: &Entity<AppView>) {
    let state = state(cx, view);
    cx.update(|cx| state.update(cx, AppState::refresh));
    cx.run_until_parked();
}

/// Three files in a gallery. Display order: b/inner.rs, a.md, c.png.
fn three(dir: &Path) {
    write(dir, "b/inner.rs", "inner");
    write(dir, "a.md", "# a");
    write(dir, "c.png", png(64, 48));
}

#[gpui_kit::test]
fn space_on_the_stage_presents_and_escape_restores_the_split(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    three(dir.path());
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    click(cx, handle, "select-all");
    drag_divider(cx, handle, 400.);
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));
    // Three checked files show the gallery: Enter opens the first.
    press(cx, handle, "enter");
    let before = current(cx, &view);
    assert_eq!(layout(cx, handle), Layout::Split(px(400.)));

    press(cx, handle, "space");
    assert!(presenting(cx, &view));
    assert_eq!(layout(cx, handle), Layout::Presentation);
    assert_eq!(shown(cx, handle), Some(Shown::Code("inner.rs".into())));
    assert_eq!(current(cx, &view), before);
    assert!(is_focused(cx, handle, "stage"));

    press(cx, handle, "escape");
    assert!(!presenting(cx, &view));
    assert_eq!(layout(cx, handle), Layout::Split(px(400.)));
    assert_eq!(current(cx, &view), before);
    assert!(is_focused(cx, handle, "stage"));
    assert_eq!(position(cx, handle).as_deref(), Some("1 / 3"));
}

#[gpui_kit::test]
fn enter_on_the_stage_presents_too(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    three(dir.path());
    let (handle, view) = open_checked(cx, dir.path());
    press(cx, handle, "enter");
    assert!(presenting(cx, &view));
    assert_eq!(layout(cx, handle), Layout::Presentation);
    press(cx, handle, "escape");
    assert_eq!(layout(cx, handle), Layout::Split(px(300.)));
}

#[gpui_kit::test]
fn a_blank_stage_does_not_present(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    three(dir.path());
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));
    for key in ["space", "enter"] {
        press(cx, handle, key);
    }
    assert!(!presenting(cx, &view));
    assert_eq!(layout(cx, handle), Layout::Split(px(300.)));
    assert!(!fullscreen(cx, handle));
}

#[gpui_kit::test]
fn right_space_and_left_walk_the_gallery_and_stop_at_the_ends(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    three(dir.path());
    let (handle, view) = open_checked(cx, dir.path());
    press(cx, handle, "space");
    assert!(presenting(cx, &view));

    let [inner, notes, picture] = [
        Shown::Code("inner.rs".into()),
        Shown::Markdown("a.md".into()),
        Shown::Image("c.png".into()),
    ];
    let walk = [
        ("right", &notes),
        ("space", &picture),
        // The last file: no wrap.
        ("space", &picture),
        ("right", &picture),
        ("left", &notes),
        ("left", &inner),
        // The first file: no wrap.
        ("left", &inner),
        ("right", &notes),
        ("right", &picture),
    ];
    for (key, expected) in walk {
        press(cx, handle, key);
        assert_eq!(shown(cx, handle).as_ref(), Some(expected), "after {key}");
        assert_eq!(layout(cx, handle), Layout::Presentation, "after {key}");
    }

    // Leaving keeps the file the presentation ended on.
    press(cx, handle, "escape");
    assert_eq!(current(cx, &view), Some(node(cx, &view, "c.png")));
    assert_eq!(shown(cx, handle), Some(picture));
    assert_eq!(position(cx, handle).as_deref(), Some("3 / 3"));
}

#[gpui_kit::test]
fn the_presentation_keeps_the_stage_its_content_and_its_scroll(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.rs", numbered(400, "let line_", " = 0;\n"));
    write(
        dir.path(),
        "b.md",
        numbered(200, "Paragraph ", ", long enough to read.\n\n"),
    );
    let (handle, view) = open_checked(cx, dir.path());

    for name in ["a.rs", "b.md"] {
        let scroll = |cx: &mut TestAppContext| probe(cx, &view).scroll();
        press(cx, handle, "pagedown");
        let scrolled = scroll(cx);
        assert!(scrolled > 0., "{name}");
        let loads = probe(cx, &view).loads();

        // The same view, scrolled as it was: nothing is read again.
        press(cx, handle, "space");
        assert_eq!(layout(cx, handle), Layout::Presentation);
        assert_eq!(scroll(cx), scrolled, "{name} keeps its place");

        // The vertical keys scroll as on the stage.
        press(cx, handle, "down");
        assert_eq!(scroll(cx), scrolled + 40., "{name}: one step");
        press(cx, handle, "up");
        assert_eq!(scroll(cx), scrolled, "{name}: back one step");
        press(cx, handle, "pagedown");
        assert!(scroll(cx) > scrolled, "{name}: a page");
        press(cx, handle, "end");
        let end = scroll(cx);
        press(cx, handle, "pageup");
        assert!(scroll(cx) < end, "{name}: a page back");
        press(cx, handle, "home");
        assert_eq!(scroll(cx), 0., "{name}: home");
        press(cx, handle, "down");

        press(cx, handle, "escape");
        assert_eq!(scroll(cx), 40., "{name} keeps its place on the way back");
        assert_eq!(probe(cx, &view).loads(), loads, "{name} was not read again");
        press(cx, handle, "right");
    }
}

#[gpui_kit::test]
fn presenting_makes_a_window_fullscreen_and_leaving_makes_it_windowed_again(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    three(dir.path());
    let (handle, _) = open_checked(cx, dir.path());
    assert!(!fullscreen(cx, handle));
    press(cx, handle, "space");
    assert!(fullscreen(cx, handle));
    press(cx, handle, "escape");
    assert!(!fullscreen(cx, handle));
}

#[gpui_kit::test]
fn a_window_already_fullscreen_stays_fullscreen_after_presenting(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    three(dir.path());
    let (handle, view) = open_checked(cx, dir.path());
    toggle_fullscreen(cx, handle);
    assert!(fullscreen(cx, handle));
    press(cx, handle, "space");
    assert!(presenting(cx, &view));
    assert!(fullscreen(cx, handle));
    press(cx, handle, "escape");
    assert!(!presenting(cx, &view));
    assert!(
        fullscreen(cx, handle),
        "presenting did not make it fullscreen"
    );
}

#[gpui_kit::test]
fn leaving_fullscreen_by_the_window_keeps_presenting_windowed(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    three(dir.path());
    let (handle, view) = open_checked(cx, dir.path());
    press(cx, handle, "space");
    assert!(fullscreen(cx, handle));

    // The green button or the system gesture.
    toggle_fullscreen(cx, handle);
    assert!(!fullscreen(cx, handle));
    assert!(presenting(cx, &view));
    assert_eq!(layout(cx, handle), Layout::Presentation);

    // Esc leaves, and the window stays as the person left it.
    press(cx, handle, "escape");
    assert_eq!(layout(cx, handle), Layout::Split(px(300.)));
    assert!(!fullscreen(cx, handle));
}

#[gpui_kit::test]
fn a_fullscreen_the_person_makes_during_the_presentation_stays_after_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    three(dir.path());
    let (handle, view) = open_checked(cx, dir.path());
    // Each change of fullscreen resizes the window, as on the platform.
    let green_button = |cx: &mut TestAppContext, width: f32, height: f32| {
        toggle_fullscreen(cx, handle);
        cx.simulate_window_resize(handle, size(px(width), px(height)));
        cx.run_until_parked();
    };
    press(cx, handle, "space");
    assert!(fullscreen(cx, handle));
    cx.simulate_window_resize(handle, size(px(1400.), px(900.)));
    cx.run_until_parked();

    // Out by the window, then in again: that fullscreen is the person's.
    green_button(cx, 1000., 700.);
    assert!(!fullscreen(cx, handle));
    green_button(cx, 1400., 900.);
    assert!(fullscreen(cx, handle));
    assert!(presenting(cx, &view));

    press(cx, handle, "escape");
    assert!(matches!(layout(cx, handle), Layout::Split(_)));
    assert!(
        fullscreen(cx, handle),
        "Esc took away the person's fullscreen"
    );
}

#[gpui_kit::test]
fn the_split_comes_back_at_its_width_after_the_window_grows_and_shrinks(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    three(dir.path());
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    click(cx, handle, "select-all");
    drag_divider(cx, handle, 400.);
    press(cx, handle, "tab");
    press(cx, handle, "space");

    // Fullscreen grows the window; the stage follows it.
    cx.simulate_window_resize(handle, size(px(1400.), px(900.)));
    cx.run_until_parked();
    assert_eq!(layout(cx, handle), Layout::Presentation);

    // Leaving: the split comes back first, then the window shrinks.
    press(cx, handle, "escape");
    assert!(matches!(layout(cx, handle), Layout::Split(_)));
    cx.simulate_window_resize(handle, size(px(1000.), px(700.)));
    cx.run_until_parked();
    let Layout::Split(width) = layout(cx, handle) else {
        panic!("the split is back");
    };
    assert!(
        (width - px(400.)).abs() < px(0.5),
        "the explorer is {width:?}"
    );
    assert!(!presenting(cx, &view));
}

#[gpui_kit::test]
fn keys_the_presentation_leaves_out_do_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.rs", "b.rs", "c.rs"] {
        write(dir.path(), name, name);
    }
    let (handle, view) = open_checked(cx, dir.path());
    press(cx, handle, "space");
    let on_stage = current(cx, &view);

    for key in ["enter", "secondary-f", "tab", "shift-tab", "secondary-o"] {
        press(cx, handle, key);
        assert!(presenting(cx, &view), "{key} keeps the presentation");
        assert_eq!(current(cx, &view), on_stage, "{key} keeps the file");
        assert!(is_focused(cx, handle, "stage"), "{key} keeps focus");
    }
    assert!(!cx.did_prompt_for_paths(), "⌘O waits for the presentation");

    // Space moves on and checks nothing.
    press(cx, handle, "space");
    press(cx, handle, "space");
    assert_eq!(read(cx, &view, AppState::selected_count), 3);
    press(cx, handle, "escape");
    for name in ["a.rs", "b.rs", "c.rs"] {
        let id = node(cx, &view, name);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let check = window.within(row(id)).find("check");
            assert_eq!(check.checked(), Some(true), "{name}");
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn a_file_outside_an_empty_gallery_is_presented_alone(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.rs", "b.rs", "c.rs"] {
        write(dir.path(), name, name);
    }
    let (handle, view) = open_app(cx, Launch::Folders(vec![dir.path().to_owned()]));
    let b = node(cx, &view, "b.rs");
    show(cx, handle, b);
    press(cx, handle, "tab");
    assert!(is_focused(cx, handle, "stage"));

    press(cx, handle, "space");
    assert!(presenting(cx, &view));
    for key in ["right", "space", "left"] {
        press(cx, handle, key);
        assert_eq!(current(cx, &view), Some(b), "{key}");
    }
    assert_eq!(shown(cx, handle), Some(Shown::Code("b.rs".into())));
    assert_eq!(layout(cx, handle), Layout::Presentation);
    press(cx, handle, "escape");
    assert_eq!(current(cx, &view), Some(b));
    assert_eq!(position(cx, handle), None);
}

#[gpui_kit::test]
fn a_refresh_that_deletes_the_presented_file_presents_its_neighbour(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.rs", "b.rs", "c.rs"] {
        write(dir.path(), name, name);
    }
    let (handle, view) = open_checked(cx, dir.path());
    press(cx, handle, "right");
    press(cx, handle, "space");
    assert_eq!(shown(cx, handle), Some(Shown::Code("b.rs".into())));

    std::fs::remove_file(dir.path().join("b.rs")).unwrap();
    refresh(cx, &view);
    assert!(presenting(cx, &view));
    assert_eq!(layout(cx, handle), Layout::Presentation);
    assert_eq!(shown(cx, handle), Some(Shown::Code("c.rs".into())));
    press(cx, handle, "left");
    assert_eq!(shown(cx, handle), Some(Shown::Code("a.rs".into())));
}

#[gpui_kit::test]
fn a_refresh_that_leaves_nothing_to_present_ends_the_presentation(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.rs", "b.rs"] {
        write(dir.path(), name, name);
    }
    let (handle, view) = open_checked(cx, dir.path());
    press(cx, handle, "space");
    assert!(fullscreen(cx, handle));

    for name in ["a.rs", "b.rs"] {
        std::fs::remove_file(dir.path().join(name)).unwrap();
    }
    refresh(cx, &view);
    assert!(!presenting(cx, &view));
    assert_eq!(current(cx, &view), None);
    assert_eq!(layout(cx, handle), Layout::Split(px(300.)));
    assert!(!fullscreen(cx, handle));
    assert_eq!(shown(cx, handle), None);
}

#[gpui_kit::test]
fn the_presentation_keys_work_while_the_code_view_holds_focus(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.rs", numbered(200, "line ", "\n"));
    write(dir.path(), "b.rs", "b");
    let (handle, view) = open_checked(cx, dir.path());
    press(cx, handle, "space");

    // A click in the code view gives it focus, for selecting and copying.
    click(cx, handle, "stage-code");
    press(cx, handle, "down");
    assert_eq!(probe(cx, &view).scroll(), 40.);
    click(cx, handle, "stage-code");
    press(cx, handle, "space");
    assert_eq!(shown(cx, handle), Some(Shown::Code("b.rs".into())));
    click(cx, handle, "stage-code");
    press(cx, handle, "tab");
    assert!(presenting(cx, &view));
    press(cx, handle, "escape");
    assert!(!presenting(cx, &view));
    assert!(is_focused(cx, handle, "stage"));
}
