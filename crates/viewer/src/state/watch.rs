//! Live refresh: a recursive file watcher on each open folder, and one on
//! each file opened on its own, alone.
//!
//! Each watcher runs on its own thread and sends what it sees into a
//! channel. A task on the UI thread waits for the first change, lets more
//! arrive for 150 ms on GPUI's timer, then asks the engine, off the UI
//! thread, whether any of them is something a new scan of that folder would
//! show ([`explorer::changes_matter`]). Changes the ignore rules hide (a
//! build's `target/`, `node_modules/`, `.git/`) stop there, as does a
//! folder's own change of times, which Windows reports beside each entry
//! added to it or taken from it. Anything else rescans that folder alone,
//! as Refresh does, and its new tree carries checks, open folders, the
//! stage and focus across by path.
//!
//! Changes that arrive while a folder's first scan runs wait for its tree
//! and are judged against it, so a busy ignored folder never forces a
//! second scan. A change while a scan runs asks for one more scan of that
//! folder once it lands, never more. A watcher error, or a watcher that
//! lost track, rescans once. A watcher that cannot start leaves its folder
//! unfollowed, and the title bar's Refresh button says so; Refresh tries
//! again.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use explorer::{NodeId, ScanOptions, Tree, changes_matter};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui_kit::{AppContext as _, AsyncApp, Context, Result, SharedString, WeakEntity};
use notify::{Config, Event, EventKindMask, RecommendedWatcher, RecursiveMode, Watcher as _};

use super::AppState;
use super::scan::os_message;

/// How long changes gather after the first before they are looked at.
const WINDOW: Duration = Duration::from_millis(150);

/// How an [`AppState`] follows its folders.
#[derive(Clone, Debug)]
pub(super) struct Watching {
    /// Changes to these files never matter: the app's own state file and
    /// its temporary name, should an open folder hold them.
    pub(super) quiet: Vec<PathBuf>,
}

/// What a batch of one folder's changes is judged against.
enum Watched {
    /// The tree on screen, the folder's node in it, and the rules it was
    /// scanned with.
    Tree(Arc<Tree>, NodeId, ScanOptions),
    /// No tree of it yet: its first scan runs, and its tree will judge.
    FirstScan,
    /// No tree of it that can judge: only a scan can tell.
    Nothing,
    /// The folder was removed.
    Gone,
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
    /// Why a folder is not followed on disk, the first of them: its
    /// watcher could not start.
    #[must_use]
    pub fn watch_error(&self) -> Option<&SharedString> {
        self.folders
            .iter()
            .find_map(|folder| folder.watch_error.as_ref())
    }

    /// Something changed on disk under every folder: rescan each, as its
    /// watcher would ask.
    pub fn disk_changed(&mut self, cx: &mut Context<Self>) {
        let keys: Vec<u64> = self.folders.iter().map(|folder| folder.key).collect();
        for key in keys {
            self.folder_changed(key, cx);
        }
    }

    /// Something under the folder `key` changed on disk: rescan it,
    /// keeping its tree on screen. If its scan is running, rescan once more
    /// when it lands, however many changes arrive meanwhile.
    fn folder_changed(&mut self, key: u64, cx: &mut Context<Self>) {
        let Some(folder) = self.folder_mut(key) else {
            return;
        };
        if folder.scanning.is_some() {
            folder.rescan_pending = true;
        } else {
            self.refresh_folder(key, cx);
        }
    }

    /// Starts following the folder `key` if this state follows its
    /// folders, replacing the watcher it had.
    pub(super) fn follow(&mut self, key: u64, cx: &mut Context<Self>) {
        let quiet = self
            .watching
            .as_ref()
            .map(|watching| watching.quiet.clone());
        let Some(folder) = self.folder_mut(key) else {
            return;
        };
        folder.watch_task = None;
        folder.rescan_pending = false;
        let had_error = folder.watch_error.take().is_some();
        if let Some(quiet) = quiet {
            let root = folder.path.clone();
            folder.watch_task = Some(
                cx.spawn(async move |this, cx| follow_folder(this, key, root, quiet, cx).await),
            );
        }
        if had_error {
            cx.notify();
        }
    }

