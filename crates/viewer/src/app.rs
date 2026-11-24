//! The root view: the explorer and the stage side by side, and keyboard
//! focus moving between the panes: filter, tree, stage. While presenting,
//! the same stage alone, fullscreen.
//!
//! Both panes are embedded as cached views: a frame reuses a pane's last
//! drawing unless that pane notified, so a key in the tree never rebuilds
//! the stage.
//!
//! The split's sizes live in an entity this view owns rather than in the
//! split element's own state, which lasts only while the split is drawn:
//! so the explorer keeps its width while the presentation hides it.
//!
//! With a [`StateStore`] the view saves itself: half a second after a
//! change (at most one save per half second), and when the app quits. A
//! save copies the state out on the UI thread, cheaply, and names, encodes
//! and writes it on the background executor.

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::component::{ResizablePanelEvent, ResizableState, h_resizable, resizable_panel};
use gpui_kit::{
    App, AppContext as _, Bounds, ClipboardItem, Context, Entity, ExternalPaths, Focusable as _,
    IntoElement, ParentElement as _, Pixels, Render, SharedString, StyleRefinement, Styled as _,
    Subscription, Task, TestSupportExt as _, Window, WindowBounds, div, prelude::*, px,
};

use crate::explorer_panel::{ExplorerFocus, ExplorerPanel};
use crate::keys::{
    AddFolder, CloseWindow, CopyPath, FocusExplorer, FocusFilter, Minimize, NextPane, PreviousPane,
    Refresh, Reveal, StartPresenting, ToggleFullScreen, ToggleShowIgnored, Zoom,
};
use crate::menus;
use crate::persist::{SavedView, SavedWindow, Snapshot, StateStore};
use crate::stage::{GalleryProbe, Stage, StageProbe};
use crate::state::{AppState, Launch, StateOptions};
use crate::theme;

/// Starting width of the explorer pane, in pixels.
pub const EXPLORER_WIDTH: f32 = 300.0;

/// Narrowest width the explorer pane can be dragged to, in pixels.
pub const EXPLORER_MIN_WIDTH: f32 = 220.0;

/// Narrowest width the stage keeps beside the explorer, in pixels.
pub const STAGE_MIN_WIDTH: f32 = 220.0;

/// How long a change waits before it is saved; changes meanwhile ride
/// along with it.
pub const SAVE_DELAY: Duration = Duration::from_millis(500);

/// What a window brings back from a saved state, where it saves, and
/// whether it follows its folders on disk. The default does none of these.
#[derive(Debug, Default)]
pub struct Session {
    /// The saved view of the folders being opened.
    pub view: Option<SavedView>,
    /// The explorer's saved width, in pixels.
    pub explorer_width: Option<f32>,
    /// Where to save. `None` saves nothing.
    pub store: Option<StateStore>,
    /// Rescan a folder when it changes on disk.
    pub watch: bool,
    /// The bounds the window opened with, as [`SavedWindow`] holds them:
    /// what a fullscreen window is saved with until it has been windowed.
    pub opened_at: Option<Bounds<Pixels>>,
}

/// What the root view knows of the window's fullscreen.
#[derive(Clone, Copy, Debug, Default)]
struct Fullscreen {
    /// Whether the presentation made the window fullscreen, so leaving it
    /// should make the window windowed again. Cleared when the person
    /// takes the window out of that fullscreen themselves.
    entered: bool,
    /// Whether the window was fullscreen when its bounds last changed.
    was: bool,
}

/// A pane that takes keyboard focus, in Tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pane {
    Filter,
    Tree,
    Stage,
}

