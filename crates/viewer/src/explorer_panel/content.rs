//! A submitted content query names its source. The folder tree and its
//! filename filter continue to work while inference runs on a worker.
mod render;
mod storage;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};

use super::ExplorerPanel;
use crate::search::{self, Event, Request, Results};
use futures::StreamExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::{App, AppContext as _, Context, Entity, Window};

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Offer,
    Working(String),
    Download(u64),
    Matches(Results),
    Paused,
    Pausing,
    Error(String),
    Unavailable(String),
    Ready,
    Storage(storage::Storage),
    Confirm(storage::Remove, String),
}

struct Job {
    stop: Arc<AtomicBool>,
    queries: Sender<String>,
}
impl Drop for Job {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub(super) struct Content {
    pub(super) enabled: bool,
    pub(super) input: Entity<InputState>,
    phase: Phase,
    folder: Option<PathBuf>,
    query: String,
    job: Option<Job>,
    generation: u64,
    installed: bool,
    store: Option<PathBuf>,
    saved_bytes: u64,
    selected: Option<PathBuf>,
    indexed_generation: u64,
    result_scroll: gpui_kit::UniformListScrollHandle,
}

impl Content {
    pub(super) fn new(
        store: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<ExplorerPanel>,
    ) -> Self {
        Self {
            store,
            saved_bytes: 0,
            selected: None,
            indexed_generation: 0,
            result_scroll: gpui_kit::UniformListScrollHandle::new(),
            enabled: false,
            input: cx.new(|cx| InputState::new(window, cx).placeholder("Search contents")),
            phase: Phase::Idle,
            folder: None,
            query: String::new(),
            job: None,
            generation: 0,
            installed: false,
        }
    }
}

impl ExplorerPanel {
    fn search_folder(&self, cx: &App) -> Option<PathBuf> {
        let state = self.state.read(cx);
        let tree = state.explorer()?.tree();
        let id = state
            .current()
            .or(state.focus())
            .or_else(|| tree.folders().next())?;
        if !tree.kind(id).is_folder() && self.content.selected.as_ref() == Some(&tree.abs_path(id))
        {
            return self.content.folder.clone();
        }
        let folder = if tree.kind(id).is_folder() {
            id
        } else {
            tree.folder_of(id)?
        };
        tree.kind(folder).is_folder().then(|| tree.abs_path(folder))
    }

    pub(super) fn submit_content(&mut self, cx: &mut Context<Self>) {
        let query = self.content.input.read(cx).value().trim().to_owned();
        if query.is_empty() {
            return;
        }
        let Some(folder) = self.search_folder(cx) else {
            self.content.phase = Phase::Unavailable("Select a folder to search".into());
            cx.notify();
            return;
        };
        if self.content.folder.as_ref() == Some(&folder)
            && self.content.indexed_generation == self.state.read(cx).staged().0
            && let Some(job) = &self.content.job
            && job.queries.send(query.clone()).is_ok()
        {
            self.content.query = query;
            cx.notify();
            return;
        }
        if self.content.job.is_some() {
            // Keep the active source visible until its worker has stopped.
            self.content.stop();
        }
        self.offer(Some(folder), query, cx);
    }

    fn offer(&mut self, folder: Option<PathBuf>, query: String, cx: &mut Context<Self>) {
        self.content.selected = None;
        self.content.folder = folder;
        self.content.query = query;
        self.content.phase = match self
            .content
            .store
            .clone()
            .ok_or("Local search storage is unavailable")
        {
            Ok(store) => {
                self.content.installed = search::installed(&store);
                match search::partial_bytes(&store) {
                    Ok(bytes) => self.content.saved_bytes = bytes,
                    Err(error) => {
                        self.content.phase = Phase::Error(error.to_string());
                        cx.notify();
                        return;
                    }
                }
                if self.content.installed
                    && self
                        .content
                        .folder
                        .as_deref()
                        .is_some_and(|root| search::index_path(&store, root).is_file())
                {
                    self.start_content(cx);
                    return;
                }
                Phase::Offer
            }
            Err(e) => Phase::Unavailable(e.to_string()),
        };
        cx.notify();
    }

    pub(super) fn dismiss_content(&mut self, cx: &mut Context<Self>) {
        if matches!(self.content.phase, Phase::Offer) {
            self.content.phase = Phase::Idle;
        } else {
            self.content.enabled = false;
        }
        cx.notify();
    }

