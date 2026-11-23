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
//! Changes that arrive while the folder's first scan runs wait for its
//! tree and are judged against it, so a busy ignored folder never forces a
//! second scan. A change while a scan runs asks for one more scan once it
//! lands, never more. A watcher error, or a watcher that lost track,
//! rescans once. A watcher that cannot start leaves the folder unfollowed,
//! and the toolbar's Refresh button says so; Refresh tries again.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use explorer::{NodeId, ScanOptions, Tree, changes_matter};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui_kit::{AppContext as _, AsyncApp, Context, Result, SharedString, WeakEntity};
use notify::{Config, Event, EventKindMask, RecommendedWatcher, RecursiveMode, Watcher as _};

use super::scan::os_message;
use super::{AppState, ScanStatus};

/// How long changes gather after the first before they are looked at.
const WINDOW: Duration = Duration::from_millis(150);

/// How an [`AppState`] follows its folder.
#[derive(Clone, Debug)]
pub(super) struct Watching {
    /// Changes to these files never matter: the app's own state file and
    /// its temporary name, should the open folder hold them.
    pub(super) quiet: Vec<PathBuf>,
}

/// What a batch of changes is judged against.
enum Watched {
    /// The tree on screen and the rules it was scanned with.
    Tree(Arc<Tree>, ScanOptions),
    /// No tree yet: the folder's first scan runs, and its tree will judge.
    FirstScan,
    /// No tree and no scan: only a scan can tell.
    Nothing,
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