/// The root view: explorer on the left, stage on the right. Owns the
/// [`AppState`] both observe, keeps the window title on the folder's name
/// or the app's, handles ⌘O, ⌘F, moving between panes and the menu bar's
/// commands, follows the presentation, and saves the state.
pub struct AppView {
    state: Entity<AppState>,
    explorer: Entity<ExplorerPanel>,
    stage: Entity<Stage>,
    /// The sizes of the explorer and the stage beside it.
    split: Entity<ResizableState>,
    /// The explorer's width when the split is first laid out.
    explorer_width: Pixels,
    /// The title last given to the window.
    title: Option<SharedString>,
    /// Whether the window is laid out for the presentation: follows
    /// [`AppState::is_presenting`].
    presenting: bool,
    /// What the view knows of the window's fullscreen.
    fullscreen: Fullscreen,
    /// Where the window is, as last reported.
    window: Option<SavedWindow>,
    /// The window's place while windowed, as a window opens with it: the
    /// top-left corner of its frame and the size of its content. Kept
    /// while fullscreen, which is saved as the window to return to.
    windowed: Option<Bounds<Pixels>>,
    /// Where the state is saved, if anywhere.
    store: Option<StateStore>,
    /// The save waiting for its delay. While it waits, changes add nothing.
    save_task: Option<Task<()>>,
    /// Whether the menu bar's Show Ignored is checked.
    menu_shows_ignored: bool,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for AppView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppView")
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

impl AppView {
    /// Builds the window's content and starts what `launch` asks for. The
    /// tree takes keyboard focus. Nothing is restored or saved, and no
    /// folder is followed on disk.
    pub fn new(launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_session(launch, Session::default(), window, cx)
    }

    /// Builds the window's content as [`AppView::new`] does, restoring,
    /// saving and following the folders as `session` says.
    pub fn with_session(
        launch: Launch,
        session: Session,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = StateOptions {
            view: session.view,
            watch: session.watch,
            quiet: session.store.as_ref().map_or_default(StateStore::own_files),
        };
        let app_state = cx.new(|cx| AppState::with_options(launch, options, cx));
        let explorer = cx.new(|cx| ExplorerPanel::new(app_state.clone(), window, cx));
        let stage = cx.new(|cx| Stage::new(app_state.clone(), window, cx));
        let split = cx.new(|_| ResizableState::default());
        let subscriptions = vec![
            cx.observe_in(&app_state, window, |this, _, window, cx| {
                this.sync_title(window, cx);
                this.sync_presentation(window, cx);
                this.sync_menus(cx);
                this.schedule_save(cx);
            }),
            cx.observe_window_appearance(window, |_, window, cx| theme::follow_system(window, cx)),
            cx.observe_window_bounds(window, |this, window, cx| {
                this.follow_fullscreen(window);
                this.window = Some(this.place(window, cx));
                this.schedule_save(cx);
            }),
            cx.subscribe(&split, |this, _, _: &ResizablePanelEvent, cx| {
                this.schedule_save(cx);
            }),
            cx.on_app_quit(|this, cx| {
                let saved = this.save(cx);
                async move {
                    if let Some(saved) = saved {
                        saved.await;
                    }
                }
            }),
        ];
        window.focus(&explorer.focus_handle(cx), cx);

        let mut view = Self {
            state: app_state,
            explorer,
            stage,
            split,
            explorer_width: px(fit_explorer(
                session.explorer_width.unwrap_or(EXPLORER_WIDTH),
                window.viewport_size().width.as_f32(),
            )),
            title: None,
            presenting: false,
            fullscreen: Fullscreen {
                entered: false,
                was: window.is_fullscreen(),
            },
            window: None,
            windowed: session.opened_at,
            store: session.store,
            save_task: None,
            menu_shows_ignored: false,
            _subscriptions: subscriptions,
        };
        view.window = Some(view.place(window, cx));
        view.sync_title(window, cx);
        view.sync_menus(cx);
        view
    }

    /// The shared application state.
    #[must_use]
    pub fn state(&self) -> &Entity<AppState> {
        &self.state
    }

    /// What the stage holds and how often it has rendered: a probe that
    /// tests use to see that work in the explorer leaves the stage alone
    /// and that its caches stay within their budgets.
    #[must_use]
    pub fn stage_probe(&self, cx: &App) -> StageProbe {
        self.stage.read(cx).probe(cx)
    }

