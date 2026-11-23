//! Live refresh: a recursive file watcher on the open folder.
//!
//! The watcher runs on its own thread and sends what it sees into a
//! channel. A task on the UI thread waits for the first change, lets more
//! arrive for 150 ms on GPUI's timer, then asks the engine, off the UI
//! thread, whether any of them is something a new scan would show
//! ([`explorer::changes_matter`]). Changes the ignore rules hide (a
//! build's `target/`, `node_modules/`, `.git/`) stop there. Anything
//! else rescans the whole folder, as Refresh does, and the new tree
//! carries checks, open folders, the stage and focus across by path.
//!
//! A change while a scan runs asks for one more scan once it lands, never
//! more. A watcher error, or a watcher that lost track, rescans once. A
//! watcher that cannot start leaves the folder unfollowed, and the
//! toolbar's Refresh button says so.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use explorer::{ScanOptions, Tree, changes_matter};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui_kit::{AppContext as _, AsyncApp, Context, Result, SharedString, WeakEntity};
use notify::{Config, Event, EventKindMask, RecommendedWatcher, RecursiveMode, Watcher as _};

use super::scan::os_message;
use super::{AppState, ScanStatus};

/// How long changes gather after the first before they are looked at.
const WINDOW: Duration = Duration::from_millis(150);

/// How an [`AppState`] follows its folder.
#[derive(Clone, Debug)]
pub(super) struct Watching {
    /// Changes under this folder never matter: the app's own state.
    pub(super) quiet: Option<PathBuf>,
}

/// What the watcher's thread reports.
#[derive(Debug)]
enum Change {
    /// Something happened at these paths.
    Paths(Vec<PathBuf>),
    /// The watcher lost track or failed: only a full rescan is safe.
    Rescan,
}

/// The changes gathered in one window.
#[derive(Debug, Default)]
struct Batch {
    paths: Vec<PathBuf>,
    rescan: bool,
}

impl Batch {
    fn add(&mut self, change: Change) {
        match change {
            Change::Paths(paths) => self.paths.extend(paths),
            Change::Rescan => self.rescan = true,
        }
    }
}

impl AppState {
    /// Why the folder is not followed on disk: the watcher could not start.
    #[must_use]
    pub fn watch_error(&self) -> Option<&SharedString> {
        self.watch_error.as_ref()
    }

    /// Something under the folder changed on disk: rescan it, keeping the
    /// tree on screen. If a scan is running, rescan once more when it
    /// lands, however many changes arrive meanwhile.
    pub fn disk_changed(&mut self, cx: &mut Context<Self>) {
        if self.root.is_none() {
            return;
        }
        if matches!(self.status, ScanStatus::Scanning { .. }) {
            self.rescan_pending = true;
        } else {
            self.refresh(cx);
        }
    }

    /// Starts following `root` if this state follows its folder, and stops
    /// following the folder before.
    pub(super) fn follow(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self.watch_task = None;
        self.rescan_pending = false;
        if self.watch_error.take().is_some() {
            cx.notify();
        }
        let Some(watching) = &self.watching else {
            return;
        };
        let quiet = watching.quiet.clone();
        self.watch_task =
            Some(cx.spawn(async move |this, cx| follow_folder(this, root, quiet, cx).await));
    }

    /// The tree on screen and the rules it was scanned with, for deciding
    /// whether changes matter. `None` until a tree is on screen.
    fn watched(&self) -> Option<(Arc<Tree>, ScanOptions)> {
        let explorer = self.explorer.as_ref()?;
        let options = ScanOptions {
            show_ignored: self.show_ignored,
        };
        Some((Arc::clone(explorer.tree()), options))
    }

    fn watch_failed(&mut self, message: &str, cx: &mut Context<Self>) {
        self.watch_error = Some(os_message(message).to_owned().into());
        cx.notify();
    }
}

/// Follows `root` until the task is dropped: starts the watcher off the UI
/// thread, then turns each window of changes into at most one rescan.
async fn follow_folder(
    this: WeakEntity<AppState>,
    root: PathBuf,
    quiet: Option<PathBuf>,
    cx: &mut AsyncApp,
) -> Result<()> {
    let (sender, mut changes) = unbounded();
    let started = cx
        .background_spawn(async move { watch(&root, quiet, sender) })
        .await;
    // Held for as long as the task runs; dropping it stops the events.
    let _watcher = match started {
        Ok(watcher) => watcher,
        Err(err) => return this.update(cx, |this, cx| this.watch_failed(&err.to_string(), cx)),
    };
    while let Some(first) = changes.next().await {
        let mut batch = Batch::default();
        batch.add(first);
        cx.background_executor().timer(WINDOW).await;
        while let Ok(change) = changes.try_recv() {
            batch.add(change);
        }
        let watched = this.read_with(cx, |this, _| this.watched())?;
        let matters = match watched {
            Some((tree, options)) if !batch.rescan => {
                let paths = batch.paths;
                cx.background_spawn(async move { changes_matter(&tree, options, &paths) })
                    .await
            }
            // No tree yet: the first scan is running, and settles this.
            _ => true,
        };
        if matters {
            this.update(cx, AppState::disk_changed)?;
        }
    }
    Ok(())
}

/// A watcher on `root`, made canonical so the paths it reports start with
/// the scanned tree's root, sending what it sees to `sender`. Paths under
/// `quiet` are dropped.
fn watch(
    root: &Path,
    quiet: Option<PathBuf>,
    sender: UnboundedSender<Change>,
) -> notify::Result<RecommendedWatcher> {
    let root = std::fs::canonicalize(root).map_err(notify::Error::io)?;
    let quiet = quiet.map(|quiet| std::fs::canonicalize(&quiet).unwrap_or(quiet));
    let handler = move |event: notify::Result<Event>| {
        let change = match event {
            Ok(event) if event.need_rescan() && event.paths.is_empty() => Change::Rescan,
            Ok(event) => {
                let paths: Vec<PathBuf> = event
                    .paths
                    .into_iter()
                    .filter(|path| quiet.as_ref().is_none_or(|quiet| !path.starts_with(quiet)))
                    .collect();
                if paths.is_empty() {
                    return;
                }
                Change::Paths(paths)
            }
            Err(_) => Change::Rescan,
        };
        // Once the folder is closed nothing listens, and the change goes
        // nowhere.
        sender.unbounded_send(change).unwrap_or_default();
    };
    // Creations, removals and modifications; reads never matter, and on
    // some platforms the scan's own reads would report them.
    let config = Config::default().with_event_kinds(EventKindMask::CORE);
    let mut watcher = RecommendedWatcher::new(handler, config)?;
    watcher.watch(&root, RecursiveMode::Recursive)?;
    Ok(watcher)
}