    /// What decides whether the folder `key`'s changes matter: its tree on
    /// screen and the rules it was scanned with, once it has one that
    /// could be read.
    fn watched(&self, key: u64) -> Watched {
        let Some(folder) = self.folder(key) else {
            return Watched::Gone;
        };
        let top = self.folder_node(&folder.path);
        match (&self.explorer, top) {
            (Some(explorer), Some(top)) if folder.error.is_none() => {
                let options = ScanOptions {
                    show_ignored: self.show_ignored,
                };
                Watched::Tree(Arc::clone(explorer.tree()), top, options)
            }
            (_, None) if folder.scanning.is_some() => Watched::FirstScan,
            _ => Watched::Nothing,
        }
    }

    fn watch_failed(&mut self, key: u64, message: &str, cx: &mut Context<Self>) {
        if let Some(folder) = self.folder_mut(key) {
            folder.watch_error = Some(os_message(message).to_owned().into());
            cx.notify();
        }
    }
}

/// Follows the folder `key` at `root` until the task is dropped: starts
/// the watcher off the UI thread, then turns each window of changes into at
/// most one rescan.
async fn follow_folder(
    this: WeakEntity<AppState>,
    key: u64,
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
        Err(err) => {
            return this.update(cx, |this, cx| this.watch_failed(key, &err.to_string(), cx));
        }
    };
    follow_changes(this, key, changes, cx).await
}