    pub(super) fn start_content(&mut self, cx: &mut Context<Self>) {
        if self.content.job.is_some() {
            return;
        }
        let store = match self
            .content
            .store
            .clone()
            .ok_or("Local search storage is unavailable")
        {
            Ok(dir) => dir,
            Err(e) => {
                self.content.phase = Phase::Error(e.to_string());
                cx.notify();
                return;
            }
        };
        let folder = self.content.folder.clone();
        let request = Request {
            store,
            folder,
            query: self.content.query.clone(),
            show_ignored: self.state.read(cx).shows_ignored(),
            download: !self.content.installed,
        };
        let (commands, queries) = mpsc::channel();
        let (tx, mut events) = futures::channel::mpsc::unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let cancel = Arc::clone(&stop);
        let launched = std::thread::Builder::new()
            .name("content-search".into())
            .spawn(move || {
                let outcome = search::run(request, &cancel, &queries, |event| {
                    let _sent = tx.unbounded_send(Ok(Some(event)));
                });
                let _sent = tx.unbounded_send(outcome.map(|()| None).map_err(|e| format!("{e:#}")));
            });
        if let Err(error) = launched {
            self.content.phase = Phase::Error(error.to_string());
            cx.notify();
            return;
        }
        self.content.indexed_generation = self.state.read(cx).staged().0;
        self.content.generation += 1;
        let generation = self.content.generation;
        self.content.job = Some(Job {
            stop,
            queries: commands,
        });
        self.content.phase = Phase::Working(
            if self.content.installed {
                "Preparing search…"
            } else {
                "Connecting…"
            }
            .into(),
        );
        cx.notify();
        cx.spawn(async move |this, cx| {
            while let Some(message) = events.next().await {
                if this
                    .update(cx, |this, cx| {
                        this.search_event(generation, message, cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn search_event(
        &mut self,
        generation: u64,
        message: Result<Option<Event>, String>,
        cx: &mut Context<Self>,
    ) {
        if self.content.generation != generation {
            return;
        }
        if matches!(self.content.phase, Phase::Pausing) {
            if !matches!(message, Ok(Some(_))) {
                self.content.job = None;
                self.content.phase = Phase::Paused;
                cx.notify();
            }
            return;
        }
        self.content.phase = match message {
            Ok(Some(Event::Status(s))) => Phase::Working(s),
            Ok(Some(Event::Download(n))) => {
                self.content.saved_bytes = n;
                Phase::Download(n)
            }
            Ok(Some(Event::Results(r))) => {
                self.content.installed = true;
                Phase::Matches(r)
            }
            Ok(None) => {
                self.content.job = None;
                self.content.installed = true;
                Phase::Ready
            }
            Err(s) => {
                self.content.job = None;
                Phase::Error(s)
            }
        };
        cx.notify();
    }

    fn pause_content(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = self.content.job.as_ref() {
            job.stop.store(true, Ordering::Relaxed);
            let _wake = job.queries.send(String::new());
        }
        self.content.phase = Phase::Pausing;
        cx.notify();
    }

    fn open_hit(&mut self, path: &Path, cx: &mut Context<Self>) {
        if matches!(&self.content.phase, Phase::Matches(r) if r.unavailable.is_some()) {
            return;
        }
        self.content.selected = Some(path.to_owned());
        self.state.update(cx, |state, cx| {
            if let Some(id) = state.explorer().and_then(|e| e.tree().find_abs(path)) {
                state.show(id, cx);
            } else {
                state.open_paths(vec![path.to_owned()], Vec::new(), cx);
            }
        });
    }

    fn move_hit(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Phase::Matches(r) = &self.content.phase else {
            return;
        };
        if r.hits.is_empty() {
            return;
        }
        let pos = self
            .content
            .selected
            .as_ref()
            .and_then(|p| r.hits.iter().position(|hit| hit == p));
        let at = pos.map_or(0, |i| {
            if forward {
                (i + 1).min(r.hits.len() - 1)
            } else {
                i.saturating_sub(1)
            }
        });
        let hit = r.hits[at].clone();
        self.content
            .result_scroll
            .scroll_to_item(at, gpui_kit::ScrollStrategy::Nearest);
        self.open_hit(&hit, cx);
    }

    fn activate_hit(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.content.selected.clone() {
            self.open_hit(&path, cx);
        } else {
            self.move_hit(true, cx);
        }
    }

    pub(super) fn edit_content(&mut self, cx: &mut Context<Self>) {
        let value = self.content.input.read(cx).value().trim().to_owned();
        if self.content.enabled
            && self.content.folder.is_some()
            && self.content.folder == self.search_folder(cx)
        {
            self.content.query.clone_from(&value);
            if !value.is_empty()
                && let Some(job) = &self.content.job
            {
                let _alive = job.queries.send(value);
            }
        }
        cx.notify();
    }
}