    /// What the gallery holds: its layout, the rows it drew and its cache
    /// of thumbnails.
    #[must_use]
    pub fn gallery_probe(&self, cx: &App) -> GalleryProbe {
        self.stage.read(cx).gallery_probe(cx)
    }

    /// The title this view gave the window: the folder's name with one
    /// folder open, the application's name otherwise.
    #[must_use]
    pub fn title(&self) -> Option<&SharedString> {
        self.title.as_ref()
    }

    /// The explorer's width as laid out, in pixels; before the first
    /// layout, the width it starts at.
    #[must_use]
    pub fn explorer_width(&self, cx: &App) -> f32 {
        self.split
            .read(cx)
            .sizes()
            .first()
            .copied()
            .filter(|width| *width > px(0.))
            .unwrap_or(self.explorer_width)
            .as_f32()
    }

    /// Opens `paths` beside the folders on screen: the system's "Open
    /// With", and folders or files dropped on the Dock icon. A file opens
    /// its folder and shows on the stage; several files show as the
    /// gallery.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.open_paths(paths, Vec::new(), cx);
        });
    }

    fn sync_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = self.state.read(cx).title();
        if self.title.as_ref() != Some(&title) {
            window.set_window_title(&title);
            self.title = Some(title);
        }
    }

    /// Keeps the menu bar's Show Ignored check on the state.
    fn sync_menus(&mut self, cx: &mut Context<Self>) {
        let shows_ignored = self.state.read(cx).shows_ignored();
        if shows_ignored != self.menu_shows_ignored {
            self.menu_shows_ignored = shows_ignored;
            menus::install(shows_ignored, cx);
        }
    }

    /// Follows the presentation: the stage alone or beside the explorer,
    /// and the window fullscreen while presenting. Leaving restores the
    /// window's fullscreen state from before; if the window has left
    /// fullscreen meanwhile (the green button), it stays windowed. A
    /// presentation with no file left to show ends: a refresh removed the
    /// file and every gallery file.
    fn sync_presentation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (presenting, has_file) = {
            let state = self.state.read(cx);
            (state.is_presenting(), state.current().is_some())
        };
        if presenting && !has_file {
            // This runs again once the flag is down.
            return self.state.update(cx, AppState::stop_presenting);
        }
        if presenting == self.presenting {
            return;
        }
        self.presenting = presenting;
        if presenting {
            self.fullscreen.entered = !window.is_fullscreen();
            if self.fullscreen.entered {
                window.toggle_fullscreen();
            }
        } else if std::mem::take(&mut self.fullscreen.entered) && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        cx.notify();
    }

    /// Follows the window in and out of fullscreen, which changes its
    /// bounds. Once the person takes the window out of the fullscreen the
    /// presentation made (the green button, the system gesture), the
    /// window is theirs: leaving the presentation no longer changes it,
    /// even if they make it fullscreen again meanwhile.
    fn follow_fullscreen(&mut self, window: &Window) {
        let fullscreen = window.is_fullscreen();
        if self.presenting && self.fullscreen.entered && self.fullscreen.was && !fullscreen {
            self.fullscreen.entered = false;
        }
        self.fullscreen.was = fullscreen;
    }

    // Saving

    /// Where the window is, to restore, as a window opens with it: the
    /// top-left corner of its frame and the size of its content. (GPUI
    /// reports a window's frame, title bar included, but opens a window at
    /// the content size it is given, so saving the frame grew the window
    /// by its title bar at each launch.) Fullscreen is saved with the
    /// window it returns to; a fullscreen the presentation made is saved as
    /// that window.
    fn place(&mut self, window: &Window, cx: &App) -> SavedWindow {
        let bounds = if window.is_fullscreen() {
            let windowed = self
                .windowed
                .unwrap_or_else(|| window.window_bounds().get_bounds());
            if self.fullscreen.entered {
                WindowBounds::Windowed(windowed)
            } else {
                WindowBounds::Fullscreen(windowed)
            }
        } else {
            let content = Bounds::new(window.bounds().origin, window.viewport_size());
            self.windowed = Some(content);
            match window.window_bounds() {
                WindowBounds::Maximized(_) => WindowBounds::Maximized(content),
                _ => WindowBounds::Windowed(content),
            }
        };
        let display = window
            .display(cx)
            .and_then(|display| display.uuid().ok())
            .map(|uuid| uuid.to_string());
        SavedWindow { bounds, display }
    }

    /// Saves after [`SAVE_DELAY`], unless a save is already waiting: that
    /// one will take this change too.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if self.store.is_none() || self.save_task.is_some() {
            return;
        }
        let delay = cx.background_executor().timer(SAVE_DELAY);
        self.save_task = Some(cx.spawn(async move |this, cx| {
            delay.await;
            // Dropping this task's own handle ends it here, so the write
            // goes on alone.
            this.update(cx, |this, cx| {
                this.save_task = None;
                if let Some(write) = this.save(cx) {
                    write.detach();
                }
            })
            .unwrap_or_default();
        }));
    }

    /// Saves now: copies the state out here and writes it on the
    /// background executor. Returns the write, or `None` without a store.
    /// A write that fails leaves the last good file in place.
    pub fn save(&mut self, cx: &mut Context<Self>) -> Option<Task<()>> {
        let store = self.store.clone()?;
        let view = self.state.read(cx).take_snapshot();
        let snapshot = Snapshot {
            view,
            explorer_width: Some(self.explorer_width(cx)),
            window: self.window.clone(),
        };
        let ticket = store.ticket();
        Some(cx.background_spawn(async move {
            // A failed write leaves the last good file, and the next save
            // tries again; there is no one to tell.
            let _written = store.write(ticket, &snapshot.saved());
        }))
    }

    // Actions

    fn add_folder_picker(&mut self, _: &AddFolder, _: &mut Window, cx: &mut Context<Self>) {
        self.state
            .update(cx, |_, cx| AppState::prompt_for_folders(cx));
    }

    fn start_presenting(
        &mut self,
        _: &StartPresenting,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.read(cx).current().is_some() {
            window.focus(&self.stage.focus_handle(cx), cx);
        }
        self.state.update(cx, AppState::start_presenting);
    }

    fn toggle_show_ignored(
        &mut self,
        _: &ToggleShowIgnored,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, AppState::toggle_show_ignored);
    }

    fn refresh(&mut self, _: &Refresh, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, AppState::refresh);
    }

    /// The file or folder a command about one acts on: the file on the
    /// stage while the stage holds focus, the focus row otherwise.
    fn pointed(&self, window: &Window, cx: &App) -> Option<PathBuf> {
        let on_stage = self.focused_pane(window, cx) == Some(Pane::Stage);
        self.state.read(cx).pointed(on_stage)
    }

    fn reveal(&mut self, _: &Reveal, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.pointed(window, cx) {
            cx.reveal_path(&path);
        }
    }

    fn copy_path(&mut self, _: &CopyPath, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.pointed(window, cx) {
            let text = path.to_string_lossy().into_owned();
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    // Panes

    fn next_pane(&mut self, _: &NextPane, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_pane(true, window, cx);
    }

    fn previous_pane(&mut self, _: &PreviousPane, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle_pane(false, window, cx);
    }

    fn focus_filter(&mut self, _: &FocusFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_pane(Pane::Filter, window, cx);
    }

    fn focus_explorer(&mut self, _: &FocusExplorer, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_pane(Pane::Tree, window, cx);
    }

    /// The pane holding keyboard focus, if any.
    fn focused_pane(&self, window: &Window, cx: &App) -> Option<Pane> {
        match self.explorer.read(cx).focused(window, cx) {
            Some(ExplorerFocus::Filter) => Some(Pane::Filter),
            Some(ExplorerFocus::Tree) => Some(Pane::Tree),
            None if self.stage.focus_handle(cx).contains_focused(window, cx) => Some(Pane::Stage),
            None => None,
        }
    }

    /// Tab and shift-Tab: filter, tree, stage, round again. The filter is
    /// skipped while no folder is open.
    fn cycle_pane(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let order: &[Pane] = if self.explorer.read(cx).filter_on_screen(cx) {
            &[Pane::Filter, Pane::Tree, Pane::Stage]
        } else {
            &[Pane::Tree, Pane::Stage]
        };
        let here = self
            .focused_pane(window, cx)
            .and_then(|pane| order.iter().position(|&each| each == pane));
        let next = match (here, forward) {
            (Some(ix), true) => (ix + 1) % order.len(),
            (Some(ix), false) => (ix + order.len() - 1) % order.len(),
            (None, true) => 0,
            (None, false) => order.len() - 1,
        };
        self.focus_pane(order[next], window, cx);
    }

    fn focus_pane(&mut self, pane: Pane, window: &mut Window, cx: &mut Context<Self>) {
        match pane {
            Pane::Filter => self
                .explorer
                .update(cx, |explorer, cx| explorer.focus_filter(window, cx)),
            Pane::Tree => self
                .explorer
                .update(cx, |explorer, cx| explorer.focus_tree(window, cx)),
            Pane::Stage => {
                window.focus(&self.stage.focus_handle(cx), cx);
                self.stage.update(cx, Stage::focused_by_key);
            }
        }
    }
}

/// The explorer's width in a window `window` pixels wide: at least its
/// minimum, and otherwise narrow enough to leave the stage its own.
fn fit_explorer(width: f32, window: f32) -> f32 {
    width.min(window - STAGE_MIN_WIDTH).max(EXPLORER_MIN_WIDTH)
}

/// The layout of a cached pane: it fills its side of the split. A cached
/// view is laid out from this style instead of being measured.
fn pane() -> StyleRefinement {
    StyleRefinement::default().size_full()
}

impl Render for AppView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .size_full()
            .on_action(cx.listener(Self::next_pane))
            .on_action(cx.listener(Self::previous_pane))
            .on_action(cx.listener(Self::focus_filter))
            .on_action(cx.listener(Self::focus_explorer))
            .on_action(cx.listener(Self::start_presenting))
            .on_action(cx.listener(Self::toggle_show_ignored))
            .on_action(cx.listener(Self::refresh))
            .on_action(cx.listener(Self::reveal))
            .on_action(cx.listener(Self::copy_path))
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &Zoom, window, _| window.zoom_window())
            .on_action(|_: &ToggleFullScreen, window, _| window.toggle_fullscreen())
            // Closing the window quits, as its close button does.
            .on_action(|_: &CloseWindow, _, cx| cx.quit());
        if self.presenting {
            // No Add Folder while presenting, from a key or the menu bar,
            // whose item then shows disabled.
            return root.child(self.stage.clone().cached(pane()));
        }
        // Folders and files dropped on the window open as those dropped on
        // the Dock icon do.
        root.on_drop(cx.listener(|this, dropped: &ExternalPaths, _, cx| {
            this.open_paths(dropped.paths().to_vec(), cx);
        }))
        .on_action(cx.listener(Self::add_folder_picker))
        .child(
            h_resizable("panes")
                .with_state(&self.split)
                .child(
                    resizable_panel()
                        .size(self.explorer_width)
                        .size_range(px(EXPLORER_MIN_WIDTH)..Pixels::MAX)
                        .child(
                            div()
                                .id("explorer-pane")
                                .test_support()
                                .size_full()
                                .child(self.explorer.clone().cached(pane())),
                        ),
                )
                .child(
                    resizable_panel()
                        .size_range(px(STAGE_MIN_WIDTH)..Pixels::MAX)
                        .child(
                            div()
                                .id("stage-pane")
                                .test_support()
                                .size_full()
                                .child(self.stage.clone().cached(pane())),
                        ),
                ),
        )
    }
}
