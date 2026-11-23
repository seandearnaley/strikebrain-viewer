//! Opening the main window as the app launches: the folder from the
//! command line or the saved one, and the saved view and window.

use std::path::PathBuf;

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Result, TitlebarOptions, WindowBounds,
    WindowOptions,
};

use crate::app::{AppView, Session};
use crate::persist::{SavedWindow, StateStore, fit, plan_launch};

/// Opens the main window. `folder` is the command line's; the saved state
/// comes from `store`, which then also receives the window's state as it
/// changes. With `watch` the folder is followed on disk, as the app does;
/// tests that need determinism leave it off. Closing the window quits, so
/// the state is saved by the quit with the window still there.
///
/// # Errors
///
/// If the platform cannot open a window.
pub fn open_main_window(
    folder: Option<PathBuf>,
    store: Option<StateStore>,
    watch: bool,
    cx: &mut App,
) -> Result<(AnyWindowHandle, Entity<AppView>)> {
    let plan = plan_launch(folder, store.as_ref().and_then(StateStore::load));
    let (window_bounds, display_id) = match plan.window.as_ref() {
        Some(saved) => {
            let (bounds, display) = placement(saved, cx);
            (Some(bounds), display)
        }
        None => (None, None),
    };
    let session = Session {
        view: plan.view,
        explorer_width: plan.explorer_width,
        store,
        watch,
    };
    let launch = plan.launch;
    let opened = gpui_kit::open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Strikebrain Viewer".into()),
                ..Default::default()
            }),
            window_bounds,
            display_id,
            ..Default::default()
        },
        cx,
        |window, cx| {
            window.on_window_should_close(cx, |_, cx| {
                cx.quit();
                false
            });
            cx.new(|cx| AppView::with_session(launch, session, window, cx))
        },
    )?;
    Ok(opened)
}

/// Where to open a window saved as `saved`: its display if still there,
/// else the primary one, with the bounds moved and shrunk into what that
/// display shows.
fn placement(saved: &SavedWindow, cx: &App) -> (WindowBounds, Option<gpui_kit::DisplayId>) {
    let display = cx
        .displays()
        .into_iter()
        .find(|display| {
            saved.display.is_some()
                && display.uuid().ok().map(|uuid| uuid.to_string()) == saved.display
        })
        .or_else(|| cx.primary_display());
    let Some(display) = display else {
        return (saved.bounds, None);
    };
    let area = display.visible_bounds();
    let fitted = |bounds: Bounds<_>| fit(bounds, area);
    let bounds = match saved.bounds {
        WindowBounds::Windowed(bounds) => WindowBounds::Windowed(fitted(bounds)),
        WindowBounds::Maximized(bounds) => WindowBounds::Maximized(fitted(bounds)),
        WindowBounds::Fullscreen(bounds) => WindowBounds::Fullscreen(fitted(bounds)),
    };
    (bounds, Some(display.id()))
}
