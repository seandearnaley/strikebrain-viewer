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
    App, AppContext as _, Bounds, Context, Entity, Focusable as _, IntoElement, ParentElement as _,
    Pixels, Render, SharedString, StyleRefinement, Styled as _, Subscription, Task,
    TestSupportExt as _, Window, WindowBounds, div, prelude::*, px,
};

use crate::explorer_panel::{ExplorerFocus, ExplorerPanel};
use crate::keys::{
    FocusExplorer, FocusFilter, Minimize, NextPane, OpenFolder, PreviousPane, Refresh,
    StartPresenting, ToggleShowIgnored, Zoom,
};
use crate::menus;
use crate::persist::{FolderView, SavedFolder, SavedWindow, Snapshot, StateStore};
use crate::stage::{Stage, StageProbe};
use crate::state::{AppState, Launch, StateOptions};
use crate::theme;

/// Starting width of the explorer pane, in pixels.
pub const EXPLORER_WIDTH: f32 = 300.0;

/// Narrowest width the explorer pane can be dragged to, in pixels.
pub const EXPLORER_MIN_WIDTH: f32 = 220.0;

/// How long a change waits before it is saved; changes meanwhile ride
/// along with it.
pub const SAVE_DELAY: Duration = Duration::from_millis(500);

/// What a window brings back from a saved state, where it saves, and
/// whether it follows its folder on disk. The default does none of these.
#[derive(Debug, Default)]
pub struct Session {
    /// The saved view of the folder being opened.
    pub view: Option<FolderView>,
    /// The saved folder, when this launch does not open it: saved as it
    /// was until another folder opens.
    pub kept: Option<SavedFolder>,
    /// The explorer's saved width, in pixels.
    pub explorer_width: Option<f32>,
    /// Where to save. `None` saves nothing.
    pub store: Option<StateStore>,
    /// Rescan when the folder changes on disk.
    pub watch: bool,
    /// The bounds the window opened with, as [`SavedWindow`] holds them:
    /// what a fullscreen window is saved with until it has been windowed.
    pub opened_at: Option<Bounds<Pixels>>,
}

/// A pane that takes keyboard focus, in Tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pane {
    Filter,
    Tree,
    Stage,
}

/// The root view: explorer on the left, stage on the right. Owns the
/// [`AppState`] both observe, keeps the window title on the folder's name,
/// handles ⌘O, ⌘F, moving between panes and the menu bar's commands,
/// follows the presentation, and saves the state.
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
    /// Whether the presentation made the window fullscreen, so leaving it
    /// should make the window windowed again.
    entered_fullscreen: bool,
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
    /// tree takes keyboard focus. Nothing is restored or saved, and the
    /// folder is not followed on disk.
    pub fn new(launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_session(launch, Session::default(), window, cx)
    }

    /// Builds the window's content as [`AppView::new`] does, restoring,
    /// saving and following the folder as `session` says.
    pub fn with_session(
        launch: Launch,
        session: Session,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = StateOptions {
            view: session.view,
            kept: session.kept,
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
            explorer_width: px(session
                .explorer_width
                .unwrap_or(EXPLORER_WIDTH)
                .max(EXPLORER_MIN_WIDTH)),
            title: None,
            presenting: false,
            entered_fullscreen: false,
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

    /// The title this view gave the window: the open folder's name.
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

    /// Opens `folder` in place of the folder on screen: the system's
    /// "Open With" and a folder dropped on the Dock icon.
    pub fn open_folder(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.open_folder(folder, cx));
    }

    fn sync_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.state.read(cx).folder_name();
        if name.is_some() && name != self.title {
            if let Some(name) = &name {
                window.set_window_title(name);
            }
            self.title = name;
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
            self.entered_fullscreen = !window.is_fullscreen();
            if self.entered_fullscreen {
                window.toggle_fullscreen();
            }
        } else if std::mem::take(&mut self.entered_fullscreen) && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        cx.notify();
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
            if self.entered_fullscreen {
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
        let top_row = self.explorer.read(cx).top_row(cx);
        let (root, view) = self
            .state
            .update(cx, |state, _| state.take_snapshot(top_row));
        let snapshot = Snapshot {
            root,
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

    fn open_folder_picker(&mut self, _: &OpenFolder, _: &mut Window, cx: &mut Context<Self>) {
        self.state
            .update(cx, |_, cx| AppState::prompt_for_folder(cx));
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
            Pane::Stage => window.focus(&self.stage.focus_handle(cx), cx),
        }
    }
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
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &Zoom, window, _| window.zoom_window());
        if self.presenting {
            // No Open Folder while presenting, from a key or the menu bar,
            // whose item then shows disabled.
            return root.child(self.stage.clone().cached(pane()));
        }
        root.on_action(cx.listener(Self::open_folder_picker)).child(
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
                    resizable_panel().child(
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