    /// What decides whether changes matter: the tree on screen and the
    /// rules it was scanned with, once there is one.
    fn watched(&self) -> Watched {
        match &self.explorer {
            Some(explorer) => {
                let options = ScanOptions {
                    show_ignored: self.show_ignored,
                };
                Watched::Tree(Arc::clone(explorer.tree()), options)
            }
            None if matches!(self.status, ScanStatus::Scanning { .. }) => Watched::FirstScan,
            None => Watched::Nothing,
        }
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
    quiet: Vec<PathBuf>,
    cx: &mut AsyncApp,
) -> Result<()> {
    let (sender, changes) = unbounded();
    let started = cx
        .background_spawn(async move { watch(&root, &quiet, sender) })
        .await;
    // Held for as long as the task runs; dropping it stops the events.
    let _watcher = match started {
        Ok(watcher) => watcher,
        Err(err) => return this.update(cx, |this, cx| this.watch_failed(&err.to_string(), cx)),
    };
    follow_changes(this, changes, cx).await
}

/// Turns each window of `changes` into at most one rescan, until the
/// channel closes or the state is gone.
async fn follow_changes(
    this: WeakEntity<AppState>,
    mut changes: UnboundedReceiver<Change>,
    cx: &mut AsyncApp,
) -> Result<()> {
    while let Some(first) = changes.next().await {
        let mut batch = Batch::default();
        batch.add(first);
        let matters = loop {
            cx.background_executor().timer(WINDOW).await;
            while let Ok(change) = changes.try_recv() {
                batch.add(change);
            }
            if batch.rescan {
                break true;
            }
            match this.read_with(cx, |this, _| this.watched())? {
                Watched::Tree(tree, options) => {
                    let paths = std::mem::take(&mut batch.paths);
                    break cx
                        .background_spawn(async move {
                            changes_matter(&tree, NodeId::ROOT, options, &paths)
                        })
                        .await;
                }
                // The first scan runs: the batch waits for its tree, which
                // may well show these changes already.
                Watched::FirstScan => {}
                Watched::Nothing => break true,
            }
        };
        if matters {
            this.update(cx, AppState::disk_changed)?;
        }
    }
    Ok(())
}

/// A watcher on `root`, made canonical so the paths it reports start with
/// the scanned tree's root, sending what it sees to `sender`. Changes to
/// the `quiet` files are dropped.
fn watch(
    root: &Path,
    quiet: &[PathBuf],
    sender: UnboundedSender<Change>,
) -> notify::Result<RecommendedWatcher> {
    let root = std::fs::canonicalize(root).map_err(notify::Error::io)?;
    let quiet: Vec<PathBuf> = quiet.iter().map(|path| canonical_file(path)).collect();
    let handler = move |event: notify::Result<Event>| {
        let change = match event {
            Ok(event) if event.need_rescan() && event.paths.is_empty() => Change::Rescan,
            Ok(event) => {
                let paths: Vec<PathBuf> = event
                    .paths
                    .into_iter()
                    .filter(|path| !quiet.contains(path))
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

/// A file's path with its folder made canonical, as the watcher reports
/// it: the file itself need not exist yet.
fn canonical_file(path: &Path) -> PathBuf {
    let canonical = path
        .parent()
        .and_then(|folder| std::fs::canonicalize(folder).ok())
        .zip(path.file_name())
        .map(|(folder, name)| folder.join(name));
    canonical.unwrap_or_else(|| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use futures::channel::mpsc::unbounded;
    use gpui_kit::{AppContext as _, Entity, TestAppContext};

    use super::{Change, WINDOW, canonical_file, follow_changes};
    use crate::state::{AppState, Launch, ScanStatus, StateOptions};

    /// A state whose folder's first scan runs and never lands by itself,
    /// fed changes by hand.
    fn first_scan_running(
        cx: &mut TestAppContext,
        root: &std::path::Path,
    ) -> (
        Entity<AppState>,
        futures::channel::mpsc::UnboundedSender<Change>,
    ) {
        let state = cx.new(|cx| AppState::new(Launch::Empty, cx));
        let (sender, changes) = unbounded();
        state.update(cx, |state, cx| {
            state.root = Some(root.to_path_buf());
            state.status = ScanStatus::Scanning { slow: false };
            state.watch_task =
                Some(cx.spawn(async move |this, cx| follow_changes(this, changes, cx).await));
        });
        (state, sender)
    }

    #[gpui_kit::test]
    fn changes_during_the_first_scan_wait_for_its_tree(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/out"), "o").unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let (state, sender) = first_scan_running(cx, &root);

        // A build writes under the ignored `target/` while the scan runs.
        sender
            .unbounded_send(Change::Paths(vec![root.join("target/out")]))
            .unwrap();
        for _ in 0..3 {
            cx.executor().advance_clock(WINDOW);
            cx.run_until_parked();
        }
        assert!(!state.read_with(cx, |state, _| state.rescan_pending));

        // The tree lands: the change is judged against it, and does not
        // matter.
        let tree = explorer::scan(
            &root,
            explorer::ScanOptions::default(),
            &std::sync::atomic::AtomicBool::new(false),
        )
        .unwrap();
        state.update(cx, |state, _| state.show_tree(std::sync::Arc::new(tree)));
        cx.executor().advance_clock(WINDOW);
        cx.run_until_parked();
        let scans = |cx: &mut TestAppContext| state.read_with(cx, |state, _| state.scans());
        assert_eq!(scans(cx), 0);

        // A change the scan would show does.
        std::fs::write(root.join("b.txt"), "b").unwrap();
        sender
            .unbounded_send(Change::Paths(vec![root.join("b.txt")]))
            .unwrap();
        cx.executor().advance_clock(WINDOW);
        cx.run_until_parked();
        assert_eq!(scans(cx), 1);
    }

    #[gpui_kit::test]
    fn refresh_follows_again_a_folder_whose_watcher_failed(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        let options = StateOptions {
            watch: true,
            ..StateOptions::default()
        };
        let launch = Launch::Folder(dir.path().to_path_buf());
        let state = cx.new(|cx| AppState::with_options(launch, options, cx));
        cx.run_until_parked();
        assert!(state.read_with(cx, |state, _| state.explorer().is_some()));

        // The watcher stops, as one can at a watch limit, with the tree on
        // screen.
        state.update(cx, |state, cx| {
            state.watch_task = None;
            state.watch_failed("Too many open files (os error 24)", cx);
        });
        let error = state.read_with(cx, |state, _| state.watch_error().cloned());
        assert_eq!(error.as_deref(), Some("Too many open files"));

        state.update(cx, AppState::refresh);
        cx.run_until_parked();
        state.read_with(cx, |state, _| {
            assert_eq!(state.watch_error(), None);
            assert!(state.watch_task.is_some(), "followed again");
        });
    }

    #[test]
    fn the_state_file_is_quiet_but_not_its_neighbours() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        // Through another spelling of the folder, and for a file that does
        // not exist yet: the watcher reports canonical paths.
        let spelled = dir.path().join(".").join("state.json");
        assert_eq!(canonical_file(&spelled), folder.join("state.json"));
        let store = crate::persist::StateStore::at(dir.path().join("state.json"));
        let quiet: Vec<PathBuf> = store
            .own_files()
            .iter()
            .map(|path| canonical_file(path))
            .collect();
        assert!(quiet.contains(&folder.join("state.json")));
        assert!(quiet.contains(&folder.join(format!(".state.json.{}.tmp", std::process::id()))));
        assert!(!quiet.contains(&folder.join("notes.md")));
    }
}