/// Turns each window of the folder `key`'s `changes` into at most one
/// rescan of it, until the channel closes or the folder or the state is
/// gone.
async fn follow_changes(
    this: WeakEntity<AppState>,
    key: u64,
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
            match this.read_with(cx, |this, _| this.watched(key))? {
                Watched::Tree(tree, folder, options) => {
                    let paths = std::mem::take(&mut batch.paths);
                    break cx
                        .background_spawn(
                            async move { changes_matter(&tree, folder, options, &paths) },
                        )
                        .await;
                }
                // The first scan runs: the batch waits for its tree, which
                // may well show these changes already.
                Watched::FirstScan => {}
                Watched::Nothing => break true,
                Watched::Gone => return Ok(()),
            }
        };
        if matters {
            this.update(cx, |this, cx| this.folder_changed(key, cx))?;
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
    let root = explorer::canonical(root).map_err(notify::Error::io)?;
    let quiet: Vec<PathBuf> = quiet.iter().map(|path| canonical_file(path)).collect();
    let handler = move |event: notify::Result<Event>| {
        let change = match event {
            Ok(event) if event.need_rescan() && event.paths.is_empty() => Change::Rescan,
            #[cfg(windows)]
            Ok(event) if folder_times_only(&event) => return,
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
    // A file opened on its own is watched alone: its folder is never read.
    let mode = if root.is_dir() {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    watcher.watch(&root, mode)?;
    Ok(watcher)
}

/// Whether `event` says only that folders' own times or attributes
/// changed. Windows says so of a folder each time an entry in it is added,
/// removed or renamed: after the entry's own event, and often after the
/// window that gathered it, so it would cost a second rescan that finds
/// nothing new. A scan lists a folder the same way whatever its times; one
/// that can no longer be read is still a change.
#[cfg(windows)]
fn folder_times_only(event: &Event) -> bool {
    use notify::event::{EventKind, ModifyKind};

    event.kind == EventKind::Modify(ModifyKind::Any)
        && !event.need_rescan()
        && !event.paths.is_empty()
        && event.paths.iter().all(|path| {
            std::fs::metadata(path).is_ok_and(|found| found.is_dir())
                && std::fs::read_dir(path).is_ok()
        })
}

/// A file's path with its folder made canonical, as the watcher reports
/// it: the file itself need not exist yet.
fn canonical_file(path: &Path) -> PathBuf {
    let canonical = path
        .parent()
        .and_then(|folder| explorer::canonical(folder).ok())
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
    use crate::state::{AppState, Launch, StateOptions};

    /// A state whose folder's first scan runs and never lands by itself,
    /// fed changes by hand.
    fn first_scan_running(
        cx: &mut TestAppContext,
        root: &std::path::Path,
    ) -> (
        Entity<AppState>,
        futures::channel::mpsc::UnboundedSender<Change>,
    ) {
        let state = cx.new(|cx| AppState::new(Launch::Folders(vec![root.to_path_buf()]), cx));
        let (sender, changes) = unbounded();
        state.update(cx, |state, cx| {
            let folder = &mut state.folders[0];
            // The scan never lands: its task is gone.
            folder.scan_task = None;
            folder.watch_task =
                Some(cx.spawn(async move |this, cx| follow_changes(this, 0, changes, cx).await));
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
        let root = explorer::canonical(dir.path()).unwrap();
        let (state, sender) = first_scan_running(cx, &root);

        // A build writes under the ignored `target/` while the scan runs.
        sender
            .unbounded_send(Change::Paths(vec![root.join("target/out")]))
            .unwrap();
        for _ in 0..3 {
            cx.executor().advance_clock(WINDOW);
            cx.run_until_parked();
        }
        assert!(!state.read_with(cx, |state, _| state.folders[0].rescan_pending));

        // The tree lands: the change is judged against it, and does not
        // matter.
        let scans = |cx: &mut TestAppContext| state.read_with(cx, |state, _| state.scans());
        assert_eq!(scans(cx), 1);
        state.update(cx, |state, cx| {
            let tree = explorer::scan(
                &root,
                explorer::ScanOptions::default(),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
            let generation = state.folders[0].generation;
            state.finish_scan(0, generation, Ok(std::sync::Arc::new(tree)), cx);
        });
        cx.executor().advance_clock(WINDOW);
        cx.run_until_parked();
        assert_eq!(scans(cx), 1);

        // A change the scan would show does.
        std::fs::write(root.join("b.txt"), "b").unwrap();
        sender
            .unbounded_send(Change::Paths(vec![root.join("b.txt")]))
            .unwrap();
        cx.executor().advance_clock(WINDOW);
        cx.run_until_parked();
        assert_eq!(scans(cx), 2);
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
        let launch = Launch::Folders(vec![dir.path().to_path_buf()]);
        let state = cx.new(|cx| AppState::with_options(launch, options, cx));
        cx.run_until_parked();
        assert!(state.read_with(cx, |state, _| state.explorer().is_some()));

        // The watcher stops, as one can at a watch limit, with the tree on
        // screen.
        state.update(cx, |state, cx| {
            state.folders[0].watch_task = None;
            state.watch_failed(0, "Too many open files (os error 24)", cx);
        });
        let error = state.read_with(cx, |state, _| state.watch_error().cloned());
        assert_eq!(error.as_deref(), Some("Too many open files"));

        state.update(cx, AppState::refresh);
        cx.run_until_parked();
        state.read_with(cx, |state, _| {
            assert_eq!(state.watch_error(), None);
            assert!(state.folders[0].watch_task.is_some(), "followed again");
        });
    }

    #[cfg(windows)]
    #[test]
    fn a_folder_whose_times_changed_is_not_a_change() {
        use notify::Event;
        use notify::event::{CreateKind, EventKind, ModifyKind};

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        let event = |kind: EventKind, path: &str| Event::new(kind).add_path(dir.path().join(path));
        let modified = EventKind::Modify(ModifyKind::Any);
        assert!(super::folder_times_only(&event(modified, "src")));
        // A file's contents, a folder made, and a folder gone all matter.
        assert!(!super::folder_times_only(&event(modified, "src/lib.rs")));
        let made = EventKind::Create(CreateKind::Folder);
        assert!(!super::folder_times_only(&event(made, "src")));
        assert!(!super::folder_times_only(&event(modified, "gone")));
        // So does one the watcher asks to be rescanned.
        let rescan = event(modified, "src").set_flag(notify::event::Flag::Rescan);
        assert!(!super::folder_times_only(&rescan));
    }

    #[test]
    fn the_state_file_is_quiet_but_not_its_neighbours() {
        let dir = tempfile::tempdir().unwrap();
        let folder = explorer::canonical(dir.path()).unwrap();
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
