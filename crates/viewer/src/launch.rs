//! Opening the main window as the app launches: the folders from the
//! command line or the saved ones, the saved view and window, and folders
//! the system hands over later ("Open With", a folder dropped on the Dock
//! icon) as `file://` URLs.

use std::path::PathBuf;

use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Result, TitlebarOptions, WindowBounds,
    WindowOptions,
};

use crate::app::{AppView, Session};
use crate::menus::APP_NAME;
use crate::persist::{SavedWindow, StateStore, fit, plan_launch};

/// Opens the main window. `folders` are the command line's; the saved state
/// comes from `store`, which then also receives the window's state as it
/// changes. With `watch` the folders are followed on disk, as the app does;
/// tests that need determinism leave it off. Closing the window quits, so
/// the state is saved by the quit with the window still there.
///
/// # Errors
///
/// If the platform cannot open a window.
pub fn open_main_window(
    folders: Vec<PathBuf>,
    store: Option<StateStore>,
    watch: bool,
    cx: &mut App,
) -> Result<(AnyWindowHandle, Entity<AppView>)> {
    let plan = plan_launch(folders, store.as_ref().and_then(StateStore::load));
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
        opened_at: window_bounds.map(|bounds| bounds.get_bounds()),
    };
    let launch = plan.launch;
    let opened = gpui_kit::open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(APP_NAME.into()),
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

/// The folder a `file://` URL names, percent-decoded; `None` for any other
/// URL or a malformed one.
#[must_use]
pub fn folder_from_url(url: &str) -> Option<PathBuf> {
    let path = url.strip_prefix("file://")?;
    // A host part, if any, ends at the first slash; only the local host is
    // ours.
    let path = path.strip_prefix("localhost").unwrap_or(path);
    if !path.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(path.len());
    let mut rest = path.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let hex = tail.get(..2)?;
            let text = std::str::from_utf8(hex).ok()?;
            bytes.push(u8::from_str_radix(text, 16).ok()?);
            rest = &tail[2..];
        } else {
            bytes.push(byte);
            rest = tail;
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    Some(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::folder_from_url;

    #[test]
    fn a_file_url_names_its_folder() {
        assert_eq!(
            folder_from_url("file:///Users/sean/My%20Folder/"),
            Some(PathBuf::from("/Users/sean/My Folder/"))
        );
        assert_eq!(
            folder_from_url("file://localhost/tmp/caf%C3%A9"),
            Some(PathBuf::from("/tmp/café"))
        );
        assert_eq!(
            folder_from_url("file:///plain"),
            Some(PathBuf::from("/plain"))
        );
        for bad in [
            "https://example.com/x",
            "file://server/share",
            "file:///bad%2",
            "file:///bad%zz",
            "file:///bad%FF",
        ] {
            assert_eq!(folder_from_url(bad), None, "{bad}");
        }
    }
}
